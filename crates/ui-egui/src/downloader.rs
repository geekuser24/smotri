//! Docked downloader; external processes and credentials stay in the native host.
use crate::{FilmcraftApp, dock::PanelKind, i18n};
use filmcraft_engine::settings::DownloaderPrefs;
use filmcraft_project::{ItemId, Project};
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaInfo {
    pub title: String,
    pub heights: Vec<u32>,
    pub audio: bool,
    pub video: bool,
    pub duration: Option<f64>,
}
pub enum Event {
    Info(Result<MediaInfo, String>),
    Progress(f32, String),
    Finished(Result<PathBuf, String>),
}
pub trait Host {
    fn probe(&mut self, url: String, prefs: DownloaderPrefs) -> Result<(), String>;
    fn download(&mut self, url: String, quality: String, folder: PathBuf, prefs: DownloaderPrefs) -> Result<(), String>;
    fn cancel(&mut self);
    fn poll(&mut self) -> Option<Event>;
}
#[derive(Default)]
pub struct Downloader {
    pub host: Option<Box<dyn Host>>,
    pub url: String,
    busy: bool,
    status: String,
    progress: f32,
    info: Option<MediaInfo>,
    checked_url: String,
    quality: String,
    last_file: Option<PathBuf>,
    imported: Option<(Arc<Project>, ItemId)>,
    context: Option<Arc<Project>>,
    placement: Option<(ItemId, Tick)>,
    import_after: bool,
}
impl Downloader {
    pub fn inspect(&self) -> Value {
        json!({"busy":self.busy,"status":self.status,"progress":self.progress,"info":self.info,"quality":self.quality,"file":self.last_file})
    }
}
pub fn valid_url(url: &str) -> bool {
    url.len() <= 8192
        && !url.chars().any(char::is_control)
        && (url.starts_with("https://") || url.starts_with("http://"))
        && url.split_once("://").is_some_and(|(_, tail)| !tail.is_empty() && !tail.contains(' ') && !tail.split('/').next().unwrap_or_default().contains('@'))
}
pub fn valid_quality(q: &str) -> bool {
    matches!(q, "best" | "audio") || q.parse::<u32>().is_ok_and(|n| (1..=8640).contains(&n))
}
fn folder(app: &FilmcraftApp) -> Result<PathBuf, String> {
    let prefs = &app.session.prefs.downloader;
    if prefs.beside_project
        && let Some(parent) = app.session.path.as_ref().and_then(|p| std::path::Path::new(p).parent())
    {
        return Ok(parent.join("Downloads"));
    }
    if !prefs.folder.trim().is_empty() {
        return Ok(PathBuf::from(&prefs.folder));
    }
    filmcraft_engine::autosave::default_data_dir().map(|p| p.join("Downloads")).ok_or_else(|| i18n::t("Choose a download folder in Settings").into())
}
pub fn command(app: &mut FilmcraftApp, id: &str, p: Value) -> Result<Value, String> {
    match id {
        "downloader.inspect" => Ok(app.downloader.inspect()),
        "downloader.cancel" => {
            if let Some(host) = app.downloader.host.as_mut() {
                host.cancel();
            }
            app.downloader.status = i18n::t("Cancelling…").into();
            Ok(Value::Null)
        }
        "downloader.probe" => {
            if app.downloader.busy {
                return Err(i18n::t("A download is already running").into());
            }
            let url = p.get("url").and_then(Value::as_str).unwrap_or(&app.downloader.url).trim().to_string();
            if !valid_url(&url) {
                return Err(i18n::t("Enter an HTTP or HTTPS video link").into());
            }
            let prefs = app.session.prefs.downloader.clone();
            app.downloader.url = url.clone();
            app.downloader.checked_url = url;
            app.downloader.info = None;
            app.downloader.progress = 0.0;
            app.downloader.quality = prefs.quality.clone();
            if !prefs.ask_quality {
                return start_download(app, prefs.quality);
            }
            app.downloader.host.as_mut().ok_or("Downloader requires the desktop app")?.probe(app.downloader.checked_url.clone(), prefs)?;
            app.downloader.busy = true;
            app.downloader.status = i18n::t("Checking available formats…").into();
            Ok(Value::Null)
        }
        "downloader.download" => {
            if app.downloader.busy {
                return Err(i18n::t("A download is already running").into());
            }
            let info = app.downloader.info.as_ref().ok_or("Check the link first")?;
            if app.downloader.url.trim() != app.downloader.checked_url {
                return Err(i18n::t("The link changed. Check it again.").into());
            }
            let quality = p.get("quality").and_then(Value::as_str).unwrap_or(&app.downloader.quality).to_string();
            if !valid_quality(&quality) || (quality == "audio" && !info.audio) || (quality != "audio" && !info.video) {
                return Err(i18n::t("This quality is not available").into());
            }
            start_download(app, quality)
        }
        "downloader.import" => {
            let at = if p.get("timeline").and_then(Value::as_bool).unwrap_or(false) {
                Some((app.session.state.active_sequence.ok_or("Open a timeline first")?, app.session.playhead()))
            } else {
                None
            };
            import(app, at)?;
            Ok(Value::Null)
        }
        "downloader.reveal" => {
            let path = app.downloader.last_file.as_ref().ok_or("No completed download")?.to_string_lossy().to_string();
            app.hooks.open_path.as_mut().ok_or("File manager unavailable")?(&path, true)?;
            Ok(Value::Null)
        }
        _ => Err("Unknown downloader command".into()),
    }
}
fn start_download(app: &mut FilmcraftApp, quality: String) -> Result<Value, String> {
    let prefs = app.session.prefs.downloader.clone();
    let dest = folder(app)?;
    app.downloader.host.as_mut().ok_or("Downloader requires the desktop app")?.download(
        app.downloader.checked_url.clone(),
        quality.clone(),
        dest,
        prefs.clone(),
    )?;
    app.downloader.context = Some(app.session.project.clone());
    app.downloader.placement = if prefs.add_to_timeline { app.session.state.active_sequence.map(|id| (id, app.session.playhead())) } else { None };
    app.downloader.import_after = prefs.import_to_project || prefs.add_to_timeline;
    app.downloader.quality = quality;
    app.downloader.progress = 0.0;
    app.downloader.busy = true;
    app.downloader.status = i18n::t("Downloading…").into();
    Ok(Value::Null)
}
fn import(app: &mut FilmcraftApp, at: Option<(ItemId, Tick)>) -> Result<(), String> {
    let path = app.downloader.last_file.clone().ok_or("No completed download")?;
    let item = if let Some((project, item)) = &app.downloader.imported
        && Arc::ptr_eq(project, &app.session.project)
        && app.session.project.item(*item).is_some()
    {
        *item
    } else {
        let result = app.session.execute("file.import", json!({"paths":[path]})).map_err(|e| e.to_string())?;
        result.get("items").and_then(Value::as_array).and_then(|v| v.first()).and_then(Value::as_u64).map(ItemId).ok_or("Import returned no media")?
    };
    app.session.execute("source.open", json!({"item":item.0})).map_err(|e| e.to_string())?;
    app.show_panel(PanelKind::Source);
    app.downloader.imported = Some((app.session.project.clone(), item));
    if let Some((sequence, time)) = at {
        if app.session.state.active_sequence != Some(sequence) {
            return Err(i18n::t("Imported into Project only: the active timeline changed.").into());
        }
        let (video, audio) = app.session.project.source_streams(item);
        app.session
            .execute("timeline.place", json!({"item":item.0,"time":time.0,"video":video,"audio":audio,"track":"new","audioTrack":"new"}))
            .map_err(|e| e.to_string())?;
        app.downloader.imported = Some((app.session.project.clone(), item));
    }
    app.downloader.status = i18n::t("Download imported").into();
    Ok(())
}
pub fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    for _ in 0..32 {
        let Some(event) = app.downloader.host.as_mut().and_then(|h| h.poll()) else { break };
        match event {
            Event::Info(result) => {
                app.downloader.busy = false;
                match result {
                    Ok(info) => {
                        if !info.video && info.audio {
                            app.downloader.quality = "audio".into();
                        }
                        app.downloader.info = Some(info);
                        app.downloader.status = i18n::t("Choose quality, then download").into();
                    }
                    Err(e) => app.downloader.status = e,
                }
            }
            Event::Progress(n, status) => {
                if n.is_finite() {
                    app.downloader.progress = n.clamp(0.0, 1.0);
                }
                app.downloader.status = status;
            }
            Event::Finished(result) => {
                app.downloader.busy = false;
                match result {
                    Ok(path) => {
                        app.downloader.progress = 1.0;
                        app.downloader.last_file = Some(path);
                        app.downloader.imported = None;
                        app.downloader.status = i18n::t("Download complete").into();
                        if app.downloader.import_after {
                            if app.downloader.context.as_ref().is_some_and(|p| Arc::ptr_eq(p, &app.session.project)) {
                                if let Err(e) = import(app, app.downloader.placement) {
                                    app.downloader.status = format!("{}: {e}", i18n::t("File saved; automatic import failed"));
                                }
                            } else {
                                app.downloader.status = i18n::t("File saved. Project changed; use Import to add it.").into();
                            }
                        }
                    }
                    Err(e) => app.downloader.status = e,
                }
                app.downloader.context = None;
                app.downloader.placement = None;
                app.status(app.downloader.status.clone());
            }
        }
    }
    if app.downloader.busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.0)), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let r = ui.add_enabled(
                !app.downloader.busy,
                egui::TextEdit::singleline(&mut app.downloader.url)
                    .hint_text(i18n::t("Paste a video link…"))
                    .desired_width(ui.available_width())
                    .char_limit(8192),
            );
            app.auto.add("downloader.url", r.rect, "Video URL");
            ui.horizontal_wrapped(|ui| {
                let r = ui.add_enabled(!app.downloader.busy, egui::Button::new(i18n::t("Download")));
                app.auto.add("downloader.probe", r.rect, "Download");
                if r.clicked() {
                    action = Some(("downloader.probe", json!({})));
                }
                let r = ui.add_enabled(app.downloader.busy, egui::Button::new(i18n::t("Cancel")));
                app.auto.add("downloader.cancel", r.rect, "Cancel download");
                if r.clicked() {
                    action = Some(("downloader.cancel", json!({})));
                }
                let r = ui.button(i18n::t("Settings"));
                app.auto.add("downloader.settings", r.rect, "Downloader settings");
                if r.clicked() {
                    let _ = crate::panels::settings::open(app, "downloader");
                }
            });
            if let Some(info) = app.downloader.info.as_ref() {
                ui.label(&info.title);
                ui.horizontal_wrapped(|ui| {
                    let r = egui::ComboBox::from_id_salt("downloader.quality")
                        .selected_text(quality_label(&app.downloader.quality))
                        .show_ui(ui, |ui| {
                            if info.video {
                                ui.selectable_value(&mut app.downloader.quality, "best".into(), i18n::t("Best available"));
                                for h in &info.heights {
                                    ui.selectable_value(&mut app.downloader.quality, h.to_string(), format!("{h}p"));
                                }
                            }
                            if info.audio {
                                ui.selectable_value(&mut app.downloader.quality, "audio".into(), i18n::t("Audio only (WAV)"));
                            }
                        })
                        .response;
                    app.auto.add("downloader.quality", r.rect, "Download quality");
                    let r = ui.add_enabled(!app.downloader.busy, egui::Button::new(i18n::t("Download selected quality")));
                    app.auto.add("downloader.start", r.rect, "Download selected quality");
                    if r.clicked() {
                        action = Some(("downloader.download", json!({})));
                    }
                });
            }
            if app.downloader.busy {
                ui.add(egui::ProgressBar::new(app.downloader.progress).show_percentage().animate(true));
            }
            ui.label(&app.downloader.status);
            if let Some(path) = &app.downloader.last_file {
                ui.label(path.to_string_lossy());
            }
            ui.horizontal_wrapped(|ui| {
                for (id, label, params) in [
                    ("downloader.import", "Import into Project", json!({})),
                    ("downloader.timeline", "Add to timeline", json!({"timeline":true})),
                    ("downloader.reveal", "Show in folder", json!({})),
                ] {
                    let r = ui.add_enabled(app.downloader.last_file.is_some() && !app.downloader.busy, egui::Button::new(i18n::t(label)));
                    app.auto.add(id, r.rect, label);
                    if r.clicked() {
                        action = Some((if id == "downloader.timeline" { "downloader.import" } else { id }, params));
                    }
                }
            });
            ui.separator();
            ui.label(i18n::t("Downloads are prepared as MP4 (H.264/AAC) or WAV for editing."));
            ui.label(i18n::t("For sign-in or age-restricted media, choose your signed-in browser in Settings → Downloader."));
        });
    });
    if let Some((id, p)) = action
        && let Err(e) = command(app, id, p)
    {
        app.downloader.status = e;
    }
}
pub fn settings_help(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (id, label, cookie) in [("downloader.pickFolder", "Choose folder…", false), ("downloader.pickCookies", "Choose cookies file…", true)] {
            let r = ui.button(i18n::t(label));
            app.auto.add(id, r.rect, label);
            if r.clicked() {
                let dialog = if cookie {
                    crate::native_dialogs::FileDialog::open_file("Netscape cookies", &["txt"])
                } else {
                    crate::native_dialogs::FileDialog::Folder { at: None }
                };
                app.pick_ui(dialog, move |app, paths| {
                    if let Some(path) = paths.first()
                        && let Some(draft) = app.ui.settings.as_mut()
                    {
                        if let Some(slot) = draft.values.pointer_mut(if cookie { "/downloader/cookieFile" } else { "/downloader/folder" }) {
                            *slot = json!(path);
                        }
                        if let Some(slot) = draft.values.pointer_mut(if cookie { "/downloader/cookieSource" } else { "/downloader/besideProject" }) {
                            *slot = if cookie { json!("file") } else { json!(false) };
                        }
                    }
                });
            }
        }
        let r = ui.button(i18n::t("Sign in to YouTube in browser"));
        app.auto.add("downloader.signIn", r.rect, "Sign in to YouTube in browser");
        if r.clicked() {
            crate::links::open(app, ui.ctx(), "https://accounts.google.com/ServiceLogin?service=youtube");
        }
    });
    ui.label(i18n::t(
        "Sign in using your browser, then select that browser above. Smotri does not ask for your password. Cookies are read again for each request.",
    ));
    ui.label(i18n::t("Until the project is saved, downloads use the chosen folder or Smotri's Downloads folder."));
    ui.label(i18n::t("Bundled tools: yt-dlp, FFmpeg and Deno. No separate installation is needed."));
}

