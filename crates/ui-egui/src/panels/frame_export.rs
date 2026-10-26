//! Monitor Export Frame settings, with captured target/time and a destination folder.
use crate::native_dialogs::FileDialog;
use crate::{
    FilmcraftApp,
    icons::{self, Icon},
};
use filmcraft_engine::keyboard::StillFormat;
use filmcraft_engine::keyboard::{FrameExportTarget, frame_export_target};
use serde_json::{Value, json};

#[derive(Clone)]
struct Draft {
    target: FrameExportTarget,
    timecode: String,
    name: String,
    auto_name: String,
    base_name: String,
    format: StillFormat,
    depth: u8,
    folder: String,
    import: bool,
    replace: Option<String>,
}
fn draft_id() -> egui::Id {
    egui::Id::new("frame-export-draft")
}
fn clean(name: &str) -> String {
    name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect()
}

fn export_stem(name: &str) -> String {
    let mut stem = clean(name);
    let lower = stem.to_ascii_lowercase();
    for suffix in [".png", ".jpg", ".jpeg", ".tif", ".tiff", ".bmp"] {
        if lower.ends_with(suffix) {
            stem.truncate(stem.len().saturating_sub(suffix.len()));
            break;
        }
    }
    if stem.is_empty() { "Frame".into() } else { stem }
}

pub fn open(app: &mut FilmcraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    let target = frame_export_target(&app.session, params).map_err(|e| e.to_string())?;
    let (rate, drop_frame) = if target.source {
        let view = filmcraft_engine::clip_ops::source_view(&app.session, target.item).ok_or("Source clip is unavailable")?;
        (view.rate, false)
    } else {
        let sequence = app.session.project.sequence(target.item).ok_or("Program sequence is unavailable")?;
        (sequence.settings.frame_rate, sequence.settings.drop_frame)
    };
    let timecode = filmcraft_time::format_time(target.time, rate, drop_frame, filmcraft_time::TimeDisplay::Timecode, 48000);
    if target.source {
        app.stop_source();
    } else if app.playback.playing {
        app.stop();
    }
    let folder = ctx.data(|m| m.get_temp::<String>(egui::Id::new("frame-export-folder"))).unwrap_or_else(|| {
        if cfg!(target_arch = "wasm32") {
            return "/exports".into();
        }
        app.session.path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into())
    });
    let name = if target.source { std::path::Path::new(&target.name).file_stem().and_then(|s| s.to_str()).unwrap_or(&target.name) } else { &target.name };
    let base_name = export_stem(name);
    let format = ctx.data(|m| m.get_temp::<StillFormat>(egui::Id::new("frame-export-format-last"))).unwrap_or(StillFormat::Png);
    let name = unique_name(app, &folder, &base_name, format)?;
    ctx.data_mut(|m| {
        m.insert_temp(
            draft_id(),
            Some(Draft { target, timecode, auto_name: name.clone(), base_name, name, format, depth: 8, folder, import: false, replace: None }),
        )
    });
    ctx.request_repaint();
    Ok(json!({"dialog":"exportFrame"}))
}

/// Reserve no files here: the explicit overwrite check still runs at export time.
fn unique_name(app: &FilmcraftApp, folder: &str, stem: &str, format: StillFormat) -> Result<String, String> {
    let folder = filmcraft_engine::export_tools::expand_home(folder.trim());
    for n in 0..10_000 {
        let name = if n == 0 { stem.to_string() } else { format!("{stem} ({n})") };
        let path = std::path::Path::new(&folder).join(format!("{name}.{}", format.extension()));
        match app.session.services.file_size(&path.to_string_lossy()) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(name),
            Err(e) => return Err(tlf!("Cannot check export destination: {error}", error = e)),
        }
    }
    Err(tl!("Choose another frame name: all automatic names are occupied.").into())
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())).flatten().is_some()
}

fn refresh_name(app: &FilmcraftApp, d: &mut Draft) -> Result<(), String> {
    if d.name == d.auto_name {
        d.name = unique_name(app, &d.folder, &d.base_name, d.format)?;
        d.auto_name = d.name.clone();
    }
    d.replace = None;
    Ok(())
}

pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) else { return };
    let enter = !egui::Popup::is_any_open(ctx) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let previous_destination = (d.folder.clone(), d.format);
    let mut close = false;
    let mut apply = false;
    let mut browse = false;
    let mut replace = false;
    let mut elems = Vec::new();
    let mut popup_rects = Vec::new();
    let frame = egui::Frame::new().fill(app.tokens.panel_bg).inner_margin(0).corner_radius(6);
    let position_id = egui::Id::new("frame-export-position");
    let position = ctx.data(|m| m.get_temp::<egui::Pos2>(position_id));
    let mut movement = egui::Vec2::ZERO;
    // Only the custom white header moves the window; form controls keep their own gestures.
    let window_id = egui::Id::new("frame-export-window");
    let mut window = egui::Window::new(tl!("Export Frame"))
        .id(window_id)
        .title_bar(false)
        .movable(false)
        .collapsible(false)
        .resizable(false)
        .frame(frame)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    if let Some(position) = position {
        window = window.current_pos(position);
    }
    let shown = window.show(ctx, |ui| {
        ui.set_width(440.0);
        let (bar, _) = ui.allocate_exact_size(egui::vec2(440.0, 30.0), egui::Sense::hover());
        ui.painter().rect_filled(bar, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, egui::Color32::WHITE);
        ui.painter().text(
            bar.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            tl!("Export Frame"),
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(90),
        );
        let x = egui::Rect::from_center_size(bar.right_center() - egui::vec2(16.0, 0.0), egui::vec2(24.0, 24.0));
        let header = egui::Rect::from_min_max(bar.min, egui::pos2(x.left(), bar.bottom()));
        let drag = ui.interact(header, egui::Id::new("export-frame-header"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
        if drag.dragged() {
            movement = drag.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        elems.push(("exportFrame.header".into(), header, tl!("Move Export Frame").into()));
        let response = ui.interact(x, egui::Id::new("export-frame-close"), egui::Sense::click()).on_hover_text(tl!("Close"));
        icons::paint(ui.painter(), x.shrink(7.0), Icon::Close, egui::Color32::from_gray(90));
        elems.push(("exportFrame.close".to_string(), x, tl!("Close").to_string()));
        close = response.clicked();
        egui::Frame::NONE.inner_margin(14).show(ui, |ui| {
            egui::Grid::new("frame-export-settings").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label(tl!("Name:"));
                let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(300.0));
                elems.push(("exportFrame.name".into(), r.rect, d.name.clone()));
                ui.end_row();
                ui.label(tl!("Format:"));
                let combo = egui::ComboBox::from_id_salt("frame-export-format").width(292.0).selected_text(d.format.label()).show_ui(ui, |ui| {
                    for format in StillFormat::ALL {
                        let r = ui.selectable_value(&mut d.format, format, format.label());
                        elems.push((format!("exportFrame.format.{}", format.extension()), r.rect, format.label().into()));
                    }
                    ui.min_rect()
                });
                if let Some(rect) = combo.inner {
                    popup_rects.push(rect);
                }
                elems.push(("exportFrame.format".into(), combo.response.rect, d.format.label().into()));
                ui.end_row();
                if !d.format.supports_16() {
                    d.depth = 8;
                }
                ui.label(tl!("Depth:"));
                let combo =
                    egui::ComboBox::from_id_salt("frame-export-depth").width(292.0).selected_text(tlf!("{bits} Bit", bits = d.depth)).show_ui(ui, |ui| {
                        for bits in [8, 16] {
                            let r = ui.add_enabled(bits == 8 || d.format.supports_16(), egui::Button::selectable(d.depth == bits, tlf!("{bits} Bit", bits)));
                            if r.clicked() {
                                d.depth = bits;
                            }
                            elems.push((format!("exportFrame.depth.{bits}"), r.rect, tlf!("{bits} Bit", bits)));
                        }
                        ui.min_rect()
                    });
                if let Some(rect) = combo.inner {
                    popup_rects.push(rect);
                }
                elems.push(("exportFrame.depth".into(), combo.response.rect, tlf!("{bits} Bit", bits = d.depth)));
                ui.end_row();
                ui.label(tl!("Path:"));
                let r = ui.add(egui::TextEdit::singleline(&mut d.folder).desired_width(300.0));
                elems.push(("exportFrame.path".into(), r.rect, d.folder.clone()));
                ui.end_row();
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let r = ui.checkbox(&mut d.import, tl!("Import into project"));
                elems.push(("exportFrame.import".into(), r.rect, d.import.to_string()));
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.add_enabled(FileDialog::Folder { at: Some(String::new()) }.available(&app.hooks), egui::Button::new(tl!("Browse…")));
                    browse = r.clicked();
                    elems.push(("exportFrame.browse".into(), r.rect, tl!("Browse").into()));
                });
            });
            ui.add_space(12.0);
            if let Some(path) = d.replace.clone() {
                ui.label(tlf!("This file already exists: {path}", path));
                ui.horizontal(|ui| {
                    let r = ui.button(tl!("Replace"));
                    replace = r.clicked();
                    elems.push(("exportFrame.replace".into(), r.rect, tl!("Replace").into()));
                    let r = ui.button(tl!("Keep existing file"));
                    if r.clicked() {
                        d.replace = None;
                    }
                    elems.push(("exportFrame.keep".into(), r.rect, tl!("Keep existing file").into()));
                });
            } else {
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.button(tl!("Cancel"));
                    close |= r.clicked();
                    elems.push(("exportFrame.cancel".into(), r.rect, tl!("Cancel").into()));
                    let r = ui.add_enabled(
                        !d.name.trim().is_empty() && !d.folder.trim().is_empty(),
                        egui::Button::new(egui::RichText::new(tl!("OK")).color(egui::Color32::WHITE)).fill(app.tokens.accent),
                    );
                    apply = r.clicked();
                    elems.push(("exportFrame.export".into(), r.rect, tl!("OK").into()));
                });
            }
            ui.add_space(12.0);
            ui.separator();
            ui.label(format!("{} — {} × {}", if d.target.source { tl!("Source") } else { tl!("Program") }, d.target.width, d.target.height));
            ui.label(&d.target.name);
            let r = ui.label(tlf!("Frame time: {time}", time = &d.timecode));
            elems.push(("exportFrame.timecode".into(), r.rect, d.timecode.clone()));
        });
    });
    if let Some(shown) = &shown {
        // A press outside the dialog and its own dropdowns cancels the draft. Background
        // controls still receive the same press, so switching modes or menus works normally.
        close |= ctx.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.response.rect.contains(p) && !popup_rects.iter().any(|r| r.contains(p)))
        });
    }
    if shown.is_some() && movement != egui::Vec2::ZERO {
        // Keep the unrounded requested pivot between drag frames so subpixel deltas cannot accumulate rounding drift.
        if let Some(pivot) = position.or_else(|| egui::containers::AreaState::load(ctx, window_id).and_then(|state| state.pivot_pos)) {
            ctx.data_mut(|m| m.insert_temp(position_id, pivot + movement));
        }
        ctx.request_repaint();
    }
    for (id, rect, label) in elems {
        app.auto.add(&id, rect, &label);
    }
    close |= crate::widgets::escape_closes(ctx);
    if previous_destination != (d.folder.clone(), d.format)
        && let Err(e) = refresh_name(app, &mut d)
    {
        app.ui.status = e;
    }
    apply |= enter && d.replace.is_none() && !d.name.trim().is_empty() && !d.folder.trim().is_empty();
    if browse && !close {
        let folder = filmcraft_engine::export_tools::expand_home(d.folder.trim());
        let ctx = ctx.clone();
        // into the dialog's stored draft, if it is still open when the folder comes back
        app.pick_ui(FileDialog::Folder { at: Some(folder) }, move |app, paths| {
            let Some(path) = paths.into_iter().next() else { return };
            if let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) {
                d.folder = path;
                if let Err(e) = refresh_name(app, &mut d) {
                    app.ui.status = e;
                }
                ctx.data_mut(|m| m.insert_temp(draft_id(), Some(d)));
            }
        });
    }
    if (apply || replace) && !close {
        let stem = export_stem(&d.name);
        let ext = d.format.extension();
        let suffix = format!(".{ext}");
        let name = if stem.to_ascii_lowercase().ends_with(&suffix) { stem } else { format!("{stem}{suffix}") };
        let folder = filmcraft_engine::export_tools::expand_home(d.folder.trim());
        let path = std::path::Path::new(&folder).join(name).to_string_lossy().into_owned();
        let allowed = match app.session.services.file_size(&path) {
            Ok(_) => replace && d.replace.as_deref() == Some(path.as_str()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                app.ui.status = tlf!("Cannot check export destination: {error}", error = e);
                false
            }
        };
        if !allowed && app.session.services.file_size(&path).is_ok() {
            d.replace = Some(path.clone());
        }
        if allowed {
            let mut params =
                json!({"path":path,"format":ext,"depth":d.depth,"import":d.import,"target":if d.target.source{"source"}else{"program"},"time":d.target.time.0});
            params[if d.target.source { "item" } else { "sequence" }] = json!(d.target.item.0);
            match app.session.execute("file.exportFrame", params) {
                Ok(result) => {
                    app.ui.status = tlf!("Exported frame to {path}", path = result["path"].as_str().unwrap_or_default());
                    ctx.data_mut(|m| m.insert_temp(egui::Id::new("frame-export-folder"), d.folder.clone()));
                    ctx.data_mut(|m| m.insert_temp(egui::Id::new("frame-export-format-last"), d.format));
                    close = true;
                }
                Err(e) => app.ui.status = e.to_string(),
            }
        }
    }
    ctx.data_mut(|m| m.insert_temp(draft_id(), if close { None } else { Some(d) }));
}
