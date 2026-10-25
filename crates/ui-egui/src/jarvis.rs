//! Opt-in voice editing. Only final speech segments enter the bounded command grammar.
//! Credentials and microphone transport belong to the native host, never serialized UI state.
use crate::{FilmcraftApp, i18n, menus};
use std::time::Duration;
use web_time::Instant;

pub enum Event {
    Ready,
    SpeechFinished(Result<(), String>),
    Transcript(String, Instant),
    Error(String),
}
pub trait VoiceHost {
    fn has_key(&self) -> bool;
    fn save_key(&mut self, key: &str) -> Result<(), String>;
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self);
    fn poll(&mut self) -> Option<Event>;
    /// Cancel only the reply; microphone and Scribe session remain connected.
    fn cancel_speech(&mut self) {}
    fn speak(&mut self, _text: &str) -> Result<(), String> {
        Err("Speech replies unavailable".into())
    }
}
#[derive(Default)]
pub struct Jarvis {
    pub host: Option<Box<dyn VoiceHost>>,
    pub active: bool,
    pub ready: bool,
    key: String,
    command_search: String,
    pub feedback: String,
    armed: Option<Instant>,
    pending: Option<dialogue::Pending>,
    speaking: bool,
}
impl Jarvis {
    pub fn awaiting_choice(&self) -> bool {
        self.pending.is_some()
    }
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }
    fn cancel_speech(&mut self) {
        if let Some(host) = self.host.as_mut() {
            host.cancel_speech();
        }
        self.speaking = false;
    }
    pub fn stop(&mut self) {
        if let Some(host) = self.host.as_mut() {
            host.stop();
        }
        self.active = false;
        self.ready = false;
        self.armed = None;
        self.pending = None;
        self.speaking = false;
    }
    pub fn toggle(&mut self) {
        if self.active {
            self.stop();
            self.feedback = i18n::t("Microphone off").into();
            return;
        }
        let Some(host) = self.host.as_mut() else {
            self.feedback = i18n::t("Voice control requires the desktop app").into();
            return;
        };
        if !host.has_key() {
            self.feedback = i18n::t("Set the ElevenLabs API key in Settings → Jarvis").into();
            return;
        }
        match host.start() {
            Ok(()) => {
                self.active = true;
                self.ready = false;
                self.armed = None;
                self.feedback = i18n::t("Connecting to ElevenLabs…").into();
            }
            Err(e) => self.feedback = e,
        }
    }
}

mod dialogue;
mod grammar;
mod workflows;
#[cfg(test)]
use grammar::{COMMANDS, command};
use grammar::{Intent, parse};

