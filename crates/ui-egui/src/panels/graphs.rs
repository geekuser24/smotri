//! Original dockable normalized easing editor. Artwork is plotted from our curve mathematics.
use crate::{FilmcraftApp, dock::PanelKind, state::KeyframeRef};
use egui::{Rect, Sense, Stroke, pos2, vec2};
use filmcraft_project::easing::{EasingCurve, EasingPreset, presets};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphDraft {
    pub curve: EasingCurve,
    pub name: String,
    pub status: String,
}
fn l(ru: &'static str, en: &'static str) -> &'static str {
    if crate::i18n::current() == crate::i18n::Language::Ru { ru } else { en }
}
fn library(app: &FilmcraftApp) -> Vec<EasingPreset> {
    let mut p = presets();
    p.extend(app.session.prefs.easing_presets.iter().filter(|p| p.curve.valid()).take(128).cloned());
    p
}
pub fn command(app: &mut FilmcraftApp, id: &str, p: Value) -> Result<Value, String> {
    match id {
        "graphs.open" => {
            if let Some(k) = app.ui.keyframe_selection.first()
                && let Some((_, it)) = app.session.active_sequence().and_then(|q| q.find_item(filmcraft_project::ClipId(k.clip)))
                && let Some(e) = it.effects.get(k.effect)
                && let Some(param) = match k.mask {
                    Some(m) => e.masks.get(m).and_then(|m| m.param(&k.param)),
                    None => e.params.get(&k.param),
                }
                && let Some(curve) = param.keyframes.iter().find(|f| f.time == k.time).and_then(|f| f.easing).filter(|c| c.valid())
            {
                app.ui.graphs.curve = curve;
            }
            app.show_panel(PanelKind::Graphs);
        }
        "graphs.set" => {
            let curve = if let Some(name) = p.get("preset").and_then(Value::as_str) {
                library(app).into_iter().find(|v| v.name == name).ok_or("Unknown preset")?.curve
            } else {
                serde_json::from_value::<EasingCurve>(p.get("curve").cloned().unwrap_or(Value::Null)).map_err(|_| "Need four curve coordinates")?
            };
            if !curve.valid() {
                return Err("Invalid curve coordinates".into());
            }
            if let Some(name) = p.get("name").and_then(Value::as_str) {
                if name.len() > 120 || name.chars().any(char::is_control) {
                    return Err("Invalid preset name".into());
                }
                app.ui.graphs.name = name.to_string();
            }
            app.ui.graphs.curve = curve;
        }
        "graphs.apply" => {
            let result = app
                .session
                .execute("effects.applyEasing", json!({"keyframes":app.ui.keyframe_selection,"curve":app.ui.graphs.curve}))
                .map_err(|e| e.to_string())?;
            app.ui.graphs.status = format!("{}: {}", l("Применено сегментов", "Applied segments"), result["segments"]);
            return Ok(result);
        }
        "graphs.inspect" => {}
        _ => return Err("Unknown graph command".into()),
    }
    Ok(json!({"draft":app.ui.graphs,"selection":app.ui.keyframe_selection,"presets":library(app)}))
}
fn plot(ui: &egui::Ui, r: Rect, c: EasingCurve, color: egui::Color32) {
    let points = (0..=64)
        .map(|i| {
            let x = i as f64 / 64.0;
            pos2(r.left() + x as f32 * r.width(), r.bottom() - c.sample(x) as f32 * r.height())
        })
        .collect();
    ui.painter().with_clip_rect(r.expand(r.height() * 0.6).intersect(ui.clip_rect())).add(egui::Shape::line(points, Stroke::new(1.8, color)));
}
fn preset_button(app: &mut FilmcraftApp, ui: &mut egui::Ui, p: &EasingPreset, id: &str) -> bool {
    let r = ui
        .horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(38.0, 29.0), Sense::hover());
            plot(ui, r.shrink2(vec2(3.0, 7.0)), p.curve, app.tokens.accent);
            ui.selectable_label(app.ui.graphs.curve == p.curve, &p.name)
        })
        .inner;
    app.auto.add(id, r.rect, &p.name);
    r.clicked()
}
pub fn context_menu(app: &mut FilmcraftApp, ui: &mut egui::Ui, target: &KeyframeRef, actions: &mut Vec<(String, Value)>) {
    ui.menu_button(l("Пресеты графиков", "Curve presets"), |ui| {
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            for (i, p) in library(app).iter().enumerate() {
                if preset_button(app, ui, p, &format!("effectControls.curvePreset.{i}")) {
                    if !app.ui.keyframe_selection.contains(target) {
                        app.ui.keyframe_selection = vec![target.clone()];
                    }
                    app.ui.graphs.curve = p.curve;
                    actions.push(("graphs.apply".into(), json!({})));
                    ui.close();
                }
            }
        });
    });
    let r = ui.button(l("Графики…", "Graphs…"));
    app.auto.add("effectControls.keyframeMenu.graphs", r.rect, "Graphs");
    if r.clicked() {
        if !app.ui.keyframe_selection.contains(target) {
            app.ui.keyframe_selection = vec![target.clone()];
        }
        actions.push(("graphs.open".into(), json!({})));
        ui.close();
    }
    ui.separator();
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(10.0)).id_salt("graphs"));
    if !app.ui.graphs.curve.valid() {
        app.ui.graphs.curve = EasingCurve::default();
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut ui, |ui| {
        ui.label(l("Выделите два или несколько соседних ключей одного параметра.", "Select two or more consecutive keys of a parameter."));
        let size = vec2(ui.available_width().max(80.0), (rect.height() * 0.42).clamp(120.0, 300.0));
        let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
        let base = canvas.shrink(22.0);
        let [_, y1, _, y2] = app.ui.graphs.curve.0;
        let mut bounds = (y1.min(y2).min(0.0) - 0.15, y1.max(y2).max(1.0) + 0.15);
        let range_id = ui.id().with("graph-range");
        if ui.input(|i| i.pointer.any_down()) {
            bounds = ui.data(|d| d.get_temp(range_id)).unwrap_or(bounds);
        } else {
            ui.data_mut(|d| d.insert_temp(range_id, bounds));
        }
        let (low, high) = bounds;
        let height = base.height() / (high - low) as f32;
        let bottom = base.bottom() + low as f32 * height;
        let area = Rect::from_min_max(pos2(base.left(), bottom - height), pos2(base.right(), bottom));
        let point = |x: f64, y: f64| pos2(area.left() + x as f32 * area.width(), area.bottom() - y as f32 * area.height());
        let paint = ui.painter().with_clip_rect(canvas);
        paint.rect_filled(canvas, 6.0, app.tokens.keyframe_plot_bg);
        for n in 0..=4 {
            let f = n as f64 / 4.0;
            paint.line_segment([point(f, 0.0), point(f, 1.0)], Stroke::new(1.0, app.tokens.keyframe_plot_grid));
            paint.line_segment([point(0.0, f), point(1.0, f)], Stroke::new(1.0, app.tokens.keyframe_plot_grid));
        }
        let [x1, y1, x2, y2] = app.ui.graphs.curve.0;
        for (i, (x, y, anchor)) in [(x1, y1, point(0.0, 0.0)), (x2, y2, point(1.0, 1.0))].into_iter().enumerate() {
            let hp = point(x, y);
            paint.line_segment([anchor, hp], Stroke::new(1.0, app.tokens.text_dim));
            paint.circle_filled(hp, 5.0, app.tokens.accent);
            let r = ui.interact(Rect::from_center_size(hp, vec2(20.0, 20.0)), ui.id().with(("handle", i)), Sense::drag());
            app.auto.add(&format!("graphs.handle.{i}"), r.rect, "Curve handle");
            if r.dragged()
                && let Some(p) = r.interact_pointer_pos()
            {
                if let Some(v) = app.ui.graphs.curve.0.get_mut(i * 2) {
                    *v = ((p.x - area.left()) / area.width()).clamp(0.0, 1.0) as f64;
                }
                if let Some(v) = app.ui.graphs.curve.0.get_mut(i * 2 + 1) {
                    *v = ((area.bottom() - p.y) / area.height()).clamp(-2.0, 3.0) as f64;
                }
            }
        }
        plot(ui, area, app.ui.graphs.curve, app.tokens.text);
        ui.horizontal_wrapped(|ui| {
            for (i, label) in ["X1", "Y1", "X2", "Y2"].iter().enumerate() {
                ui.label(*label);
                if let Some(v) = app.ui.graphs.curve.0.get_mut(i) {
                    let r = ui.add(egui::DragValue::new(v).speed(0.01).range(if i % 2 == 0 { 0.0..=1.0 } else { -2.0..=3.0 }).fixed_decimals(2));
                    app.auto.add(&format!("graphs.coordinate.{i}"), r.rect, label);
                }
            }
        });
        let r = ui.add_sized([ui.available_width(), 30.0], egui::Button::new(l("Применить к выделенным ключам", "Apply to selected keys")));
        app.auto.add("graphs.apply", r.rect, "Apply");
        if r.clicked()
            && let Err(e) = command(app, "graphs.apply", json!({}))
        {
            app.ui.graphs.status = e;
        }
        if !app.ui.graphs.status.is_empty() {
            ui.label(&app.ui.graphs.status);
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.ui.graphs.name)
                    .hint_text(l("Название своего пресета", "Custom preset name"))
                    .char_limit(120)
                    .desired_width(160.0),
            );
            app.auto.add("graphs.name", r.rect, "Preset name");
            let r = ui.button(l("Сохранить", "Save"));
            app.auto.add("graphs.save", r.rect, "Save");
            if r.clicked() {
                match app.session.execute("easing.save", json!({"name":app.ui.graphs.name,"curve":app.ui.graphs.curve})) {
                    Ok(_) => app.ui.graphs.status = l("Пресет сохранён", "Preset saved").into(),
                    Err(e) => app.ui.graphs.status = e.to_string(),
                }
            }
            let r = ui.button(l("Удалить свой", "Delete custom"));
            app.auto.add("graphs.delete", r.rect, "Delete custom");
            if r.clicked() {
                match app.session.execute("easing.delete", json!({"name":app.ui.graphs.name})) {
                    Ok(_) => app.ui.graphs.status = l("Пресет удалён", "Preset deleted").into(),
                    Err(e) => app.ui.graphs.status = e.to_string(),
                }
            }
        });
        ui.separator();
        let list = library(app);
        let columns = ((ui.available_width() / 175.0) as usize).clamp(1, 4);
        egui::Grid::new("curve-presets").num_columns(columns).spacing(vec2(8.0, 3.0)).show(ui, |ui| {
            for (i, p) in list.iter().enumerate() {
                if preset_button(app, ui, p, &format!("graphs.preset.{i}")) {
                    app.ui.graphs.curve = p.curve;
                    app.ui.graphs.name = p.name.clone();
                    app.ui.graphs.status.clear();
                }
                if (i + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
    });
}
