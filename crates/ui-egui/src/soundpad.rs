//! MyInstants browser. Network and preview playback are provided by the desktop host.
use crate::{FilmcraftApp, i18n};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, serde::Serialize)]
pub struct Sound {
    pub id: String,
    pub title: String,
    pub url: String,
}
pub enum Event {
    Results(Result<(Vec<Sound>, bool), String>),
    Downloaded(Result<PathBuf, String>),
    PreviewFinished(Result<(), String>),
}
pub trait Host {
    fn configured(&self) -> bool;
    fn save_key(&mut self, key: &str) -> Result<(), String>;
    fn search(&mut self, query: &str, page: u32) -> Result<(), String>;
    fn download(&mut self, sound: &Sound, folder: PathBuf) -> Result<(), String>;
    fn preview(&mut self, sound: &Sound) -> Result<(), String>;
    fn stop(&mut self);
    fn poll(&mut self) -> Option<Event>;
}
#[derive(Default)]
pub struct SoundPad {
    pub host: Option<Box<dyn Host>>,
    pub query: String,
    pub status: String,
    pub busy: bool,
    pub downloading: bool,
    pub previewing: bool,
    pub preview_tail: Option<web_time::Instant>,
    pub key: String,
    pub results: Vec<Sound>,
    page: u32,
    next: bool,
    searched: String,
    context: Option<Arc<filmcraft_project::Project>>,
    placement: Option<(filmcraft_project::ItemId, filmcraft_time::Tick)>,
}
impl SoundPad {
    pub fn inspect(&self) -> Value {
        json!({"query":self.query,"status":self.status,"busy":self.busy,"downloading":self.downloading,"previewing":self.previewing,"page":self.page,"next":self.next,"results":self.results})
    }
    pub fn apply_key(&mut self) -> Result<(), String> {
        if !self.key.trim().is_empty() {
            self.host.as_mut().ok_or("SoundPad requires desktop app")?.save_key(self.key.trim())?;
            self.key.clear();
        }
        Ok(())
    }
}
fn l(ru: &'static str, en: &'static str) -> &'static str {
    if i18n::current() == i18n::Language::Ru { ru } else { en }
}
pub fn command(app: &mut FilmcraftApp, id: &str, p: Value) -> Result<Value, String> {
    match id {
        "soundpad.inspect" => Ok(app.soundpad.inspect()),
        "soundpad.stop" => {
            if let Some(h) = app.soundpad.host.as_mut() {
                h.stop();
            }
            app.soundpad.previewing = false;
            app.soundpad.preview_tail = Some(web_time::Instant::now());
            Ok(Value::Null)
        }
        "soundpad.search" => {
            if app.soundpad.busy {
                return Err(l("Дождись завершения поиска", "Wait for the search to finish").into());
            }
            let q = p["query"].as_str().filter(|s| !s.trim().is_empty() && s.chars().count() <= 100).ok_or("query: 1–100 characters required")?;
            let page = p.get("page").map(|v| v.as_u64().filter(|n| (1..=1000).contains(n))).unwrap_or(Some(1)).ok_or("invalid page")? as u32;
            app.soundpad.host.as_mut().ok_or("SoundPad requires desktop app")?.search(q, page)?;
            app.soundpad.query = q.into();
            app.soundpad.searched = q.into();
            app.soundpad.page = page;
            app.soundpad.busy = true;
            app.soundpad.results.clear();
            app.soundpad.status = l("Поиск…", "Searching…").into();
            app.show_panel(crate::dock::PanelKind::SoundPad);
            Ok(Value::Null)
        }
        "soundpad.preview" | "soundpad.import" => {
            let sound = if let Some(n) = p["index"].as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)) {
                app.soundpad.results.get(n)
            } else {
                app.soundpad.results.iter().find(|s| Some(s.id.as_str()) == p["id"].as_str())
            }
            .cloned()
            .ok_or(l("Выбери звук из текущих результатов поиска", "Choose a sound from the current search"))?;
            if id == "soundpad.preview" {
                app.stop();
                app.stop_source();
                app.soundpad.host.as_mut().ok_or("SoundPad requires desktop app")?.preview(&sound)?;
                app.soundpad.previewing = true;
                app.soundpad.status = format!("{}: {}", l("Предпрослушивание", "Preview"), sound.title);
            } else {
                if app.soundpad.downloading {
                    return Err("Sound download already running".into());
                }
                let timeline =
                    p.get("timeline").map(|v| v.as_bool().ok_or("timeline must be boolean")).transpose()?.unwrap_or(app.session.prefs.soundpad.add_to_timeline);
                let folder = app
                    .session
                    .path
                    .as_ref()
                    .and_then(|p| std::path::Path::new(p).parent())
                    .map(|p| p.join("SoundPad"))
                    .or_else(|| filmcraft_engine::autosave::default_data_dir().map(|p| p.join("SoundPad")))
                    .ok_or("No download folder")?;
                app.soundpad.host.as_mut().ok_or("SoundPad requires desktop app")?.download(&sound, folder)?;
                app.soundpad.context = Some(app.session.project.clone());
                app.soundpad.placement = if timeline { app.session.state.active_sequence.map(|s| (s, app.session.playhead())) } else { None };
                app.soundpad.downloading = true;
                app.soundpad.status = l("Загрузка звука…", "Downloading sound…").into();
            }
            Ok(Value::Null)
        }
        _ => Err("Unknown SoundPad command".into()),
    }
}
pub fn place(session: &mut filmcraft_engine::Session, item: filmcraft_project::ItemId, at: filmcraft_time::Tick) -> Result<(), String> {
    use filmcraft_time::{Tick, TimeRange};
    let source = session.project.item(item).ok_or("Missing sound")?;
    let duration = source.duration();
    let start = session.sequence_rate().snap_nearest(at);
    let end = session.sequence_rate().snap_nearest(Tick(start.0.checked_add(duration.0).ok_or("Sound duration overflow")?));
    if start < Tick::ZERO || end <= start {
        return Err("Invalid sound duration".into());
    }
    let range = TimeRange::new(start, end - start);
    let seq = session.active_sequence().ok_or("No active sequence")?;
    let track = seq
        .audio_tracks
        .iter()
        .find(|t| !t.locked && t.enabled && !t.items.iter().any(|c| c.range().overlaps(&range)))
        .map(|t| json!(t.id.0))
        .unwrap_or(json!("new"));
    session
        .execute("timeline.place", json!({"item":item.0,"audioTrack":track,"time":start.0,"sourceIn":0,"duration":range.duration.0,"video":false,"audio":true}))
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    for _ in 0..16 {
        let Some(event) = app.soundpad.host.as_mut().and_then(|h| h.poll()) else { break };
        match event {
            Event::Results(r) => {
                app.soundpad.busy = false;
                match r {
                    Ok((rows, next)) => {
                        app.soundpad.status = if rows.is_empty() {
                            l("Ничего не найдено", "No sounds found").into()
                        } else {
                            format!("{}: {}", l("Найдено", "Results"), rows.len())
                        };
                        app.soundpad.results = rows;
                        app.soundpad.next = next;
                    }
                    Err(e) => app.soundpad.status = e,
                }
            }
            Event::PreviewFinished(r) => {
                app.soundpad.previewing = false;
                app.soundpad.preview_tail = Some(web_time::Instant::now());
                if let Err(e) = r {
                    app.soundpad.status = e;
                }
            }
            Event::Downloaded(r) => {
                app.soundpad.downloading = false;
                let context = app.soundpad.context.take();
                let placement = app.soundpad.placement.take();
                let result = r.and_then(|path| {
                    if !context.as_ref().is_some_and(|p| Arc::ptr_eq(p, &app.session.project)) {
                        return Err(format!("{}: {}", l("Проект изменился. Файл сохранён", "Project changed. File saved"), path.display()));
                    }
                    let imported = app.session.execute("file.import", json!({"paths":[path]})).map_err(|e| e.to_string())?;
                    let item = imported["items"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).ok_or("No imported audio item")?;
                    if let Some((seq, at)) = placement {
                        if app.session.state.active_sequence != Some(seq) {
                            return Err(l("Звук в проекте; активная секвенция изменилась", "Sound imported; active sequence changed").into());
                        }
                        place(&mut app.session, filmcraft_project::ItemId(item), at).map_err(|e| format!("Sound imported; placement failed: {e}"))?;
                        Ok(l("Звук добавлен на свободную аудиодорожку", "Sound added to a free audio track"))
                    } else {
                        Ok(l("Звук импортирован в проект", "Sound imported into Project"))
                    }
                });
                app.soundpad.status = result.map(str::to_string).unwrap_or_else(|e| e);
                app.status(app.soundpad.status.clone());
            }
        }
    }
    if app.soundpad.busy || app.soundpad.downloading || app.soundpad.previewing {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    if app.ui.settings.is_none() {
        app.soundpad.key.clear();
    }
}
pub fn settings(app: &mut FilmcraftApp, ui: &mut egui::Ui, values: &mut Value) {
    ui.heading("SoundPad · MyInstants");
    let mut on = values.pointer("/soundpad/addToTimeline").and_then(Value::as_bool).unwrap_or(true);
    let r = ui.checkbox(&mut on, l("Сразу добавлять на таймлайн", "Add to timeline after import"));
    app.auto.add_response("settings.soundpad.addToTimeline", &r, "Add to timeline");
    if r.changed()
        && let Some(v) = values.pointer_mut("/soundpad/addToTimeline")
    {
        *v = json!(on);
    }
    ui.label(l(
        "На позиции курсора, на свободную аудиодорожку. Если места нет — создаётся новая. При выключении — только в проект.",
        "At the playhead on a free audio track, creating one if needed. When disabled, import into Project only.",
    ));
    ui.separator();
    ui.label(l("API-токен MyInstants (необязательно)", "MyInstants API token (optional)"));
    if app.soundpad.host.as_ref().is_some_and(|h| h.configured()) {
        ui.label(l("Токен сохранён", "Token configured"));
    }
    let r = ui.add(egui::TextEdit::singleline(&mut app.soundpad.key).password(true));
    app.auto.add_response("soundpad.apiKey", &r, "MyInstants API token");
    ui.small(l(
        "Без токена: до 10 запросов в минуту. Поиск отправляется MyInstants. Токен сохраняется при нажатии OK.",
        "Without a token: up to 10 requests per minute. Searches are sent to MyInstants. Press OK to save a token.",
    ));
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.)), |ui| {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.soundpad.query)
                    .hint_text(l("Поиск звуков…", "Search sounds…"))
                    .char_limit(100)
                    .desired_width((ui.available_width() - 100.).max(70.)),
            );
            app.auto.add_response("soundpad.query", &r, "Search sounds");
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let r = ui.add_enabled(!app.soundpad.busy, egui::Button::new(i18n::t("Search")));
            app.auto.add_response("soundpad.search", &r, "Search");
            if r.clicked() || enter {
                action = Some(("soundpad.search", json!({"query":app.soundpad.query})));
            }
        });
        ui.horizontal(|ui| {
            let r = ui.button(i18n::t("Settings"));
            app.auto.add_response("soundpad.settings", &r, "Settings");
            if r.clicked() {
                let _ = crate::panels::settings::open(app, "soundpad");
            }
            let r = ui.add_enabled(app.soundpad.previewing, egui::Button::new(l("Стоп", "Stop")));
            app.auto.add_response("soundpad.stop", &r, "Stop preview");
            if r.clicked() {
                action = Some(("soundpad.stop", json!({})));
            }
            ui.hyperlink_to("MyInstants", "https://www.myinstants.com/");
        });
        ui.label(&app.soundpad.status);
        if app.soundpad.busy || app.soundpad.downloading {
            ui.spinner();
        }
        egui::ScrollArea::vertical().id_salt("soundpad.results").show(ui, |ui| {
            for (i, sound) in app.soundpad.results.iter().enumerate() {
                ui.group(|ui| {
                    ui.set_min_width((ui.available_width() - 12.).max(0.));
                    ui.label(format!("{}. {}", i + 1, sound.title));
                    ui.horizontal(|ui| {
                        for (verb, label, on) in [("preview", l("Слушать", "Listen"), true), ("import", i18n::t("Import"), !app.soundpad.downloading)] {
                            let r = ui.add_enabled(on, egui::Button::new(label));
                            app.auto.add_response(&format!("soundpad.{verb}.{}", i + 1), &r, label);
                            if r.clicked() {
                                action = Some((if verb == "preview" { "soundpad.preview" } else { "soundpad.import" }, json!({"index":i+1})));
                            }
                        }
                        ui.hyperlink_to(l("Источник", "Source"), format!("https://www.myinstants.com/en/instant/{}/", sound.id));
                    });
                });
            }
            ui.horizontal(|ui| {
                for (id, label, page, on) in [
                    ("previous", i18n::t("Previous"), app.soundpad.page.saturating_sub(1), app.soundpad.page > 1),
                    ("next", i18n::t("Next"), app.soundpad.page.saturating_add(1), app.soundpad.next && app.soundpad.page < 1000),
                ] {
                    let r = ui.add_enabled(on && !app.soundpad.busy, egui::Button::new(label));
                    app.auto.add_response(&format!("soundpad.{id}"), &r, label);
                    if r.clicked() {
                        action = Some(("soundpad.search", json!({"query":app.soundpad.searched,"page":page})));
                    }
                }
            });
        });
    });
    if let Some((id, p)) = action
        && let Err(e) = command(app, id, p)
    {
        app.soundpad.status = e;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_uses_free_audio_track_and_undo_restores_project() {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let item = session.active_sequence().unwrap().audio_tracks.iter().flat_map(|t| &t.items).next().unwrap().item;
        let before = session.project.to_json();
        let video = session.active_sequence().unwrap().video_tracks.clone();
        place(&mut session, item, filmcraft_time::Tick::ZERO).unwrap();
        let count = session.active_sequence().unwrap().audio_tracks.len();
        place(&mut session, item, filmcraft_time::Tick::ZERO).unwrap();
        assert_eq!(session.active_sequence().unwrap().audio_tracks.len(), count + 1);
        assert_eq!(session.active_sequence().unwrap().video_tracks, video);
        let placed = session.active_sequence().unwrap().find_item(session.state.selection[0]).unwrap().1;
        assert_eq!(placed.start, filmcraft_time::Tick::ZERO);
        session.execute("edit.undo", json!({})).unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.project.to_json(), before);
        assert!(place(&mut session, filmcraft_project::ItemId(u64::MAX), filmcraft_time::Tick::ZERO).is_err());
    }
    #[test]
    fn hostile_commands_leave_project_unchanged_and_do_not_expose_keys() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.soundpad.key = "private-token".into();
        assert!(!app.soundpad.inspect().to_string().contains("private-token"));
        let before = app.session.project.to_json();
        for (id, p) in [
            ("soundpad.search", json!({"query":""})),
            ("soundpad.search", json!({"query":"x","page":-1})),
            ("soundpad.import", json!({"index":0})),
            ("soundpad.preview", json!({"index":u64::MAX})),
        ] {
            assert!(command(&mut app, id, p).is_err());
        }
        assert_eq!(app.session.project.to_json(), before);
    }
}