fn normalize(text: &str) -> String {
    text.replace('%', " percent ")
        .chars()
        .take(512)
        .flat_map(char::to_lowercase)
        .map(|c| {
            if c == 'ё' {
                'е'
            } else if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
// One transcription typo in a long word, never fuzzy-match short commands like cut/copy.
fn near_phrase(a: &str, b: &str) -> bool {
    let a: Vec<_> = a.split_whitespace().collect();
    let b: Vec<_> = b.split_whitespace().collect();
    if a.len() != b.len() {
        return false;
    }
    let mut changes = 0;
    for (a, b) in a.iter().zip(&b) {
        if a == b {
            continue;
        }
        if a.chars().count() < 5 || b.chars().count() < 5 || !one_edit(a, b) {
            return false;
        }
        changes += 1;
    }
    changes == 1
}
fn one_edit(a: &str, b: &str) -> bool {
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        if x == y {
            i += 1;
            j += 1;
        } else {
            edits += 1;
            if edits > 1 {
                return false;
            }
            if a.len() >= b.len() {
                i += 1;
            }
            if b.len() >= a.len() {
                j += 1;
            }
        }
    }
    edits + a.len().saturating_sub(i) + b.len().saturating_sub(j) <= 1
}
fn addressed(text: &str) -> Option<String> {
    let n = text.to_lowercase();
    let mut rest = n.trim_start_matches(|c: char| !c.is_alphanumeric());
    for attempt in 0..2 {
        let end = rest.find(|c: char| !c.is_alphanumeric()).unwrap_or(rest.len());
        let first = rest.get(..end)?;
        let tail = rest.get(end..)?.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '−');
        if attempt == 0 && matches!(first, "hey" | "эй" | "окей" | "okay") {
            rest = tail;
            continue;
        }
        return matches!(first, "джарвис" | "джервис" | "жарвис" | "jarvis" | "jarves" | "джарвес" | "джавис" | "джарвиз").then(|| tail.to_string());
    }
    None
}
fn final_command(j: &mut Jarvis, text: &str, at: Instant) -> Option<Intent> {
    if !j.active || text.chars().count() > 512 {
        return None;
    }
    let clean = dialogue::without_question_echo(text);
    let text = clean.as_str();
    if text.is_empty() {
        return None;
    }
    if let Some(reply) = dialogue::reply(j, text, at) {
        return reply;
    }
    // Delayed choice replies are safe only because apply_choice validates the captured context.
    // Ordinary commands still expire, so an editor stall cannot replay old edits.
    if at.elapsed() > Duration::from_secs(3) {
        return None;
    }
    let phrase = if let Some(rest) = addressed(text) {
        j.armed = None;
        if rest.is_empty() {
            j.armed = Some(at);
            return None;
        }
        rest
    } else if j.armed.take().is_some_and(|t| at.saturating_duration_since(t) <= Duration::from_secs(4)) {
        text.to_string()
    } else {
        return None;
    };
    let result = parse(&phrase);
    j.feedback = format!(
        "{text} — {}",
        i18n::t(if result.is_some() {
            "Command recognized"
        } else {
            "I could not parse the whole request. Nothing changed. Specify the clip, action and numeric value."
        })
    );
    result
}
pub fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    if !app.session.prefs.jarvis.enabled && app.jarvis.active {
        app.jarvis.stop();
    }
    if app.jarvis.active {
        ctx.request_repaint_after(Duration::from_millis(50));
        for _ in 0..8 {
            let event = app.jarvis.host.as_mut().and_then(|h| h.poll());
            match event {
                Some(Event::Ready) => {
                    app.jarvis.ready = true;
                    if let Some(p) = app.jarvis.pending.as_mut() {
                        app.jarvis.feedback = p.prompt.clone();
                    } else {
                        app.jarvis.feedback = i18n::t("Listening for Jarvis…").into();
                    }
                }
                Some(Event::SpeechFinished(result)) => {
                    app.jarvis.speaking = false;
                    if let Err(e) = result {
                        app.status(format!("{} — {e}", app.jarvis.feedback));
                    }
                }
                Some(Event::Error(e)) => {
                    app.jarvis.stop();
                    app.jarvis.feedback = i18n::t(&e).to_string();
                    app.status(app.jarvis.feedback.clone());
                    break;
                }
                Some(Event::Transcript(text, at)) => {
                    // External sound previews are untrusted audio, not editing instructions.
                    // Keep stop available, but ignore other speech until the preview tail clears.
                    if (app.soundpad.previewing || app.soundpad.preview_tail.is_some_and(|t| t.elapsed() < Duration::from_secs(3)))
                        && !addressed(&text).and_then(|s| parse(&s)).is_some_and(|i| matches!(i.id, "soundpad.stop" | "jarvis.pause" | "jarvis.stop"))
                    {
                        continue;
                    }
                    let previous_feedback = app.jarvis.feedback.clone();
                    if let Some(intent) = final_command(&mut app.jarvis, &text, at) {
                        app.jarvis.cancel_speech();
                        let result = execute(app, ctx, &intent);
                        if let Err(e) = result {
                            app.jarvis.feedback = format!("{text} — {e}");
                        } else if intent.id != "jarvis.askShake" {
                            app.jarvis.feedback = format!("{text} — {}", i18n::t("Command executed"));
                        }
                    }
                    if app.jarvis.feedback != previous_feedback {
                        app.status(app.jarvis.feedback.clone());
                    }
                }
                None => break,
            }
        }
    }
    if app.ui.settings.is_none() {
        app.jarvis.clear_key_draft();
    }
}

