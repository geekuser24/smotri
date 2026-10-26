//! Stock insert browser. Credentials and HTTP live in the desktop host, never project/UI state.
use crate::{FilmcraftApp, i18n, panels::settings};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct Gif {
    pub id: String,
    pub title: String,
    pub page: String,
    pub thumbnail: String,
    pub download: String,
}
pub enum Event {
    Results(Result<Vec<Gif>, String>),
    SearchFinished,
    Thumbnail(String, Vec<u8>),
    Downloaded(Result<PathBuf, String>),
}
pub trait Host {
    fn has_key(&self) -> bool;
    fn save_key(&mut self, key: &str) -> Result<(), String>;
    fn search(&mut self, query: &str, offset: u32) -> Result<(), String>;
    fn download(&mut self, gif: &Gif, folder: PathBuf) -> Result<(), String>;
    fn poll(&mut self) -> Option<Event>;
}
pub struct Inserts {
    pub host: Option<Box<dyn Host>>,
    pub query: String,
    pub giphy: bool,
    pub busy: bool,
    pub downloading: bool,
    pub status: String,
    key: String,
    offset: u32,
    searched_query: String,
    results: Vec<Gif>,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    context: Option<Arc<filmcraft_project::Project>>,
    placement: Option<(filmcraft_project::ItemId, filmcraft_time::Tick)>,
}
impl Default for Inserts {
    fn default() -> Self {
        Self {
            host: None,
            query: String::new(),
            giphy: true,
            busy: false,
            downloading: false,
            status: String::new(),
            key: String::new(),
            offset: 0,
            searched_query: String::new(),
            results: Vec::new(),
            textures: Default::default(),
            context: None,
            placement: None,
        }
    }
}
impl Inserts {
    pub fn clear_key_draft(&mut self) {
        self.key.clear();
    }
    pub fn apply_key_draft(&mut self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Ok(());
        }
        self.host.as_mut().ok_or_else(|| i18n::t("Inserts require the desktop app").to_string())?.save_key(self.key.trim())?;
        self.key.clear();
        Ok(())
    }
    pub fn inspect(&self) -> Value {
        json!({"query":self.query,"giphy":self.giphy,"busy":self.busy,"downloading":self.downloading,"status":self.status,"offset":self.offset,"results":self.results.iter().map(|g|json!({"id":g.id,"title":g.title})).collect::<Vec<_>>()})
    }
}
pub fn command(app: &mut FilmcraftApp, id: &str, p: Value) -> Result<Value, String> {
    match id {
        "inserts.search" => {
            let query = p.get("query").and_then(Value::as_str).ok_or("query required")?;
            if query.trim().is_empty() || query.chars().count() > 50 {
                return Err(i18n::t("Enter a search phrase of 1–50 characters").into());
            }
            if !app.inserts.giphy {
                return Err(i18n::t("Enable GIPHY in Sources").into());
            }
            let offset =
                p.get("offset").map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= 4992)).unwrap_or(Some(0)).ok_or("invalid offset")?;
            app.inserts.host.as_mut().ok_or(i18n::t("Inserts require the desktop app"))?.search(query, offset)?;
            app.inserts.query = query.into();
            app.inserts.searched_query = query.into();
            app.inserts.offset = offset;
            app.inserts.busy = true;
            app.inserts.results.clear();
            app.inserts.textures.clear();
            app.inserts.status = i18n::t("Searching GIPHY…").into();
            Ok(Value::Null)
        }
        "inserts.import" => {
            if app.inserts.downloading {
                return Err(i18n::t("An insert is already downloading").into());
            }
            let id = p.get("id").and_then(Value::as_str).ok_or("GIF id required")?;
            let gif = app.inserts.results.iter().find(|g| g.id == id).ok_or("Choose a result from the current search")?.clone();
            let folder = app
                .session
                .path
                .as_ref()
                .and_then(|p| std::path::Path::new(p).parent())
                .map(|p| p.join("Inserts"))
                .or_else(|| filmcraft_engine::autosave::default_data_dir().map(|p| p.join("Inserts")))
                .ok_or("No download folder available")?;
            app.inserts.host.as_mut().ok_or(i18n::t("Inserts require the desktop app"))?.download(&gif, folder)?;
            app.inserts.context = Some(app.session.project.clone());
            app.inserts.placement =
                if app.session.prefs.inserts.add_to_timeline { app.session.state.active_sequence.map(|id| (id, app.session.playhead())) } else { None };
            app.inserts.downloading = true;
            app.inserts.status = i18n::t("Downloading GIF…").into();
            Ok(Value::Null)
        }
        "inserts.inspect" => Ok(app.inserts.inspect()),
        _ => Err("unknown inserts command".into()),
    }
}
pub fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    for _ in 0..32 {
        let Some(event) = app.inserts.host.as_mut().and_then(|h| h.poll()) else { break };
        match event {
            Event::Results(result) => match result {
                Ok(results) => {
                    app.inserts.status = i18n::t(if results.is_empty() { "No GIFs found" } else { "Choose a GIF to import into the project" }).into();
                    app.inserts.results = results;
                }
                Err(e) => {
                    app.inserts.busy = false;
                    app.inserts.status = e;
                }
            },
            Event::SearchFinished => app.inserts.busy = false,
            Event::Thumbnail(id, bytes) => {
                if app.inserts.results.iter().any(|g| g.id == id)
                    && let Ok(image) = thumbnail(&bytes)
                {
                    let tex = ctx.load_texture(format!("giphy-{id}"), image, egui::TextureOptions::LINEAR);
                    app.inserts.textures.insert(id, tex);
                }
            }
            Event::Downloaded(result) => {
                app.inserts.downloading = false;
                let context = app.inserts.context.take();
                let placement = app.inserts.placement.take();
                let result = result.and_then(|path| {
                    if !context.as_ref().is_some_and(|p| Arc::ptr_eq(p, &app.session.project)) {
                        return Err(i18n::t("Project changed during download. Search and import again.").into());
                    }
                    let imported = app.session.execute("file.import", json!({"paths":[path]})).map_err(|e| e.to_string())?;
                    if let Some(item) = imported.get("items").and_then(|v| v.as_array()).and_then(|v| v.first()) {
                        app.session.execute("source.open", json!({"item":item})).map_err(|e| e.to_string())?;
                        app.show_panel(crate::dock::PanelKind::Source);
                        if let Some((sequence, at)) = placement {
                            if app.session.state.active_sequence != Some(sequence) {
                                return Err(i18n::t("GIF imported into Project only: the active timeline changed.").into());
                            }
                            let id = item.as_u64().map(filmcraft_project::ItemId).ok_or("Invalid imported item")?;
                            place_insert(&mut app.session, id, at)
                                .map_err(|e| format!("{}: {e}", i18n::t("GIF imported into Project; timeline placement failed")))?;
                            return Ok(true);
                        }
                    }
                    Ok(false)
                });
                app.inserts.status = match result {
                    Ok(true) => i18n::t("GIF imported and added to the timeline.").into(),
                    Ok(false) => i18n::t("GIF imported. Drag it from Project to the timeline.").into(),
                    Err(e) => e,
                };
                app.status(app.inserts.status.clone());
            }
        }
    }
    if app.inserts.busy || app.inserts.downloading {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    if app.ui.settings.is_none() {
        app.inserts.clear_key_draft();
    }
}
/// Use ordinary placement so adding a track and clip is one undoable operation.
fn place_insert(session: &mut filmcraft_engine::Session, item: filmcraft_project::ItemId, at: filmcraft_time::Tick) -> Result<(), String> {
    use filmcraft_time::{Tick, TimeRange};
    let source = session.project.item(item).ok_or("Imported item not found")?;
    let duration = match &source.kind {
        filmcraft_project::ItemKind::Media(m) if matches!(m.info.kind, filmcraft_media::MediaKind::Still) => {
            session.prefs.timeline.still_duration(session.sequence_rate())
        }
        _ => source.duration(),
    };
    let rate = session.sequence_rate();
    let start = rate.snap_nearest(at);
    let end = rate.snap_nearest(Tick(at.0.checked_add(duration.0).ok_or("Insert time exceeds timeline limits")?));
    if start < Tick::ZERO || end <= start {
        return Err("Invalid insert duration or position".into());
    }
    let range = TimeRange::new(start, end - start);
    let seq = session.active_sequence().ok_or("No active timeline")?;
    // Stay above overlapping footage so an insert is visible, and never overwrite any clip.
    let bottom = seq.video_tracks.iter().rposition(|tr| tr.items.iter().any(|c| c.range().overlaps(&range))).map_or(0, |i| i + 1);
    let track = seq
        .video_tracks
        .iter()
        .skip(bottom)
        .find(|tr| !tr.locked && tr.enabled && !tr.items.iter().any(|c| c.range().overlaps(&range)))
        .map(|tr| json!(tr.id.0))
        .unwrap_or(json!("new"));
    session
        .execute("timeline.place", json!({"item":item.0,"track":track,"time":start.0,"sourceIn":0,"duration":range.duration.0,"video":true,"audio":false}))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn thumbnail(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() > 1024 * 1024 {
        return Err("thumbnail too large".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|_| "invalid thumbnail")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "invalid thumbnail")?.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}
pub fn settings_page(app: &mut FilmcraftApp, ui: &mut egui::Ui, values: &mut Value) {
    ui.heading(i18n::t("Inserts"));
    let mut enabled = values.pointer("/inserts/addToTimeline").and_then(Value::as_bool).unwrap_or(true);
    let r = ui.checkbox(&mut enabled, i18n::t("Add to timeline after import"));
    app.auto.add("settings.inserts.addToTimeline", r.rect, "Add to timeline after import");
    if r.changed()
        && let Some(slot) = values.pointer_mut("/inserts/addToTimeline")
    {
        *slot = serde_json::json!(enabled);
    }
    ui.label(i18n::t("Places the GIF at the playhead on a free video track, creating one if needed."));
    ui.add_space(12.0);
    ui.heading(i18n::t("Clipboard"));
    let mut clipboard = values.pointer("/clipboard/addToTimeline").and_then(Value::as_bool).unwrap_or(true);
    let r = ui.checkbox(&mut clipboard, i18n::t("Add clipboard media to timeline"));
    app.auto.add("settings.clipboard.addToTimeline", r.rect, "Add clipboard media to timeline");
    if r.changed()
        && let Some(slot) = values.pointer_mut("/clipboard/addToTimeline")
    {
        *slot = json!(clipboard);
    }
    ui.label(i18n::t("Paste images or GIF files with Cmd+V on Mac or Ctrl+V on Windows/Linux. Turn off to import into the project only."));
    ui.label(i18n::t("Uses a free track at the playhead. Without an active sequence, imports into the project."));
    ui.add_space(12.0);
    ui.label("GIPHY · GIF");
    ui.add_space(12.);
    let configured = app.inserts.host.as_ref().is_some_and(|h| h.has_key());
    ui.label(i18n::t(if configured { "API key configured" } else { "API key not configured" }));
    ui.label(tl!("GIPHY API key"));
    let r = ui.add(
        egui::TextEdit::singleline(&mut app.inserts.key).password(true).hint_text(i18n::t("Enter a new key to replace the saved key")).desired_width(420.),
    );
    app.auto.add("inserts.apiKey", r.rect, "GIPHY API key");
    ui.label(i18n::t("Click OK to save the key. Cancel discards changes."));
    ui.label(i18n::t("The GIPHY key is stored in macOS Keychain. Search terms are sent to GIPHY."));
    ui.add_space(12.);
    ui.label(i18n::t("Open Window → Inserts to search. More sources can be added later."));
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.)), |ui| {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.inserts.query)
                    .hint_text(i18n::t("Search GIFs…"))
                    .desired_width((ui.available_width() - 130.).max(60.))
                    .char_limit(50),
            );
            app.auto.add("inserts.query", r.rect, "Search GIFs");
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let r = ui.add_enabled(!app.inserts.busy, egui::Button::new(i18n::t("Search")));
            app.auto.add("inserts.search", r.rect, "Search");
            if r.clicked() || enter {
                let q = app.inserts.query.clone();
                if let Err(e) = command(app, "inserts.search", json!({"query":q})) {
                    app.inserts.status = e;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(i18n::t("Sources"));
            let r = ui.checkbox(&mut app.inserts.giphy, "GIPHY");
            app.auto.add("inserts.source.giphy", r.rect, "GIPHY");
            let r = ui.button(i18n::t("Settings"));
            app.auto.add("inserts.settings", r.rect, "Settings");
            if r.clicked() {
                let _ = settings::open(app, "inserts");
            }
            let r = ui.hyperlink_to("Powered By GIPHY", "https://giphy.com/");
            app.auto.add("inserts.attribution", r.rect, "Powered By GIPHY");
        });
        ui.label(&app.inserts.status);
        if app.inserts.busy || app.inserts.downloading {
            ui.spinner();
        }
        if !app.inserts.giphy {
            ui.label(i18n::t("Enable GIPHY in Sources"));
            return;
        }
        let mut import = None;
        egui::ScrollArea::vertical().id_salt("inserts.results").show(ui, |ui| {
            let cols = ((ui.available_width() / 156.).floor() as usize).clamp(1, 6);
            for row in app.inserts.results.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for gif in row {
                        ui.vertical(|ui| {
                            ui.set_width(148.);
                            if let Some(tex) = app.inserts.textures.get(&gif.id) {
                                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(144., 94.)));
                            } else {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(144., 94.), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 4., ui.visuals().extreme_bg_color);
                            }
                            ui.add(egui::Label::new(&gif.title).truncate()).on_hover_text(&gif.title);
                            ui.horizontal(|ui| {
                                let r = ui.add_enabled(!app.inserts.downloading, egui::Button::new(i18n::t("Import")));
                                app.auto.add(&format!("inserts.import.{}", gif.id), r.rect, "Import GIF");
                                if r.clicked() {
                                    import = Some(gif.id.clone());
                                }
                                let r = ui.hyperlink_to("GIPHY", &gif.page);
                                app.auto.add(&format!("inserts.source.{}", gif.id), r.rect, "View on GIPHY");
                            });
                        });
                    }
                });
                ui.add_space(10.);
            }
            ui.horizontal(|ui| {
                for (key, label, offset, enabled) in [
                    ("previous", "Previous", app.inserts.offset.saturating_sub(24), app.inserts.offset > 0),
                    ("next", "Next", app.inserts.offset.saturating_add(24), app.inserts.results.len() == 24 && app.inserts.offset < 4992),
                ] {
                    let r = ui.add_enabled(enabled && !app.inserts.busy, egui::Button::new(i18n::t(label)));
                    app.auto.add(&format!("inserts.{key}"), r.rect, label);
                    if r.clicked() {
                        let q = app.inserts.searched_query.clone();
                        if let Err(e) = command(app, "inserts.search", json!({"query":q,"offset":offset})) {
                            app.inserts.status = e;
                        }
                    }
                }
            });
        });
        if let Some(id) = import
            && let Err(e) = command(app, "inserts.import", json!({"id":id}))
        {
            app.inserts.status = e;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeHost;
    impl Host for FakeHost {
        fn has_key(&self) -> bool {
            true
        }
        fn save_key(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn search(&mut self, _: &str, _: u32) -> Result<(), String> {
            Ok(())
        }
        fn download(&mut self, _: &Gif, _: PathBuf) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<Event> {
            None
        }
    }
    #[test]
    fn auto_insert_uses_free_tracks_creates_when_full_and_undoes() {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let item = s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| &t.items).next().unwrap().item;
        let at = filmcraft_time::Tick::ZERO;
        let before = s.project.to_json();
        let tracks = s.active_sequence().unwrap().video_tracks.len();
        place_insert(&mut s, item, at).unwrap();
        let placed = s.active_sequence().unwrap().find_item(s.state.selection[0]).unwrap().1;
        assert_eq!(placed.start, at);
        assert!(s.active_sequence().unwrap().video_tracks.len() >= tracks);
        let count = s.active_sequence().unwrap().video_tracks.len();
        place_insert(&mut s, item, at).unwrap();
        assert_eq!(s.active_sequence().unwrap().video_tracks.len(), count + 1);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.to_json(), before);
        assert!(place_insert(&mut s, filmcraft_project::ItemId(u64::MAX), at).is_err());
        assert_eq!(s.project.to_json(), before);
    }

    #[test]
    fn inserts_commands_validate_inputs_and_do_not_expose_keys() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.inserts.host = Some(Box::new(FakeHost));
        app.inserts.key = "private-draft".into();
        assert!(!app.inserts.inspect().to_string().contains("private-draft"));
        for p in [json!({}), json!({"query":""}), json!({"query":"x".repeat(51)}), json!({"query":"cat","offset":-1}), json!({"query":"cat","offset":u64::MAX})]
        {
            assert!(command(&mut app, "inserts.search", p).is_err());
        }
        app.inserts.giphy = false;
        assert!(command(&mut app, "inserts.search", json!({"query":"cat"})).is_err());
        app.inserts.giphy = true;
        command(&mut app, "inserts.search", json!({"query":"кот"})).unwrap();
        assert_eq!(app.inserts.query, "кот");
        assert!(app.inserts.busy);
        assert!(command(&mut app, "inserts.import", json!({"id":"unknown"})).is_err());
    }
    #[test]
    fn inserts_invalid_thumbnails_do_not_panic() {
        for bytes in [vec![], vec![0; 50], b"GIF89a".to_vec(), vec![255; 1000]] {
            assert!(std::panic::catch_unwind(|| thumbnail(&bytes)).unwrap().is_err());
        }
    }
}
