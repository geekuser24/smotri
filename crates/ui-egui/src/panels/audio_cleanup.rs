//! Manual audio cleanup and take review, backed by the same command system as automation.
use crate::FilmcraftApp;
use filmcraft_time::Tick;
use serde_json::{Value, json};
#[derive(Default)]
pub struct State {
    silence: Option<Dialog>,
    duplicates: Option<Dialog>,
    preview_end: Option<Tick>,
}
pub struct Dialog {
    duplicates: bool,
    threshold: f32,
    minimum: f32,
    padding: f32,
    auto: bool,
    protect: bool,
    page: usize,
    error: String,
    message: String,
}
fn label(ru: &'static str, en: &'static str) -> &'static str {
    if crate::i18n::current() == crate::i18n::Language::Ru { ru } else { en }
}
impl Dialog {
    fn new(duplicates: bool) -> Self {
        Self { duplicates, threshold: -42., minimum: 400., padding: 120., auto: true, protect: false, page: 0, error: String::new(), message: String::new() }
    }
}
pub fn open(app: &mut FilmcraftApp, duplicates: bool) -> Result<Value, String> {
    app.show_panel(if duplicates { crate::dock::PanelKind::DuplicateTakes } else { crate::dock::PanelKind::SilenceRemoval });
    Ok(Value::Null)
}
pub fn tick(app: &mut FilmcraftApp) {
    if app.audio_cleanup.preview_end.is_some() && !app.playback.playing {
        app.audio_cleanup.preview_end = None;
    }
    if app.audio_cleanup.preview_end.is_some_and(|end| app.session.playhead() >= end) {
        app.stop();
        app.audio_cleanup.preview_end = None;
    }
}
fn button(app: &mut FilmcraftApp, ui: &mut egui::Ui, duplicates: bool, id: &str, text: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(text));
    let id = id.replacen("cleanup.", if duplicates { "cleanup.duplicates." } else { "cleanup.silence." }, 1);
    app.auto.add_response(&id, &r, text);
    r.clicked()
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: egui::Rect, duplicates: bool) {
    let ctx = ui.ctx().clone();
    let mut d = if duplicates { app.audio_cleanup.duplicates.take() } else { app.audio_cleanup.silence.take() }.unwrap_or_else(|| Dialog::new(duplicates));
    let mut status = filmcraft_engine::audio_cleanup::inspect(&mut app.session, &Value::Null).unwrap_or(Value::Null);
    let busy = !status.is_null() && !status["finished"].as_bool().unwrap_or(true);
    if status["duplicates"].as_bool() != Some(duplicates) {
        status = Value::Null;
    }
    let analysis = status.get("analysis").filter(|a| !a.is_null());
    if d.auto
        && let Some(value) = analysis.and_then(|a| a["threshold_db"].as_f64())
    {
        d.threshold = value as f32;
    }
    let running = !status.is_null() && !status["finished"].as_bool().unwrap_or(true);
    let valid = status["valid"].as_bool().unwrap_or(false);
    if running {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut cancel = false;
    let mut analyze = false;
    let mut apply = None;
    let mut preview = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.)), |ui| {
        egui::ScrollArea::vertical().id_salt(("cleanup", duplicates)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.heading(if d.duplicates {
                label("Удалить дубли", "Remove duplicate takes")
            } else {
                label("Удалить тишину", "Remove silence")
            });
            ui.label(format!("{}: {}", label("Выделено клипов", "Selected clips"), app.session.state.selection.len()));
            if d.duplicates {
                ui.label(label(
                    "Прослушай варианты и оставь нужный дубль. Остальные удалятся со сдвигом. Cmd+Z — отменить.",
                    "Preview takes and keep one. The others are ripple-deleted. Undo is available.",
                ));
            } else {
                let mut changed = false;
                ui.add_enabled_ui(!running, |ui| {
                    let r = ui.checkbox(&mut d.auto, label("Автоподбор порога по звуку (локально)", "Auto-detect threshold from audio (local)"));
                    changed |= r.changed();
                    app.auto.add_response("cleanup.auto", &r, "Auto threshold");
                    let r = ui
                        .add_enabled(!d.auto, egui::Slider::new(&mut d.threshold, -80.0..=-10.0).text(label("Порог тишины, dBFS", "Silence threshold, dBFS")));
                    changed |= r.changed();
                    app.auto.add_response("cleanup.threshold", &r, "Threshold");
                    let r = ui.add(egui::Slider::new(&mut d.minimum, 100.0..=3000.0).text(label("Минимальная пауза, мс", "Minimum silence, ms")));
                    changed |= r.changed();
                    app.auto.add_response("cleanup.minimum", &r, "Minimum silence");
                    let r = ui.add(egui::Slider::new(&mut d.padding, 40.0..=500.0).text(label("Отступ от звука, мс", "Sound padding, ms")));
                    changed |= r.changed();
                    app.auto.add_response("cleanup.padding", &r, "Padding");
                    let r =
                        ui.checkbox(&mut d.protect, label("Защитить речь через AI — ElevenLabs Scribe v2", "Protect speech with AI — ElevenLabs Scribe v2"));
                    changed |= r.changed();
                    app.auto.add_response("cleanup.protectSpeech", &r, "Protect speech");
                });
                ui.small(label(
                    "Без AI анализ выполняется локально, без отправки записи в ElevenLabs.",
                    "Without AI, audio is analyzed locally without sending it to ElevenLabs.",
                ));
                if changed && !status.is_null() {
                    let _ = app.session.execute("audioCleanup.cancel", json!({}));
                }
            }
            ui.horizontal(|ui| {
                analyze = button(
                    app,
                    ui,
                    d.duplicates,
                    "cleanup.analyze",
                    label("Анализ выделенных клипов", "Analyze selected clips"),
                    !busy && !app.session.state.selection.is_empty(),
                );
                if running {
                    ui.spinner();
                    ui.label(format!("{}%", status["progress"].as_u64().unwrap_or(0)));
                }
            });
            if let Some(e) = status["error"].as_str() {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            if !d.message.is_empty() {
                ui.label(&d.message);
            }
            if !d.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
            }
            if !status.is_null() && !valid {
                ui.colored_label(egui::Color32::YELLOW, label("Проект изменился. Запусти анализ заново.", "Project changed. Analyze again."));
            }
            if let Some(a) = analysis {
                if !d.duplicates {
                    let ranges = a["ranges"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                    ui.label(format!("{}: {}", label("Найдено пауз", "Silences found"), ranges.len()));
                    ui.label(format!(
                        "{}: {:.1} dBFS · {}: {:.1} dBFS · {}: {:.1} dBFS",
                        label("Тихие участки", "Quiet level"),
                        a["noise_db"].as_f64().unwrap_or(-120.),
                        label("Звук", "Sound level"),
                        a["peak_db"].as_f64().unwrap_or(-120.),
                        label("Порог", "Threshold"),
                        a["threshold_db"].as_f64().unwrap_or(-42.)
                    ));
                    if button(
                        app,
                        ui,
                        d.duplicates,
                        "cleanup.apply",
                        label("OK — удалить тишину со сдвигом", "OK — ripple-delete silence"),
                        valid && !running && !ranges.is_empty(),
                    ) {
                        apply = Some(json!({}));
                    }
                } else {
                    let groups = a["groups"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                    d.page = d.page.min(groups.len().saturating_sub(1));
                    if groups.is_empty() {
                        ui.label(label("Безопасных повторных дублей не найдено.", "No safely bounded repeated takes found."));
                    }
                    if let Some(group) = groups.get(d.page) {
                        ui.horizontal(|ui| {
                            if button(app, ui, d.duplicates, "cleanup.previous", "←", d.page > 0) {
                                d.page -= 1;
                            }
                            ui.label(format!("{} {} / {}", label("Группа", "Group"), d.page + 1, groups.len()));
                            if button(app, ui, d.duplicates, "cleanup.next", "→", d.page + 1 < groups.len()) {
                                d.page += 1;
                            }
                        });
                        let resolved = group["resolved"].as_bool().unwrap_or(false);
                        if resolved {
                            ui.label(label("Готово — выбранный дубль сохранён.", "Done — selected take kept."));
                        }
                        egui::ScrollArea::vertical().max_height(270.).show(ui, |ui| {
                            if let Some(takes) = group["takes"].as_array() {
                                for (i, t) in takes.iter().enumerate() {
                                    ui.group(|ui| {
                                        ui.label(format!("{} {}", label("Дубль", "Take"), i + 1));
                                        ui.label(t["text"].as_str().unwrap_or(""));
                                        ui.horizontal(|ui| {
                                            if button(
                                                app,
                                                ui,
                                                d.duplicates,
                                                &format!("cleanup.preview.{i}"),
                                                label("▶ Прослушать / посмотреть", "▶ Preview"),
                                                valid && !resolved,
                                            ) {
                                                preview = Some(json!({"group":d.page,"take":i}));
                                            }
                                            if button(
                                                app,
                                                ui,
                                                d.duplicates,
                                                &format!("cleanup.keep.{i}"),
                                                label("Оставить этот дубль", "Keep this take"),
                                                valid && !resolved,
                                            ) {
                                                apply = Some(json!({"group":d.page,"keep":i}));
                                            }
                                        });
                                    });
                                }
                            }
                        });
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if button(app, ui, d.duplicates, "cleanup.stop", label("■ Стоп", "■ Stop"), app.playback.playing) {
                    app.stop();
                    app.audio_cleanup.preview_end = None;
                }
                cancel = button(app, ui, d.duplicates, "cleanup.cancel", label("Сбросить анализ", "Reset analysis"), !status.is_null());
            });
        });
    });
    if analyze {
        app.stop();
        app.audio_cleanup.preview_end = None;
        d.message.clear();
        let p = json!({"duplicates":d.duplicates,"thresholdDb":d.threshold,"minimumMs":d.minimum,"paddingMs":d.padding,"autoThreshold":d.auto,"protectSpeech":d.protect});
        match app.session.execute("audioCleanup.analyze", p) {
            Ok(_) => {
                d.error.clear();
                d.page = 0;
            }
            Err(e) => d.error = e.to_string(),
        }
    }
    if let Some(p) = preview {
        match app.session.execute("audioCleanup.preview", p) {
            Ok(v) => {
                if let (Some(a), Some(b)) = (v["start"].as_i64(), v["end"].as_i64()) {
                    app.stop();
                    app.session.set_playhead(Tick(a));
                    app.audio_cleanup.preview_end = Some(Tick(b));
                    app.play(1.);
                }
            }
            Err(e) => d.error = e.to_string(),
        }
    }
    if let Some(p) = apply {
        app.stop();
        app.audio_cleanup.preview_end = None;
        match app.session.execute("audioCleanup.apply", p) {
            Ok(_) => {
                d.error.clear();
                if d.duplicates {
                    d.page += 1;
                } else {
                    d.message = label("Готово. Тишина удалена. Cmd+Z — отменить.", "Done. Silence removed. Undo is available.").into();
                }
            }
            Err(e) => d.error = e.to_string(),
        }
    }
    if cancel {
        app.stop();
        app.audio_cleanup.preview_end = None;
        let _ = app.session.execute("audioCleanup.cancel", json!({}));
        d.error.clear();
        d.message.clear();
    }
    if duplicates {
        app.audio_cleanup.duplicates = Some(d);
    } else {
        app.audio_cleanup.silence = Some(d);
    }
}