fn execute(app: &mut FilmcraftApp, ctx: &egui::Context, intent: &Intent) -> Result<serde_json::Value, String> {
    if (app.native_dialog.is_some()
        || app.dialog.is_some()
        || app.ui.settings.is_some()
        || app.ui.clip_dialog.is_some()
        || app.ui.color_dialog.is_some()
        || app.ui.link_media.is_some()
        || app.ui.create_proxies.is_some()
        || app.ui.project_manager.is_some()
        || app.ui.sync_dialog.is_some()
        || app.ui.workspace_dialog.is_some()
        || app.ui.guide_dialog.is_some()
        || app.ui.text_props_dialog.is_some()
        || app.ui.edit_cameras.is_some())
        && !matches!(intent.id, "jarvis.stop" | "jarvis.pause" | "jarvis.cancel")
    {
        return Err(i18n::t("Close the dialog before using editor voice commands").into());
    }
    match intent.id {
        id if id.starts_with("jarvis.shorts")
            || matches!(
                id,
                "jarvis.importGif"
                    | "jarvis.cleanupSilence"
                    | "jarvis.cleanupDuplicates"
                    | "jarvis.keepTake"
                    | "jarvis.previewTake"
                    | "jarvis.soundNext"
                    | "jarvis.soundPrevious"
            ) =>
        {
            workflows::execute(app, id, &intent.params)
        }
        "edit.rippleDelete" => app.session.execute("voice.rippleDelete", intent.params.clone()).map_err(|e| e.to_string()),
        "jarvis.askShake" => dialogue::ask_shake(app, intent),
        "jarvis.choose" => dialogue::apply_choice(app, ctx, intent),
        "jarvis.cancel" => {
            if app.ui.settings.is_some() {
                app.ui.settings = None;
                app.jarvis.clear_key_draft();
                app.dialog = None;
            } else if app.ui.clip_dialog.is_some() {
                app.ui.clip_dialog = None;
            } else {
                return Err(i18n::t("Close this dialog with its Cancel button").into());
            }
            Ok(serde_json::Value::Null)
        }
        "jarvis.stop" => {
            app.jarvis.stop();
            Ok(serde_json::Value::Null)
        }
        "jarvis.play" => {
            if app.ui.focused == crate::dock::PanelKind::Source {
                app.play_source()?;
            } else {
                if app.session.active_sequence().is_none() {
                    return Err("no sequence is open".into());
                }
                app.play(1.0);
            }
            Ok(serde_json::Value::Null)
        }
        "jarvis.pause" => {
            app.stop();
            app.stop_source();
            let _ = crate::soundpad::command(app, "soundpad.stop", serde_json::json!({}));
            Ok(serde_json::Value::Null)
        }
        id if id.starts_with("window.panel.") => {
            let panel = id.strip_prefix("window.panel.").and_then(crate::dock::PanelKind::from_name).ok_or_else(|| "unknown panel".to_string())?;
            // A spoken “show” must reveal the panel, even when its menu item is checked.
            app.show_panel(panel);
            Ok(serde_json::Value::Null)
        }
        _ => menus::invoke(app, ctx, intent.id, intent.params.clone()),
    }
}

impl Jarvis {
    pub fn clear_key_draft(&mut self) {
        self.key.clear();
    }
    pub fn apply_key_draft(&mut self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Ok(());
        }
        if self.active {
            return Err(i18n::t("Turn Jarvis off before changing the API key").into());
        }
        self.host.as_mut().ok_or_else(|| i18n::t("Voice control requires the desktop app").to_string())?.save_key(self.key.trim())?;
        self.key.clear();
        self.feedback = i18n::t("API key saved").into();
        Ok(())
    }
}

