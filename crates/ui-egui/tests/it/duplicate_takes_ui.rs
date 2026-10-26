//! Duplicate-take threshold, cached regrouping and safe manual choice through the UI.
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::{Transcript, Word};
use filmcraft_time::Tick;
use filmcraft_ui_egui::{FilmcraftApp, control::ControlRequest, dock::PanelKind};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new(session: Session) -> Self {
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        app.show_panel(PanelKind::DuplicateTakes);
        app.ui.keys.maximized = Some(PanelKind::DuplicateTakes);
        let mut builder = Harness::builder().with_size(egui::vec2(1000.0, 850.0)).with_max_steps(10_000);
        if std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").is_some() {
            builder = builder.wgpu();
        }
        let harness = builder.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        let v = self.ok("ui.elements", json!({"prefix": prefix}));
        v.as_array().unwrap().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }
}
fn ms(n: i64) -> Tick {
    Tick::from_units(n, 1000)
}
fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name":"Duplicate takes", "audio":1, "video":1})).unwrap();
    let mut tr = Transcript::default();
    let mut audio = vec![0f32; 16000 * 6];
    for (offset, phrase) in [(500, "Сегодня. мы покажем новый редактор!"), (3000, "сегодня мы покажем крутой редактор")]
    {
        for (i, text) in phrase.split_whitespace().enumerate() {
            let a = offset + i as i64 * 300;
            tr.words.push(Word::new(text, ms(a), ms(a + 250)));
            audio[(a * 16) as usize..((a + 250) * 16) as usize].fill(0.2);
        }
    }
    let bytes: Arc<[u8]> = filmcraft_media::wav::write_wav16(&audio, 1, 16000).into();
    let item = filmcraft_engine::commands::import_bytes(&mut s, "/duplicate-takes.wav", bytes, None).unwrap();
    let placed = s.execute("timeline.place", json!({"item":item.0, "audioTrack":"A1", "seconds":0})).unwrap();
    s.execute("timeline.select", json!({"clips":placed["clips"]})).unwrap();
    s.transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber { transcript: tr, id: "fixture".into() }));
    s
}
#[test]
fn duplicate_threshold_and_cached_regroup_are_reviewable() {
    let mut d = Driver::new(session());
    assert!(d.ids("cleanup.duplicates").contains(&"cleanup.duplicates.similarity".into()));
    d.ok("ui.click", json!({"id":"cleanup.duplicates.analyze"}));
    let deadline = web_time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        d.frames(2);
        let v = d.exec("audioCleanup.inspect", json!({}));
        if v["finished"] == true {
            assert!(v["error"].is_null(), "{v}");
            assert_eq!(v["analysis"]["similarity_percent"], 80.);
            assert_eq!(v["analysis"]["groups"].as_array().unwrap().len(), 1);
            break;
        }
        assert!(web_time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let before = d.harness.state().session.project.clone();
    // Slider track starts at the left of the registered response; click near its 100% end.
    let elems = d.ok("ui.elements", json!({"prefix":"cleanup.duplicates.similarity"}));
    let r = &elems[0]["rect"];
    let x = r[0].as_f64().unwrap();
    let y = r[1].as_f64().unwrap() + r[3].as_f64().unwrap() * 0.5;
    d.ok("ui.click", json!({"x":x+99.,"y":y}));
    d.frames(3);
    let controls = d.ok("ui.elements", json!({"prefix":"cleanup.duplicates"}));
    assert_eq!(controls.as_array().unwrap().iter().find(|e| e["id"] == "cleanup.duplicates.keep.0").unwrap()["enabled"], false);
    d.ok("ui.click", json!({"id":"cleanup.duplicates.regroup"}));
    d.frames(3);
    let v = d.exec("audioCleanup.inspect", json!({}));
    assert!(v["analysis"]["similarity_percent"].as_f64().unwrap() > 80.);
    assert_eq!(v["analysis"]["groups"].as_array().unwrap().len(), 0);
    assert_eq!(d.harness.state().session.project, before);
    assert_eq!(d.harness.state().session.jobs.len(), 1);
    d.exec("audioCleanup.regroup", json!({"similarityPercent":80}));
    d.frames(3);
    let controls = d.ok("ui.elements", json!({"prefix":"cleanup.duplicates"}));
    assert_eq!(controls.as_array().unwrap().iter().find(|e| e["id"] == "cleanup.duplicates.keep.0").unwrap()["enabled"], true);
    if let Some(dir) = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        d.harness.render().unwrap().save(dir.join("duplicate-takes-threshold.png")).unwrap();
    }
    d.ok("ui.click", json!({"id":"cleanup.duplicates.keep.0"}));
    d.frames(3);
    assert_ne!(d.harness.state().session.project, before);
    let controls = d.ok("ui.elements", json!({"prefix":"cleanup.duplicates.similarity"}));
    assert_eq!(controls[0]["enabled"], false);
    d.exec("edit.undo", json!({}));
    assert_eq!(d.harness.state().session.project, before);
}
