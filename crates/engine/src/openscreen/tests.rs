use super::*;
use crate::media_test_util::{make_movie, tmp_dir};
use filmcraft_media::DemoScene;
use std::io::Cursor as IoCursor;

fn document() -> Value {
    json!({"version":2,"media":{"screenVideoPath":"screen.mov","webcamVideoPath":"camera.mov","cursorCaptureMode":"editable-overlay"},"editor":{"aspectRatio":"16:9","webcamOffsetMs":125,"compositionKeyframes":[
        {"timeMs":0,"layoutPreset":"picture-in-picture","screen":{"x":0.1,"y":0.1,"width":0.8,"height":0.8},"webcam":{"x":0.8,"y":0.8,"width":0.2,"height":0.2}},
        {"timeMs":1000,"layoutPreset":"no-webcam"},
        {"timeMs":2000,"layoutPreset":"dual-frame"}
    ],"zoomRegions":[{"startMs":2000,"endMs":2600,"depth":3,"focus":{"cx":0.5,"cy":0.5},"focusMode":"auto"}]}})
}
fn cursor_asset(id: &str, color: [u8; 4]) -> Value {
    let mut png = IoCursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 8, image::Rgba(color)).write_to(&mut png, image::ImageFormat::Png).unwrap();
    json!({"id":id,"imageDataUrl":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(png.into_inner())),"width":8,"height":8,"hotspotX":4,"hotspotY":4})
}
fn fixture() -> (std::path::PathBuf, String) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = tmp_dir(&format!("openscreen-{id}"));
    make_movie(&dir.join("screen.mov"), DemoScene::Aurora, 64, 36, 72);
    make_movie(&dir.join("camera.mov"), DemoScene::Plasma, 32, 24, 72);
    let path = dir.join("take.openscreen");
    std::fs::write(&path, serde_json::to_vec(&document()).unwrap()).unwrap();
    let c = json!({"samples":[{"timeMs":0,"cx":0.5,"cy":0.5,"assetId":"red"},{"timeMs":1500,"cx":0.5,"cy":0.5,"assetId":"green"}],"assets":[cursor_asset("red",[255,0,0,255]),cursor_asset("green",[0,255,0,255])]});
    std::fs::write(dir.join("screen.mov.cursor.json"), serde_json::to_vec(&c).unwrap()).unwrap();
    (dir, path.to_string_lossy().into_owned())
}
#[test]
fn malformed_documents_and_cursor_mutations_never_panic() {
    let bytes = serde_json::to_vec(&document()).unwrap();
    for i in 0..bytes.len() {
        let short = &bytes[..i];
        assert!(std::panic::catch_unwind(|| model::parse(short)).unwrap().is_err());
        let mut corrupt = bytes.clone();
        corrupt[i] ^= 0xff;
        assert!(std::panic::catch_unwind(|| model::parse(&corrupt)).is_ok());
    }
    for editor in [
        json!({"padding":-1}),
        json!({"cropRegion":{"x":0,"y":0,"width":0,"height":1}}),
        json!({"webcamOffsetMs":1e99}),
        json!({"compositionKeyframes":[{"timeMs":-1,"layoutPreset":"no-webcam"}]}),
        json!({"zoomRegions":[{"startMs":5,"endMs":1,"depth":0,"focus":{"cx":0.5,"cy":0.5}}]}),
    ] {
        let mut v = document();
        v["editor"] = editor;
        assert!(model::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    assert!(model::cursor(br#"{"samples":[{"timeMs":1e99,"cx":0,"cy":0,"assetId":"x"}]}"#).is_err());
    assert!(model::cursor(br#"{"assets":[{"id":"x","imageDataUrl":"","width":0,"height":1,"hotspotX":0,"hotspotY":0}]}"#).is_err());
}
#[test]
fn layout_interpolates_only_within_the_same_preset() {
    let mut v = document();
    v["editor"]["compositionKeyframes"][1] = json!({"timeMs":1000,"layoutPreset":"picture-in-picture","screen":{"x":0.3,"y":0.1,"width":0.4,"height":0.8}});
    let d = model::parse(&serde_json::to_vec(&v).unwrap()).unwrap();
    let f = |t| motion::layout(&d.editor, t, 1920., 1080., 64., 36., Some((32., 24.)));
    assert!((f(500.).screen.x - 384.).abs() < 1e-8);
    assert!((f(1500.).screen.x - 576.).abs() < 1e-8, "hold previous layout until the preset cut");
    assert!(f(2000.).cover);
}
#[test]
fn nested_import_keeps_source_time_keys_camera_sync_and_cursor_layers_and_undo() {
    let (dir, path) = fixture();
    let mut s = Session::default();
    let before = serde_json::to_value(&*s.project).unwrap();
    let r = s.execute("file.import", json!({"paths":[path]})).unwrap();
    let main = ItemId(r["documents"][0]["originalSequence"].as_u64().unwrap());
    s.execute("sequence.open", json!({"item": main.0})).unwrap();
    let seq = s.project.sequence(main).unwrap();
    assert_eq!(seq.video_tracks[0].items.len(), 4, "three layouts plus camera end hold");
    assert_eq!(seq.audio_tracks[0].items.len(), 4);
    let nests: Vec<_> = seq.video_tracks[0].items.iter().map(|v| v.item).collect();
    let first = s.project.sequence(nests[0]).unwrap();
    assert_eq!(first.video_tracks.len(), 5);
    assert_eq!(first.video_tracks[2].items.len(), 1);
    assert_eq!(first.video_tracks[3].items.len(), 1, "cursor assets must not overlap on one track");
    assert_eq!(first.video_tracks[4].items[0].source_in, ms(125.));
    let second = s.project.sequence(nests[1]).unwrap();
    let desktop = &second.video_tracks[1].items[0];
    assert_eq!(desktop.source_in, ms(1000.));
    assert_eq!(desktop.effect("motion").unwrap().params["position"].keyframes[0].time, ms(1000.));
    let cam = &second.video_tracks[4].items[0];
    assert_eq!(cam.effect("opacity").unwrap().f64_at("opacity", ms(1500.)), 0.);
    // Both cursor assets must render in their own spans, even after nesting and saving.
    let pixel = |s: &mut Session, t: f64| {
        s.set_playhead(ms(t));
        let img = s.render_program(0.1).unwrap();
        let p = img.over_black_rgba8();
        let i = (img.h / 2 * img.w + img.w / 2) * 4;
        [p[i], p[i + 1], p[i + 2]]
    };
    assert_eq!(pixel(&mut s, 500.), [255, 0, 0]);
    assert_eq!(pixel(&mut s, 1750.), [0, 255, 0]);
    let save = dir.join("converted.fcproj");
    s.execute("file.saveAs", json!({"path":save})).unwrap();
    let mut reopened = Session::default();
    reopened.execute("file.open", json!({"path":save})).unwrap();
    assert_eq!(pixel(&mut reopened, 1750.), [0, 255, 0]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*s.project).unwrap(), before);
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn failure_does_not_mutate_open_project_or_overwrite_assets() {
    let (dir, path) = fixture();
    let mut s = Session::default();
    let before = serde_json::to_value(&*s.project).unwrap();
    std::fs::remove_file(dir.join("camera.mov")).unwrap();
    assert!(s.execute("file.open", json!({"path":path})).is_err());
    assert_eq!(serde_json::to_value(&*s.project).unwrap(), before);
    let keep = dir.join("take.openscreen-media");
    std::fs::create_dir_all(&keep).unwrap();
    std::fs::write(keep.join("cursor-0-0.png"), b"keep").unwrap();
    let a = cursor_asset("red", [255, 0, 0, 255]);
    let extracted = embedded(&s, &path, "cursor-0", a["imageDataUrl"].as_str().unwrap()).unwrap();
    assert!(extracted.ends_with("cursor-0-1.png"));
    assert_eq!(std::fs::read(keep.join("cursor-0-0.png")).unwrap(), b"keep");
    assert_eq!(embedded(&s, &path, "cursor-0", a["imageDataUrl"].as_str().unwrap()).unwrap(), extracted);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn trims_and_speed_apply_to_linked_nests_without_touching_source_keys() {
    let (dir, path) = fixture();
    let mut v = document();
    v["editor"]["trimRegions"] = json!([{"startMs":500,"endMs":1000}]);
    v["editor"]["speedRegions"] = json!([{"startMs":1000,"endMs":2000,"speed":2}]);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let mut s = Session::default();
    let r = s.execute("file.open", json!({"path":path})).unwrap();
    let q = s.project.sequence(ItemId(r["originalSequence"].as_u64().unwrap())).unwrap();
    let v = &q.video_tracks[0].items;
    let a = &q.audio_tracks[0].items;
    assert_eq!(v.iter().map(|i| i.duration.0).sum::<i64>(), ms(2000.).0);
    assert_eq!(v[1].speed, 2.);
    assert_eq!(v[1].duration, ms(500.));
    for (v, a) in v.iter().zip(a) {
        assert_eq!((v.start, v.duration, v.speed, v.link), (a.start, a.duration, a.speed, a.link));
    }
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn zoom_tracks_cursor_and_returns_to_neutral_without_nonfinite_keys() {
    let d = model::parse(&serde_json::to_vec(&document()).unwrap()).unwrap();
    let c = model::cursor(br#"{"samples":[{"timeMs":0,"cx":0.3,"cy":0.5,"assetId":"a"},{"timeMs":4000,"cx":0.7,"cy":0.5,"assetId":"a"}]}"#).unwrap();
    let mut cam = motion::Camera::default();
    let mut peak = 1.;
    for i in 0..360 {
        let v = cam.step(&d.editor, &c, f64::from(i) * 1000. / 60., 1920., 1080.);
        assert!(v.x.is_finite() && v.y.is_finite() && v.k.is_finite());
        if v.k > peak {
            peak = v.k;
        }
    }
    assert!(peak > 1.7);
    assert!((cam.value.k - 1.).abs() < 1e-3);
    assert!(cam.value.x.abs() < 1e-2);
    let path = motion::CursorPath::new(&c, 0.67);
    let p = path.at(&c, 1500.).unwrap();
    assert!(p.cx > 0.3 && p.cx < 0.45, "spring trails movement");
    assert_eq!(path.at(&c, 1500.).unwrap().cx, p.cx, "random seek is deterministic");
}

#[test]
fn full_length_layout_tracks_and_cursor_zoom_are_editable_and_undoable() {
    let (dir, path) = fixture();
    let mut s = Session::default();
    let r = s.execute("file.open", json!({"path":path})).unwrap();
    let main = ItemId(r["sequences"][0].as_u64().unwrap());
    let q = s.project.sequence(main).unwrap();
    assert_eq!(q.video_tracks.len(), 4);
    assert_eq!(q.video_tracks.iter().filter(|t| t.enabled).count(), 1);
    assert_eq!(q.audio_tracks.len(), 1);
    assert_eq!(q.audio_tracks[0].items.len(), 1);
    let mut ids = Vec::new();
    for track in &q.video_tracks {
        assert_eq!(track.items.len(), 1);
        assert_eq!(track.items[0].duration, ms(3000.));
        let id = track.items[0].item;
        ids.push(id);
        let child = s.project.sequence(id).unwrap();
        assert_eq!(child.duration(), ms(3000.));
        assert_eq!(child.video_tracks[1].name, "Desktop");
        assert!(child.video_tracks[1].items.iter().all(|c| s.project.sequence(c.item).is_none()), "direct media inside angle");
    }
    let camera = s.project.sequence(ids[3]).unwrap();
    assert_eq!(camera.video_tracks[1].items[0].effect("opacity").unwrap().f64_at("opacity", ms(1000.)), 0.);
    let clip_id = q.video_tracks[0].items[0].id;
    let fx = q.video_tracks[0].items[0].effect("openscreen_zoom").unwrap();
    assert_eq!(fx.params["active"].keyframes.len(), 2);
    assert_eq!(fx.params["active"].keyframes[0].time, ms(2000.));
    assert_eq!(fx.params["active"].keyframes[1].value, ParamValue::Bool(false));
    let before = s.project.sequence(main).unwrap().clone();
    let children: Vec<_> = ids.iter().map(|id| s.project.sequence(*id).unwrap().clone()).collect();
    s.execute("timeline.select", json!({"clips":[clip_id.0],"linked":false})).unwrap();
    for bad in [json!({"scale":0}), json!({"duration":1e99}), json!({"start":-1}), json!({"follow":"yes"})] {
        assert!(s.execute("openscreen.addZoom", bad).is_err());
    }
    assert_eq!(*s.project.sequence(main).unwrap(), before);
    let render = |s: &Session, id: ItemId| {
        filmcraft_render::render_sequence(
            &s.project,
            id,
            ms(750.),
            filmcraft_render::RenderOptions { scale: 0.1, ..Default::default() },
            &s.media.provider(s.project.clone(), s.services.clone()),
        )
        .over_black_rgba8()
    };
    let neutral = render(&s, main);
    let others: Vec<_> = ids.iter().skip(1).map(|id| render(&s, *id)).collect();
    s.execute("openscreen.addZoom", json!({"start":0.25,"duration":1.,"scale":200.,"follow":true})).unwrap();
    let zoomed = render(&s, main);
    assert_ne!(zoomed, neutral, "effect changes the rendered desktop");
    // Camera in the lower-right corner is composited after the zoomed desktop.
    for y in 90..102 {
        for x in 170..185 {
            let pixel = (y * 192 + x) * 4;
            assert_eq!(&zoomed[pixel..pixel + 4], &neutral[pixel..pixel + 4], "camera unchanged");
        }
    }
    for (id, before) in ids.iter().zip(&children) {
        assert_eq!(s.project.sequence(*id).unwrap(), before, "no baked edits inside layouts");
    }
    for (id, pixels) in ids.iter().skip(1).zip(&others) {
        assert_eq!(&render(&s, *id), pixels, "other layouts unaffected");
    }
    let save = dir.join("angles.fcproj");
    s.execute("file.saveAs", json!({"path":save})).unwrap();
    let mut reopened = Session::default();
    reopened.execute("file.open", json!({"path":save})).unwrap();
    assert_eq!(render(&reopened, main), zoomed, "saved effect renders identically");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.project.sequence(main).unwrap(), before);
    assert_eq!(render(&s, main), neutral);
    // The effect can be removed and re-applied through the standard effects commands.
    s.execute("effects.remove", json!({"clip":clip_id.0,"index":0})).unwrap();
    assert!(s.project.sequence(main).unwrap().video_tracks[0].items[0].effect("openscreen_zoom").is_none());
    s.execute("effects.apply", json!({"effect":"openscreen_zoom","clips":[clip_id.0]})).unwrap();
    assert!(s.execute("effects.apply", json!({"effect":"openscreen_zoom","clips":[clip_id.0]})).is_err(), "no duplicate special effects");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.sequence(main).unwrap().video_tracks[0].items[0].effect("openscreen_zoom").is_none());
    s.execute("effects.apply", json!({"effect":"OpenScreen Zoom","clips":[clip_id.0]})).unwrap();
    assert!(s.project.sequence(main).unwrap().video_tracks[0].items[0].effect("openscreen_zoom").is_some());
    let other = s.project.sequence(main).unwrap().video_tracks[1].items[0].id;
    assert!(s.execute("effects.apply", json!({"effect":"openscreen_zoom","clips":[other.0]})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bundled_wallpaper_is_copied_locally_and_cursor_tracks_are_sparse() {
    let (dir, path) = fixture();
    let wallpapers = dir.join("wallpapers");
    std::fs::create_dir_all(&wallpapers).unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 40, 70, 255])).save(wallpapers.join("fixture.png")).unwrap();
    let mut d = document();
    d["editor"]["wallpaper"] = json!("/wallpapers/fixture.png");
    std::fs::write(&path, serde_json::to_vec(&d).unwrap()).unwrap();
    let mut s = Session::default();
    let result = s.execute("file.open", json!({"path":path})).unwrap();
    let background = s.project.items.values().find(|i| i.name == "Background").unwrap().as_media().unwrap();
    let MediaRef::File { path: extracted } = &background.media else { panic!("expected extracted wallpaper") };
    assert!(Path::new(extracted).starts_with(Path::new(&path).with_extension("openscreen-media")));
    assert_eq!(std::fs::read(extracted).unwrap(), std::fs::read(wallpapers.join("fixture.png")).unwrap());
    let pip = s.project.sequence(ItemId(result["compositions"][0].as_u64().unwrap())).unwrap();
    for (index, values) in [(2, [100., 0.]), (3, [0., 100.])] {
        let fx = pip.video_tracks[index].items[0].effect("opacity").unwrap();
        assert!(fx.params["opacity"].keyframes.len() <= 2, "inactive cursor assets need only switches");
        assert_eq!(fx.f64_at("opacity", ms(750.)), values[0]);
        assert_eq!(fx.f64_at("opacity", ms(2000.)), values[1]);
    }
    for bad in ["/wallpapers/../fixture.png", "/wallpapers/nested/file.png", "/wallpapers/evil.exe"] {
        assert!(wallpaper::resolve(&s, &path, bad).is_err());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn opening_creates_an_unsaved_native_document_and_never_overwrites_the_source() {
    let (dir, path) = fixture();
    let original = std::fs::read(&path).unwrap();
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let previous = dir.join("previous.fcproj");
    s.execute("file.saveAs", json!({"path":previous})).unwrap();
    let previous_bytes = std::fs::read(&previous).unwrap();
    let r = s.execute("file.open", json!({"path":path})).unwrap();
    assert!(s.path.is_none(), "Save must request a native project destination");
    assert!(s.is_dirty());
    assert_eq!(s.project.name, "take");
    assert_eq!(s.state.active_sequence.map(|id| id.0), r["sequences"][0].as_u64());
    assert!(!s.project.items.values().any(|i| i.name == "Main Edit"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(&previous).unwrap(), previous_bytes);
    std::fs::remove_dir_all(dir).unwrap();
}