/// Settings → Jarvis. OK saves to the native credential store; Cancel discards the draft.
pub fn settings_page(app: &mut FilmcraftApp, ui: &mut egui::Ui, values: &mut serde_json::Value) {
    ui.heading(i18n::t("Jarvis"));
    let mut enabled = values.pointer("/jarvis/enabled").and_then(serde_json::Value::as_bool).unwrap_or(true);
    ui.horizontal(|ui| {
        let (rect, mut response) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
        if response.clicked() {
            enabled = !enabled;
            if let Some(slot) = values.pointer_mut("/jarvis/enabled") {
                *slot = serde_json::json!(enabled);
            }
            response.mark_changed();
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, enabled, i18n::t("Enable Jarvis")));
        let amount = ui.ctx().animate_bool(response.id, enabled);
        ui.painter().rect_filled(rect, 11.0, if enabled { app.tokens.accent } else { app.tokens.field_bg });
        ui.painter().circle_filled(egui::pos2(rect.left() + 11.0 + amount * 18.0, rect.center().y), 8.0, app.tokens.text);
        app.auto.add("settings.jarvis.enabled", rect, "Enable Jarvis");
        ui.label(i18n::t("Enable Jarvis"));
    });
    ui.label(i18n::t("When disabled, the Jarvis button is hidden and voice control stops."));
    ui.add_space(12.0);
    ui.label("ElevenLabs · Scribe v2 Realtime");
    ui.add_space(12.0);
    shared_key_ui(app, ui);
    ui.add_space(12.0);
    ui.separator();
    ui.label(i18n::t("Use the Jarvis button in the header to turn voice control on or off."));
    ui.label(i18n::t("While enabled, microphone audio is sent to ElevenLabs. Russian and English are detected automatically."));
    ui.add_space(12.0);
    ui.label(i18n::t("Say Jarvis, then a command. Select clips before copy, cut or delete. Split cuts at the playhead."));
    ui.label(i18n::t("Voice examples: forward 10 frames; speed 50 percent; louder 3 db; lock track video 1."));
    ui.label(i18n::t("Menu commands also accept their English or Russian menu name. Dialogs still require manual input."));
    let r = ui.add(egui::TextEdit::singleline(&mut app.jarvis.command_search).hint_text(i18n::t("Search voice commands")));
    app.auto.add("jarvis.commandSearch", r.rect, "Search voice commands");
    let query = normalize(&app.jarvis.command_search);
    egui::ScrollArea::vertical().id_salt("jarvis.commands").max_height(200.0).show(ui, |ui| {
        for text in [
            "добавь эффект шейкер • включи хит режим • поставь ключ удара • add shaker hit",
            "пресет шейкера эдит • shaker preset handheld • выключи хит режим",
            "найди звук удар • search sounds impact • послушай звук второй • preview sound two",
            "добавь звук 2 на таймлайн • import sound 2 into project • останови звук",
            "найди гифку кот • find gif cat • добавь гифку 1",
            "проанализируй шортсы на 60 секунд • analyze shorts for 60 seconds",
            "выбери шортс второй • покажи шортс 1 • выбери все шортсы • создай шортсы",
            "шортсы с размытым фоном • шортсы сплит скрин • субтитры шортсов снизу",
            "сделай версии шортсов 3 • экспортируй шортсы (папку выбери в панели)",
            "проанализируй тишину • примени удаление тишины • проанализируй дубли",
            "выдели второй клип, потом уменьши его масштаб на 20 процентов, затем разрежь его пополам",
            "select second clip then reduce scale by twenty five percent then split it in half",
            "хочу на втором клипе сделать яркость поменьше на 20 процентов",
            "сделай эдит • edit • make an edit — выдели видео и музыку / select video and music",
            "выдели клип • select clip • выдели второй клип на видео 2 • select second clip on video 2",
            "выдели первый аудио клип на дорожке 2 • select first audio clip on track 2",
            "выдели клип по имени Sunset • select clip named Sunset",
            "перейди к следующему клипу • go to next clip • перейди к предыдущему клипу",
            "сдвинь таймлайн влево на 10 кадров • move playhead right by 10 frames",
            "добавь черно белый эффект • add black and white • добавь шейк • add shake",
            "добавь ударный шейк • add hit shake • ответ на вопрос: один / два / три • answer: one / two / three",
            "создай корректирующий слой • create adjustment layer",
            "добавь короткий шейк • добавь ударный шейк на сам клип",
            "добавь тряску камеры корректирующим слоем • add handheld shake as adjustment layer",
            "добавь тряску камеры • add handheld shake • сделай шейк как в EDIT • add EDIT shake",
            "смени метод наложения на screen • set blend mode to screen",
            "сделай ниже на 20 • сделай 100% • сделай 10% (масштаб / scale)",
            "разрежь второй клип по центру • split second clip in half",
            "сделай две секунды второго клипа • make second clip two seconds",
            "передвинь второй клип на первое место • move second clip to first position",
            "добавь маркер под названием удар • поставь маркер на 10 секунд",
            "понизь яркость на 20 процентов • decrease brightness by 20 percent",
            "уменьши scale на 20 процентов • decrease scale by 20 percent",
            "выбери эффект camera shake • select effect camera shake",
            "установи параметр frequency на 2 в эффекте shaker • set parameter frequency to 2 in effect shaker",
            "расшифруй клип • transcribe clip • расшифруй таймлайн • transcribe timeline",
            "удали паузы учитывая все слои • remove pauses all layers",
            "удали паузы в голосе в клипе • remove pauses in clip voice only",
            "удали паузы везде только голос • remove pauses everywhere voice only",
        ] {
            if query.is_empty() || normalize(text).contains(&query) {
                ui.label(text);
            }
        }
        ui.separator();
        for (id, aliases) in grammar::COMMANDS {
            let text = aliases.join(" • ");
            if query.is_empty() || normalize(&text).contains(&query) || id.to_lowercase().contains(&query) {
                ui.label(text);
            }
        }
        ui.separator();
        for entry in grammar::catalog() {
            let text = entry.phrases.join(" • ");
            if query.is_empty() || text.contains(&query) {
                ui.label(text);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_preference_stops_voice_and_blocks_activation() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        let ctx = egui::Context::default();
        app.jarvis.active = true;
        app.jarvis.ready = true;
        app.jarvis.speaking = true;
        app.session.execute("prefs.set", serde_json::json!({"key":"jarvis.enabled", "value":false})).unwrap();
        tick(&mut app, &ctx);
        assert!(!app.jarvis.active && !app.jarvis.ready && !app.jarvis.speaking);
        assert!(crate::menus::invoke(&mut app, &ctx, "jarvis.toggle", serde_json::json!({})).is_err());
        let saved = serde_json::to_string(&app.session.prefs).unwrap();
        let restored: filmcraft_engine::autosave::Preferences = serde_json::from_str(&saved).unwrap();
        assert!(!restored.jarvis.enabled);
        let old: filmcraft_engine::autosave::Preferences = serde_json::from_str("{}").unwrap();
        assert!(old.jarvis.enabled);
    }

    #[test]
    fn bilingual_commands_and_bounded_fuzzy_matching() {
        for (id, aliases) in COMMANDS {
            for alias in *aliases {
                assert_eq!(command(alias), Some(*id), "{alias}");
            }
        }
        for (text, id) in [
            ("нарежь", "sequence.addEdit"),
            ("please split this clip", "sequence.addEdit"),
            ("скопируй пожалуйста", "edit.copy"),
            ("удали со сдвигом", "edit.rippleDelete"),
            ("ripple delete", "edit.rippleDelete"),
            ("скоперуй", "edit.copy"),
            ("вырежи", "edit.cut"),
        ] {
            assert_eq!(command(text), Some(id), "{text}");
        }
        for text in [
            "не удаляй",
            "do not delete",
            "don't delete",
            "удали и скопируй",
            "copy and delete",
            "я хочу рассказать про удалить",
            "",
            "cute",
            "удали все",
            "delete project",
        ] {
            assert_eq!(command(text), None, "{text}");
        }
    }
    #[test]
    fn wake_word_is_required_and_expires() {
        let mut j = Jarvis { active: true, ..Default::default() };
        let now = Instant::now();
        assert_eq!(final_command(&mut j, "удали", now).map(|c| c.id), None);
        assert_eq!(final_command(&mut j, "Jarvis, copy", now).map(|c| c.id), Some("edit.copy"));
        assert_eq!(final_command(&mut j, "мы говорим про джарвис удали", now).map(|c| c.id), None);
        assert_eq!(final_command(&mut j, "Джарвис", now).map(|c| c.id), None);
        assert_eq!(final_command(&mut j, "удали со сдвигом", now).map(|c| c.id), Some("edit.rippleDelete"));
        j.armed = now.checked_sub(Duration::from_secs(5));
        assert_eq!(final_command(&mut j, "удали", now).map(|c| c.id), None);
        j.active = false;
        assert_eq!(final_command(&mut j, "Джарвис удали", now).map(|c| c.id), None);
    }
    struct FakeHost(std::collections::VecDeque<Event>);
    impl VoiceHost for FakeHost {
        fn has_key(&self) -> bool {
            true
        }
        fn save_key(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) {
            self.0.clear();
        }
        fn poll(&mut self) -> Option<Event> {
            self.0.pop_front()
        }
    }
    #[test]
    fn spoken_delete_and_undo_restore_the_project() {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", serde_json::json!({})).unwrap();
        let clip = session.active_sequence().unwrap().video_tracks[0].items[0].id;
        session.state.selection.push(clip);
        let before = session.project.clone();
        let mut app = FilmcraftApp::new(session);
        app.jarvis.active = true;
        app.jarvis.host = Some(Box::new(FakeHost([Event::Transcript("Джарвис удали".into(), Instant::now())].into())));
        let ctx = egui::Context::default();
        tick(&mut app, &ctx);
        assert!(app.session.active_sequence().unwrap().find_item(clip).is_none());
        app.jarvis.host = Some(Box::new(FakeHost([Event::Transcript("Jarvis undo".into(), Instant::now())].into())));
        tick(&mut app, &ctx);
        assert_eq!(*app.session.project, *before);
    }
    #[test]
    fn punctuation_does_not_drop_wake_words_or_numeric_signs() {
        for text in ["Jarvis:copy", "Джарвис,скопируй", "Hey, Jarvis, copy"] {
            let mut j = Jarvis { active: true, ..Default::default() };
            assert_eq!(final_command(&mut j, text, Instant::now()).map(|c| c.id), Some("edit.copy"), "{text}");
        }
        let mut j = Jarvis { active: true, ..Default::default() };
        assert!(final_command(&mut j, "Jarvis speed -50 percent", Instant::now()).is_none());
    }
    #[test]
    fn stopped_and_stale_events_cannot_edit() {
        let mut j = Jarvis { active: true, ..Default::default() };
        let stale = Instant::now().checked_sub(Duration::from_secs(10)).unwrap();
        assert_eq!(final_command(&mut j, "Jarvis delete", stale).map(|c| c.id), None);
        j.armed = Some(Instant::now());
        j.stop();
        assert!(j.armed.is_none());
        assert_eq!(final_command(&mut j, "Jarvis delete", Instant::now()).map(|c| c.id), None);
        assert_eq!(command(&format!("delete{}do not", " ".repeat(600))), None);
    }
    #[test]
    fn transcription_mutations_never_panic() {
        for text in ["Джарвис удали со сдвигом", "Jarvis, copy", "💥 Джарвис", "\0", "джарвис не удаляй"] {
            for (i, _) in text.char_indices() {
                let mut j = Jarvis { active: true, ..Default::default() };
                let _ = final_command(&mut j, &text[..i], Instant::now());
            }
        }
    }
    #[test]
    fn settings_key_draft_stays_out_of_serialized_preferences() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        crate::panels::settings::open(&mut app, "jarvis").unwrap();
        app.jarvis.key = "secret-draft-for-test".into();
        let serialized = serde_json::to_string(&app.ui.settings).unwrap();
        assert!(!serialized.contains("secret-draft-for-test"));
        assert_eq!(app.ui.settings.as_ref().unwrap().page, "jarvis");
        app.jarvis.clear_key_draft();
        assert!(app.jarvis.key.is_empty());
        assert!(app.jarvis.apply_key_draft().is_ok());
    }
}

#[cfg(test)]
mod workflow_tests {
    use super::*;
    use serde_json::json;
    fn demo() -> FilmcraftApp {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        session.execute("playhead.step", json!({"frames":10})).unwrap();
        session.execute("edit.selectAll", json!({})).unwrap();
        session.execute("edit.copy", json!({})).unwrap();
        let mut app = FilmcraftApp::new(session);
        app.ui.focused = crate::dock::PanelKind::Timeline;
        app
    }
    #[test]
    fn spoken_workflows_match_editor_commands() {
        let ctx = egui::Context::default();
        let cases = [
            ("выдели все", "edit.selectAll"),
            ("сними выделение", "edit.deselectAll"),
            ("выдели клип под курсором", "timeline.selectClipAtPlayhead"),
            ("select next clip", "timeline.selectNextClip"),
            ("select previous clip", "timeline.selectPrevClip"),
            ("скопируй", "edit.copy"),
            ("вырежи", "edit.cut"),
            ("удали со сдвигом", "edit.rippleDelete"),
            ("разрежь", "sequence.addEdit"),
            ("split all tracks", "sequence.addEditAllTracks"),
            ("поставь вход", "markers.markIn"),
            ("mark out", "markers.markOut"),
            ("отметь клип", "markers.markClip"),
            ("отметь выделение", "markers.markSelection"),
            ("добавь маркер", "markers.add"),
            ("clear in and out", "markers.clearInOut"),
            ("в начало", "playhead.start"),
            ("в конец", "playhead.end"),
            ("следующая склейка", "playhead.nextEdit"),
            ("предыдущая склейка", "playhead.prevEdit"),
            ("следующий кадр", "playhead.stepForward"),
            ("previous frame", "playhead.stepBack"),
            ("в начало клипа", "playhead.selectedClipStart"),
            ("в конец клипа", "playhead.selectedClipEnd"),
            ("сгруппируй", "clip.group"),
            ("разгруппируй", "clip.ungroup"),
            ("переключи привязку", "sequence.snap"),
            ("toggle linked selection", "sequence.linkedSelection"),
            ("приблизь", "view.zoomIn"),
            ("отдали", "view.zoomOut"),
            ("fit timeline", "view.zoomToSequence"),
            ("выбери лезвие", "tool.razor"),
            ("selection tool", "tool.selection"),
            ("type tool", "tool.type"),
            ("экспорт видео", "file.exportMedia"),
            ("настрой скорость", "clip.speedDuration"),
            ("добавь дорожки", "sequence.addTracks"),
            ("скорость и длительность", "clip.speedDuration"),
        ];
        for (phrase, id) in cases {
            let mut voice = demo();
            let mut direct = demo();
            voice.jarvis.active = true;
            let intent = final_command(&mut voice.jarvis, &format!("Джарвис, {phrase}"), Instant::now()).unwrap();
            assert_eq!(intent.id, id, "{phrase}");
            execute(&mut voice, &ctx, &intent).unwrap_or_else(|e| panic!("{phrase}: {e}"));
            menus::invoke(&mut direct, &ctx, id, json!({})).unwrap();
            assert_eq!(*voice.session.project, *direct.session.project, "{phrase}");
            assert_eq!(serde_json::to_value(&voice.session.state).unwrap(), serde_json::to_value(&direct.session.state).unwrap(), "{phrase}");
            assert_eq!(serde_json::to_value(&voice.ui).unwrap(), serde_json::to_value(&direct.ui).unwrap(), "{phrase}");
        }
    }
    #[test]
    fn spoken_show_keeps_an_existing_panel_open() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        let ctx = egui::Context::default();
        app.jarvis.active = true;
        for _ in 0..2 {
            let intent = final_command(&mut app.jarvis, "Jarvis, show project", Instant::now()).unwrap();
            execute(&mut app, &ctx, &intent).unwrap();
            assert!(app.ui.dock.contains(crate::dock::PanelKind::Project));
            assert_eq!(app.ui.focused, crate::dock::PanelKind::Project);
        }
    }

    #[test]
    fn spoken_ripple_delete_handles_sync_locked_overlays() {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let clip = session.active_sequence().unwrap().video_tracks[0].items[1].id;
        session.execute("timeline.select", json!({"clips":[clip.0]})).unwrap();
        let before = session.project.clone();
        let mut app = FilmcraftApp::new(session);
        app.jarvis.active = true;
        let ctx = egui::Context::default();
        let intent = final_command(&mut app.jarvis, "Джарвис, удалить со сдвигом", Instant::now()).unwrap();
        execute(&mut app, &ctx, &intent).unwrap();
        assert!(app.session.active_sequence().unwrap().find_item(clip).is_none());
        let undo = final_command(&mut app.jarvis, "Jarvis, undo", Instant::now()).unwrap();
        execute(&mut app, &ctx, &undo).unwrap();
        assert_eq!(app.session.project, before);
    }
    #[test]
    fn parameter_edits_apply_and_undo_restores_project() {
        let ctx = egui::Context::default();
        for phrase in ["speed 200 percent", "громче на 3 дб", "lock track video one", "mute track audio one"] {
            let mut app = demo();
            let before = app.session.project.clone();
            let intent = parse(phrase).unwrap();
            execute(&mut app, &ctx, &intent).unwrap_or_else(|e| panic!("{phrase}: {e}"));
            assert_ne!(*app.session.project, *before, "{phrase}");
            app.session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*app.session.project, *before, "{phrase}");
        }
        let mut app = demo();
        let rate = app.session.active_sequence().unwrap().settings.frame_rate;
        let before = rate.frame_at(app.session.playhead());
        execute(&mut app, &ctx, &parse("forward 10 frames").unwrap()).unwrap();
        assert_eq!(rate.frame_at(app.session.playhead()), before + 10);
    }
    #[test]
    fn dialogs_and_disabled_commands_cannot_change_the_timeline() {
        let ctx = egui::Context::default();
        let mut app = demo();
        let before = app.session.project.clone();
        execute(&mut app, &ctx, &parse("настрой скорость").unwrap()).unwrap();
        assert!(app.ui.clip_dialog.is_some());
        assert!(execute(&mut app, &ctx, &parse("удали").unwrap()).is_err());
        assert_eq!(*app.session.project, *before);
        execute(&mut app, &ctx, &parse("закрой диалог").unwrap()).unwrap();
        assert!(app.ui.clip_dialog.is_none());
        app.session.execute("edit.deselectAll", json!({})).unwrap();
        assert!(execute(&mut app, &ctx, &parse("удали").unwrap()).is_err());
        assert_eq!(*app.session.project, *before);
    }
    #[test]
    fn live_scribe_fixtures_when_requested() {
        let Ok(path) = std::env::var("FILMCRAFT_JARVIS_SPEECH_FIXTURES") else {
            return;
        };
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(rows.len() >= 25);
        for row in rows {
            let mut j = Jarvis { active: true, ..Default::default() };
            let text = row["transcript"].as_str().unwrap();
            assert_eq!(final_command(&mut j, text, Instant::now()).map(|c| c.id), row["expected"].as_str(), "{text}");
        }
    }
}

