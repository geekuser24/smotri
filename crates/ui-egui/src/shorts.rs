//! Dockable vertical video workflow; engine commands own every project edit.
use crate::FilmcraftApp;
use filmcraft_engine::shorts::Options;
use filmcraft_project::ItemId;
use filmcraft_time::Tick;
use serde_json::{Value, json};
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct State {
    #[serde(skip)]
    pub key: String,
    pub source: Option<ItemId>,
    pub duration: u32,
    pub options: Options,
    pub selected: std::collections::BTreeSet<usize>,
    pub created: Vec<ItemId>,
    pub export_selected: std::collections::BTreeSet<ItemId>,
    pub folder: String,
    pub message: String,
    pub preview_source: Option<ItemId>,
    pub preview_end: Option<Tick>,
}
fn l(ru: &'static str, en: &'static str) -> &'static str {
    if crate::i18n::current() == crate::i18n::Language::Ru { ru } else { en }
}
fn button(app: &mut FilmcraftApp, ui: &mut egui::Ui, id: &str, text: &str, on: bool) -> bool {
    let r = ui.add_enabled(on, egui::Button::new(text));
    app.auto.add_response(id, &r, text);
    r.clicked()
}
fn choice(app: &mut FilmcraftApp, ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let r = egui::ComboBox::from_id_salt(id).selected_text(options.iter().find(|(v, _)| *v == value).map(|(_, t)| *t).unwrap_or(value)).show_ui(ui, |ui| {
        for (v, t) in options {
            let r = ui.selectable_value(value, (*v).into(), *t);
            app.auto.add_response(&format!("{id}.{v}"), &r, t);
        }
    });
    app.auto.add_response(id, &r.response, id);
}
pub fn tick(app: &mut FilmcraftApp) {
    if !app.playback.playing || app.shorts.preview_source != app.session.state.active_sequence {
        app.shorts.preview_end = None;
    }
    if app.shorts.preview_end.is_some_and(|end| app.session.playhead() >= end) {
        app.stop();
        app.shorts.preview_end = None;
    }
}
pub fn settings(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    ui.heading(l("Вертикальные видео", "Vertical Videos"));
    ui.label("OpenAI · GPT-5.4 mini");
    ui.small(l("В модель отправляется только расшифровка с таймкодами. До 12 предложений за анализ; максимум 2 часа аудио. Расшифровка ElevenLabs оплачивается отдельно.","Only timestamped transcript text is sent to the model. Up to 12 suggestions per analysis; maximum 2 hours of audio. ElevenLabs transcription is billed separately."));
    ui.label(if app.session.shorts_analyst.as_ref().is_some_and(|a| a.configured()) {
        l("Ключ OpenAI сохранён", "OpenAI key configured")
    } else {
        l("Добавь ключ OpenAI", "Add OpenAI API key")
    });
    let r = ui.add(egui::TextEdit::singleline(&mut app.shorts.key).password(true).hint_text("OpenAI API key"));
    app.auto.add_response("shorts.apiKey", &r, "OpenAI API key");
    ui.separator();
    ui.label(l("Общий ключ ElevenLabs для Jarvis, расшифровки и вертикальных видео", "Shared ElevenLabs key for Jarvis, transcription and vertical videos"));
    crate::jarvis::shared_key_ui(app, ui);
    ui.small(l(
        "Оценка интереса — редакционная подсказка, не прогноз просмотров. Вариации оформления не гарантируют уникальность или охваты.",
        "Editorial scores are suggestions, not view predictions. Cosmetic variants do not guarantee uniqueness or reach.",
    ));
}
pub fn apply_key(app: &mut FilmcraftApp) -> Result<(), String> {
    if !app.shorts.key.trim().is_empty() {
        app.session.shorts_analyst.as_ref().ok_or("OpenAI requires desktop app")?.save_key(&app.shorts.key)?;
        app.shorts.key.clear();
    }
    Ok(())
}
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    let mut d = std::mem::take(&mut app.shorts);
    if d.duration == 0 {
        d.duration = 60;
    }
    if d.source.is_none() {
        d.source = app.session.state.active_sequence;
    }
    let status = app.session.execute("shorts.status", json!({})).unwrap_or(Value::Null);
    let busy = !status.is_null() && status["finished"] == false;
    let valid = status["valid"] == true && status["source"].as_u64().map(ItemId) == d.source && status["duration"].as_u64() == Some(u64::from(d.duration));
    let proposals = status["proposals"].as_array().cloned().unwrap_or_default();
    d.created = app.session.project.sequences().filter(|i| i.metadata.contains_key("SmotriShort")).map(|i| i.id).collect();
    d.export_selected.retain(|id| d.created.contains(id));
    let seqs: Vec<_> = app.session.project.sequences().map(|i| (i.id, i.name.clone())).collect();
    let mut analyze = false;
    let mut create = None;
    let mut preview = None;
    let mut folder = false;
    let mut export = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(8.)), |ui| {
        egui::ScrollArea::vertical().id_salt("shorts.panel").show(ui, |ui| {
            ui.heading(l("Вертикальные видео", "Vertical Videos"));
            ui.label(l("1. Анализ → 2. Просмотр и выбор → 3. Создать → 4. Экспорт", "1. Analyze → 2. Preview and select → 3. Create → 4. Export"));
            ui.add_enabled_ui(!busy, |ui| {
                let r = egui::ComboBox::from_id_salt("shorts.sequence")
                    .selected_text(seqs.iter().find(|(i, _)| Some(*i) == d.source).map(|(_, n)| n.as_str()).unwrap_or("—"))
                    .show_ui(ui, |ui| {
                        for (id, name) in &seqs {
                            let r = ui.selectable_value(&mut d.source, Some(*id), name);
                            app.auto.add_response(&format!("shorts.sequence.{}", id.0), &r, name);
                        }
                    });
                app.auto.add_response("shorts.sequence", &r.response, "Source sequence");
                ui.horizontal_wrapped(|ui| {
                    ui.label(l("Максимальная длительность:", "Maximum duration:"));
                    for sec in [30, 60, 90] {
                        let r = ui.selectable_value(&mut d.duration, sec, format!("{sec} s"));
                        app.auto.add_response(&format!("shorts.duration.{sec}"), &r, "Duration");
                    }
                });
                analyze = button(app, ui, "shorts.analyze", l("Анализ", "Analyze"), d.source.is_some());
            });
            if busy {
                ui.spinner();
                ui.label(format!("{}% · {}", status["progress"].as_u64().unwrap_or(0), status["status"].as_str().unwrap_or("")));
                if button(app, ui, "shorts.cancel", l("Отмена", "Cancel"), true) {
                    let _ = app.session.execute("jobs.cancel", json!({"id":status["job"]}));
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            }
            if let Some(e) = status["error"].as_str() {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            ui.separator();
            let options_header = egui::CollapsingHeader::new(l("Оформление и версии", "Layout and variants")).id_salt("shorts.options").show(ui, |ui| {
                ui.label(l("Формат и компоновка", "Format and layout"));
                choice(app, ui, "shorts.format", &mut d.options.format, &[("9:16", "9:16"), ("1:1", "1:1"), ("4:5", "4:5"), ("16:9", "16:9")]);
                choice(
                    app,
                    ui,
                    "shorts.layout",
                    &mut d.options.layout,
                    &[
                        ("auto", l("Микс компоновок", "Layout mix")),
                        ("fill", l("Авто-кадрирование / заполнение", "Auto reframe / fill")),
                        ("background", l("Видео целиком + размытый фон", "Fit + blurred background")),
                        ("split", "Split screen"),
                    ],
                );
                let r = ui.checkbox(&mut d.options.captions, l("Пословные субтитры", "Word-by-word captions"));
                app.auto.add_response("shorts.captions", &r, "Word captions");
                if d.options.captions {
                    choice(
                        app,
                        ui,
                        "shorts.captionPreset",
                        &mut d.options.caption_preset,
                        &[
                            ("bold", l("Крупные", "Bold")),
                            ("minimal", l("Минимализм", "Minimal")),
                            ("boxed", l("Подложка", "Boxed")),
                            ("accent", l("Акцент", "Accent")),
                        ],
                    );
                    let r = ui.text_edit_singleline(&mut d.options.font);
                    app.auto.add_response("shorts.font", &r, "Caption font");
                    choice(
                        app,
                        ui,
                        "shorts.position",
                        &mut d.options.position,
                        &[("top", l("Сверху", "Top")), ("center", l("По центру", "Center")), ("bottom", l("Снизу", "Bottom"))],
                    );
                    choice(
                        app,
                        ui,
                        "shorts.animation",
                        &mut d.options.animation,
                        &[("none", l("Без анимации", "No animation")), ("pop", "Pop"), ("fade", "Fade")],
                    );
                }
                let r = ui.add(egui::Slider::new(&mut d.options.versions, 1..=5).text(l("Число версий", "Versions")));
                app.auto.add_response("shorts.versions", &r, "Versions");
                ui.small(l(
                    "Версии меняют оформление субтитров, цвет, скорость и описание. Это не гарантия уникальности для платформ.",
                    "Variants change captions, color, speed and description. This does not guarantee platform uniqueness.",
                ));
            });
            app.auto.add_response("shorts.options", &options_header.header_response, "Layout and variants");
            if !proposals.is_empty() {
                ui.separator();
                ui.label(l(
                    "Оценка 0–100 — интерес и связность, не вероятность вирусности. Просмотри перед созданием.",
                    "Score 0–100 reflects editorial interest and coherence, not viral probability. Preview before creating.",
                ));
                if !valid {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        l("Источник или длительность изменились — повтори анализ", "Source or duration changed — analyze again"),
                    );
                }
                for (i, p) in proposals.iter().enumerate() {
                    ui.group(|ui| {
                        let mut chosen = d.selected.contains(&i);
                        let r = ui.checkbox(&mut chosen, format!("{} · {}/100", p["title"].as_str().unwrap_or(""), p["score"]));
                        app.auto.add_response(&format!("shorts.select.{i}"), &r, "Select fragment");
                        if chosen {
                            d.selected.insert(i);
                        } else {
                            d.selected.remove(&i);
                        }
                        ui.label(format!("{:.1}–{:.1} s", p["startMs"].as_f64().unwrap_or(0.) / 1000., p["endMs"].as_f64().unwrap_or(0.) / 1000.));
                        ui.small(p["reason"].as_str().unwrap_or(""));
                        ui.horizontal_wrapped(|ui| {
                            if button(app, ui, &format!("shorts.preview.{i}"), l("Смотреть", "Preview"), valid) {
                                preview = Some(i);
                            }
                            if button(app, ui, &format!("shorts.variants.{i}"), l("Создать версии", "Create variants"), valid) {
                                create = Some(vec![i]);
                            }
                        });
                    });
                }
                if button(app, ui, "shorts.create", l("Создать выбранные", "Create selected"), valid && !d.selected.is_empty()) {
                    create = Some(d.selected.iter().copied().collect());
                }
            }
            if !d.created.is_empty() {
                ui.separator();
                ui.heading(l("Созданные шортсы", "Created shorts"));
                for id in d.created.clone() {
                    if let Some(name) = app.session.project.item(id).map(|i| i.name.clone()) {
                        let mut on = d.export_selected.contains(&id);
                        let r = ui.checkbox(&mut on, &name);
                        app.auto.add_response(&format!("shorts.exportSelect.{}", id.0), &r, &name);
                        if on {
                            d.export_selected.insert(id);
                        } else {
                            d.export_selected.remove(&id);
                        }
                        let match_index = app.session.project.item(id).and_then(|item| {
                            let start = item.metadata.get("ShortStartMs")?.parse::<i64>().ok()?;
                            let end = item.metadata.get("ShortEndMs")?.parse::<i64>().ok()?;
                            if item.metadata.get("ShortSource")?.parse::<u64>().ok() != status["source"].as_u64() {
                                return None;
                            }
                            proposals.iter().position(|p| p["startMs"].as_i64() == Some(start) && p["endMs"].as_i64() == Some(end))
                        });
                        if button(app, ui, &format!("shorts.moreVariants.{}", id.0), l("Создать вариации", "Create variants"), valid && match_index.is_some())
                        {
                            create = match_index.map(|i| vec![i]);
                        }
                        if button(app, ui, &format!("shorts.open.{}", id.0), l("Открыть секвенцию", "Open sequence"), true) {
                            let _ = app.session.execute("sequence.open", json!({"item":id}));
                        }
                    }
                }
                let r = ui.text_edit_singleline(&mut d.folder);
                app.auto.add_response("shorts.folder", &r, "Export folder");
                folder = button(app, ui, "shorts.chooseFolder", l("Выбрать папку", "Choose folder"), true);
                export = button(app, ui, "shorts.export", l("Пакетный экспорт MP4", "Batch export MP4"), !d.export_selected.is_empty() && !d.folder.is_empty());
                for job in app.session.export_queue.items.iter().filter(|j| d.created.contains(&j.sequence)).rev().take(40) {
                    let label = match job.status {
                        filmcraft_engine::export_tools::QueueStatus::Ready => l("В очереди", "Queued"),
                        filmcraft_engine::export_tools::QueueStatus::Encoding => l("Экспорт", "Encoding"),
                        filmcraft_engine::export_tools::QueueStatus::Done => l("Готово", "Done"),
                        filmcraft_engine::export_tools::QueueStatus::Failed => l("Ошибка", "Failed"),
                        filmcraft_engine::export_tools::QueueStatus::Cancelled => l("Отменено", "Cancelled"),
                    };
                    ui.small(format!("{label} · {}", job.sequence_name));
                    if let Some(p) = job.job.and_then(|id| app.session.jobs.iter().find(|j| j.id == id)) {
                        ui.add(egui::ProgressBar::new(p.progress.fraction()).show_percentage());
                    }
                    if let Some(error) = &job.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                }
            }
            if !d.message.is_empty() {
                ui.label(&d.message);
            }
        });
    });
    if analyze {
        d.selected.clear();
        match app.session.execute("shorts.analyze", json!({"sequence":d.source,"duration":d.duration})) {
            Ok(_) => d.message.clear(),
            Err(e) => d.message = e.to_string(),
        }
    }
    if let Some(indices) = create {
        match app.session.execute("shorts.create", json!({"selected":indices,"options":d.options})) {
            Ok(v) => {
                for id in v["sequences"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(ItemId) {
                    d.created.push(id);
                    d.export_selected.insert(id);
                }
                d.message = l("Секвенции созданы в Shorts. Отмена — Cmd/Ctrl+Z.", "Created in Shorts. Undo with Cmd/Ctrl+Z.").into();
            }
            Err(e) => d.message = e.to_string(),
        }
    }
    if let Some(i) = preview
        && let Some(p) = proposals.get(i)
    {
        app.stop();
        let result = app
            .session
            .execute("sequence.open", json!({"item":status["source"]}))
            .and_then(|_| app.session.execute("playhead.set", json!({"seconds":p["startMs"].as_f64().unwrap_or(0.)/1000.})));
        match result {
            Ok(_) => {
                d.preview_source = app.session.state.active_sequence;
                d.preview_end = Some(Tick(p["endMs"].as_i64().unwrap_or(0) * filmcraft_time::TICKS_PER_SECOND / 1000));
                app.play(1.);
            }
            Err(e) => d.message = e.to_string(),
        }
    }
    if export {
        match app.session.execute("shorts.export", json!({"sequences":d.export_selected.iter().collect::<Vec<_>>(),"folder":d.folder})) {
            Ok(_) => d.message = l("Экспорт запущен. Прогресс — в очереди экспорта.", "Export started. See the export queue for progress.").into(),
            Err(e) => d.message = e.to_string(),
        }
    }
    app.shorts = d;
    if folder {
        app.pick_ui(crate::native_dialogs::FileDialog::Folder { at: None }, |app, paths| {
            if let Some(path) = paths.into_iter().next() {
                app.shorts.folder = path;
            }
        });
    }
}