fn quality_label(q: &str) -> String {
    match q {
        "best" => i18n::t("Best available").into(),
        "audio" => i18n::t("Audio only (WAV)").into(),
        _ => format!("{q}p"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_quality_starts_without_waiting_for_a_ui_frame() {
        struct FakeHost;
        impl Host for FakeHost {
            fn probe(&mut self, _: String, _: DownloaderPrefs) -> Result<(), String> {
                Err("automatic mode must not queue a probe".into())
            }
            fn download(&mut self, _: String, quality: String, _: PathBuf, _: DownloaderPrefs) -> Result<(), String> {
                assert_eq!(quality, "audio");
                Ok(())
            }
            fn cancel(&mut self) {}
            fn poll(&mut self) -> Option<Event> {
                None
            }
        }
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.session.prefs.downloader.ask_quality = false;
        app.session.prefs.downloader.quality = "audio".into();
        app.downloader.host = Some(Box::new(FakeHost));
        command(&mut app, "downloader.probe", json!({"url":"https://example.com/test.mp4"})).unwrap();
        assert!(app.downloader.busy);
        assert!(app.downloader.info.is_none());
    }
    #[test]
    fn hostile_links_and_qualities() {
        for u in ["file:///etc/passwd", "--exec=hello", "https://user:secret@example.org", "https://a\n--exec=x", ""] {
            assert!(!valid_url(u));
        }
        assert!(valid_url("https://www.youtube.com/watch?v=abc&x=1"));
        for q in ["0", "-1", "99999999999999", "1080;echo x"] {
            assert!(!valid_quality(q));
        }
        for q in ["audio", "best", "1080"] {
            assert!(valid_quality(q));
        }
    }
}