pub(crate) fn clarification_command(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let id = match id {
        "jarvis.askShake" => "jarvis.askShake",
        "jarvis.choose" => "jarvis.choose",
        _ => return Err("Unknown clarification command".into()),
    };
    let result = execute(app, ctx, &Intent { id, params });
    app.status(app.jarvis.feedback.clone());
    result
}

pub fn shared_key_ui(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    let configured = app.jarvis.host.as_ref().is_some_and(|h| h.has_key());
    ui.label(i18n::t(if configured { "API key configured" } else { "API key not configured" }));
    ui.label("ElevenLabs API key");
    let hint = i18n::t(if configured { "Enter a new key to replace the saved key" } else { "Enter your ElevenLabs API key" });
    let r = ui.add_enabled(!app.jarvis.active, egui::TextEdit::singleline(&mut app.jarvis.key).password(true).hint_text(hint).desired_width(420.0));
    app.auto.add("jarvis.apiKey", r.rect, "ElevenLabs API key");
    if app.jarvis.active {
        ui.label(i18n::t("Turn Jarvis off before changing the API key"));
    }
    ui.label(i18n::t("Click OK to save the key. Cancel discards changes."));
    ui.label(i18n::t("On macOS the key is stored in Keychain. Voice mode is always off at startup."));
}

#[cfg(test)]
mod soundpad_voice_tests {
    use super::*;
    use crate::soundpad::{Event as SoundEvent, Host, Sound};
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    struct HostSpy(Arc<Mutex<Vec<String>>>);
    impl Host for HostSpy {
        fn configured(&self) -> bool {
            false
        }
        fn save_key(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn search(&mut self, q: &str, _: u32) -> Result<(), String> {
            self.0.lock().unwrap().push(format!("search:{q}"));
            Ok(())
        }
        fn download(&mut self, s: &Sound, _: PathBuf) -> Result<(), String> {
            self.0.lock().unwrap().push(format!("import:{}", s.id));
            Ok(())
        }
        fn preview(&mut self, s: &Sound) -> Result<(), String> {
            self.0.lock().unwrap().push(format!("preview:{}", s.id));
            Ok(())
        }
        fn stop(&mut self) {
            self.0.lock().unwrap().push("stop".into());
        }
        fn poll(&mut self) -> Option<SoundEvent> {
            None
        }
    }
    #[test]
    fn spoken_sound_search_preview_import_routes_to_current_results() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.soundpad.host = Some(Box::new(HostSpy(log.clone())));
        let ctx = egui::Context::default();
        execute(&mut app, &ctx, &parse("можешь пожалуйста найти звук удар").unwrap()).unwrap();
        app.soundpad.results = vec![Sound { id: "a".into(), title: "A".into(), url: "https://www.myinstants.com/media/sounds/a.mp3".into() }];
        execute(&mut app, &ctx, &parse("прослушай звук первый").unwrap()).unwrap();
        execute(&mut app, &ctx, &parse("импортируй звук 1 в проект").unwrap()).unwrap();
        assert_eq!(*log.lock().unwrap(), vec!["search:удар", "preview:a", "import:a"]);
        let before = app.session.project.to_json();
        assert!(parse("не добавляй звук 1").is_none());
        assert!(parse("add sound 1 then delete everything").is_none());
        assert!(execute(&mut app, &ctx, &parse("добавь звук 50").unwrap()).is_err());
        assert_eq!(before, app.session.project.to_json());
    }
    #[test]
    fn shorts_voice_settings_and_opening_preserve_original_project() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.session.execute("file.openDemoProject", serde_json::json!({})).unwrap();
        let before = app.session.project.to_json();
        let ctx = egui::Context::default();
        for s in [
            "открой шортсы",
            "шортсы сплит скрин",
            "сделай версии шортсов 3",
            "субтитры шортсов сверху",
            "shorts captions fade",
            "shorts source sequence first",
        ] {
            execute(&mut app, &ctx, &parse(s).unwrap()).unwrap();
        }
        assert_eq!(app.shorts.options.layout, "split");
        assert_eq!(app.shorts.options.versions, 3);
        assert_eq!(app.shorts.options.position, "top");
        assert_eq!(app.shorts.options.animation, "fade");
        assert!(execute(&mut app, &ctx, &parse("создай шортсы").unwrap()).is_err());
        assert_eq!(before, app.session.project.to_json());
    }
}
