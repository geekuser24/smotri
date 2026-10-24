//! Clarification turns stay open until answered/cancelled, bound to the exact editor context.
use super::*;
use filmcraft_project::{ClipId, ItemId, Project};
use filmcraft_time::Tick;
use serde_json::json;
use std::sync::Arc;

pub(super) struct Pending {
    pub prompt: String,
    project: Arc<Project>,
    sequence: Option<ItemId>,
    selection: Vec<ClipId>,
    playhead: Tick,
    choices: Vec<Intent>,
}
// The spoken question contains neither wake word, choice names nor numbers.
// Remove its exact normalized echo, also when Scribe merges it with the user's reply.
// This is transcript filtering, not acoustic echo cancellation.
pub(super) fn without_question_echo(text: &str) -> String {
    let mut words: Vec<_> = text.split_whitespace().collect();
    for prompt in ["какую тряску добавить", "which shake would you like"] {
        let n = prompt.split_whitespace().count();
        let mut i = 0;
        while i + n <= words.len() {
            if normalize(&words[i..i + n].join(" ")) == prompt {
                words.drain(i..i + n);
            } else {
                i += 1;
            }
        }
    }
    words.join(" ")
}
fn choice(text: &str) -> Option<usize> {
    let n = normalize(text);
    let mut n = n.as_str();
    for _ in 0..4 {
        let next = ["давай ", "выбери ", "добавь ", "поставь ", "хочу ", "вариант ", "номер ", "option ", "number ", "choose ", "add ", "the "]
            .iter()
            .find_map(|prefix| n.strip_prefix(prefix));
        match next {
            Some(rest) => n = rest,
            None => break,
        }
    }
    let n = n.trim_end_matches(" пожалуйста").trim_end_matches(" please");
    match n {
        "1" | "e1" | "е1" | "e 1" | "е 1" | "один" | "первый" | "one" | "first" | "ударный" | "ударный шейк" | "hit" | "hit shake" => {
            Some(0)
        }
        "2" | "e2" | "е2" | "e 2" | "е 2" | "два" | "второй" | "two" | "second" | "edt" | "эдт" | "эдит" | "со вспышкой" | "edt со вспышкой" => {
            Some(1)
        }
        "3" | "e3" | "е3" | "e 3" | "е 3" | "три" | "третий" | "three" | "third" | "живой" | "живая" | "с рук" | "handheld" | "тряска камеры" => {
            Some(2)
        }
        _ => None,
    }
}
/// `Some(None)` consumed an unaddressed reply; `None` lets a new addressed command through.
pub(super) fn reply(j: &mut Jarvis, text: &str, _at: Instant) -> Option<Option<Intent>> {
    let p = j.pending.as_ref()?;
    let addressed = super::addressed(text);
    let phrase = addressed.as_deref().unwrap_or(text);
    if matches!(normalize(phrase).as_str(), "отмена" | "не надо" | "отмени выбор" | "cancel" | "never mind" | "стоп" | "stop") {
        j.pending = None;
        j.cancel_speech();
        j.feedback = i18n::t("Choice cancelled").into();
        return Some(None);
    }
    if let Some(i) = choice(phrase) {
        return Some(Some(Intent { id: "jarvis.choose", params: json!({"choice":i}) }));
    }
    if matches!(normalize(phrase).as_str(), "повтори" | "повтори вопрос" | "repeat" | "repeat question" | "выбери шейк" | "choose shake")
    {
        let params = p
            .choices
            .first()
            .map(|c| {
                let mut params = c.params.clone();
                if let Some(obj) = params.as_object_mut() {
                    obj.remove("preset");
                }
                params
            })
            .unwrap_or_else(|| json!({}));
        return Some(Some(Intent { id: "jarvis.askShake", params }));
    }
    if addressed.is_some() {
        j.pending = None;
        return None;
    }
    j.feedback = p.prompt.clone();
    Some(None)
}
pub(super) fn ask_shake(app: &mut FilmcraftApp, intent: &Intent) -> Result<serde_json::Value, String> {
    if !app.jarvis.active {
        return Err("Turn on Jarvis before asking for a spoken choice".into());
    }
    // Check the context before asking; a nonexistent clip is not a choice between effects.
    filmcraft_engine::voice_edit::targets(&app.session, &intent.params, true).map_err(|e| e.to_string())?;
    let russian = i18n::current() == i18n::Language::Ru;
    let prompt = if russian {
        "Какую тряску добавить? 1 — ударный шейк; 2 — EDT со вспышкой; 3 — живая камера с рук. Назови номер или название."
    } else {
        "Which shake? 1: hit shake. 2: EDT with flash. 3: handheld camera. Say the number or name."
    };
    let choices = ["hit", "edt", "handheld"]
        .into_iter()
        .map(|preset| {
            let mut params = intent.params.clone();
            params["preset"] = json!(preset);
            Intent { id: "voice.shake", params }
        })
        .collect();
    app.jarvis.pending = Some(Pending {
        prompt: prompt.into(),
        project: app.session.project.clone(),
        sequence: app.session.state.active_sequence,
        selection: app.session.state.selection.clone(),
        playhead: app.session.playhead(),
        choices,
    });
    app.jarvis.feedback = prompt.into();
    app.jarvis.armed = None;
    let question = if russian { "Какую тряску добавить?" } else { "Which shake would you like?" };
    let spoken = app.jarvis.host.as_mut().ok_or_else(|| "Speech replies require the desktop app".to_string()).and_then(|h| h.speak(question));
    match spoken {
        Ok(()) => {
            app.jarvis.speaking = true;
        }
        Err(e) => {
            app.jarvis.feedback = format!("{prompt} ({e})");
        }
    }
    Ok(serde_json::Value::Null)
}
pub(super) fn apply_choice(app: &mut FilmcraftApp, ctx: &egui::Context, intent: &Intent) -> Result<serde_json::Value, String> {
    let p = app.jarvis.pending.take().ok_or_else(|| "No question is waiting for an answer".to_string())?;
    if !Arc::ptr_eq(&p.project, &app.session.project)
        || p.sequence != app.session.state.active_sequence
        || p.selection != app.session.state.selection
        || p.playhead != app.session.playhead()
    {
        return Err(i18n::t("The clip or playhead changed. Ask Jarvis again.").into());
    }
    let i = intent.params.get("choice").and_then(|v| v.as_u64()).and_then(|n| usize::try_from(n).ok()).ok_or("Invalid choice")?;
    let chosen = p.choices.get(i).ok_or("Invalid choice")?;
    super::execute(app, ctx, chosen)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pending() -> Pending {
        Pending { prompt: "choose".into(), project: Arc::new(Project::new("test")), sequence: None, selection: vec![], playhead: Tick::ZERO, choices: vec![] }
    }
    struct DialogueHost(std::collections::VecDeque<Event>);
    impl VoiceHost for DialogueHost {
        fn has_key(&self) -> bool {
            true
        }
        fn save_key(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn stop(&mut self) {}
        fn poll(&mut self) -> Option<Event> {
            self.0.pop_front()
        }
        fn speak(&mut self, text: &str) -> Result<(), String> {
            assert!(matches!(text, "Какую тряску добавить?" | "Which shake would you like?"));
            Ok(())
        }
    }
    #[test]
    fn barge_in_ignores_echo_and_applies_reply_before_speech_finishes() {
        for answer in ["E1", "Какую тряску добавить? Давай второй", "Which shake would you like? three"] {
            let mut session = filmcraft_engine::Session::default();
            session.execute("file.openDemoProject", json!({})).unwrap();
            let mut app = FilmcraftApp::new(session);
            let before = app.session.project.clone();
            app.jarvis.active = true;
            app.jarvis.ready = true;
            app.jarvis.host = Some(Box::new(DialogueHost(std::collections::VecDeque::from([
                Event::Transcript("Какую тряску добавить?".into(), Instant::now()),
                Event::Transcript(answer.into(), Instant::now()),
                Event::SpeechFinished(Ok(())),
            ]))));
            ask_shake(&mut app, &Intent { id: "jarvis.askShake", params: json!({}) }).unwrap();
            assert!(app.jarvis.ready);
            assert!(app.jarvis.speaking);
            assert!(final_command(&mut app.jarvis, "Which shake would you like?", Instant::now()).is_none());
            assert!(app.jarvis.awaiting_choice());
            tick(&mut app, &egui::Context::default());
            assert!(app.jarvis.ready);
            assert!(!app.jarvis.awaiting_choice());
            assert!(!app.jarvis.speaking);
            assert_ne!(app.session.project, before);
            app.session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.project, before);
        }
    }
    #[test]
    fn jarvis_delayed_dialogue_answer_without_wake_applies_once() {
        for answer in ["E1", "давай второй", "с рук"] {
            let mut session = filmcraft_engine::Session::default();
            session.execute("file.openDemoProject", json!({})).unwrap();
            let mut app = FilmcraftApp::new(session);
            let before = app.session.project.clone();
            app.jarvis.active = true;
            app.jarvis.host = Some(Box::new(DialogueHost(std::collections::VecDeque::from([
                Event::SpeechFinished(Ok(())),
                Event::Ready,
                Event::Transcript(answer.into(), Instant::now() - Duration::from_secs(45)),
            ]))));
            let intent = super::super::parse("выбери шейк").unwrap();
            assert_eq!(intent.id, "jarvis.askShake");
            ask_shake(&mut app, &intent).unwrap();
            assert!(app.jarvis.speaking);
            assert!(app.jarvis.awaiting_choice());
            tick(&mut app, &egui::Context::default());
            assert!(!app.jarvis.speaking);
            assert!(!app.jarvis.awaiting_choice());
            assert_ne!(app.session.project, before, "{answer}");
            app.session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.project, before);
            assert!(final_command(&mut app.jarvis, answer, Instant::now()).is_none());
        }
    }
    #[test]
    fn choices_apply_once_and_cannot_target_a_changed_context() {
        let mut session = filmcraft_engine::Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let mut app = FilmcraftApp::new(session);
        app.jarvis.active = true;
        let ctx = egui::Context::default();
        let before = app.session.project.clone();
        ask_shake(&mut app, &Intent { id: "jarvis.askShake", params: json!({}) }).unwrap();
        assert_eq!(app.session.project, before);
        apply_choice(&mut app, &ctx, &Intent { id: "jarvis.choose", params: json!({"choice":1}) }).unwrap();
        assert_ne!(app.session.project, before);
        assert!(apply_choice(&mut app, &ctx, &Intent { id: "jarvis.choose", params: json!({"choice":1}) }).is_err());
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.project, before);
        ask_shake(&mut app, &Intent { id: "jarvis.askShake", params: json!({}) }).unwrap();
        app.session.execute("playhead.step", json!({"frames":1})).unwrap();
        assert!(apply_choice(&mut app, &ctx, &Intent { id: "jarvis.choose", params: json!({"choice":0}) }).is_err());
        assert_eq!(app.session.project, before);
    }
    #[test]
    fn bounded_replies_require_a_pending_question() {
        let mut j = Jarvis { active: true, ..Default::default() };
        assert!(reply(&mut j, "two", Instant::now()).is_none());
        for phrase in ["2", "два", "второй", "Jarvis, EDT", "option two", "Джарвис, со вспышкой", "давай второй", "выбери вариант два пожалуйста", "E2"]
        {
            j.pending = Some(pending());
            assert_eq!(reply(&mut j, phrase, Instant::now()).flatten().unwrap().params, json!({"choice":1}));
        }
        for phrase in ["E1", "е 1", "поставь первый", "хочу ударный шейк"] {
            j.pending = Some(pending());
            assert_eq!(reply(&mut j, phrase, Instant::now()).flatten().unwrap().params, json!({"choice":0}));
        }
        j.pending = Some(pending());
        assert!(reply(&mut j, "delete", Instant::now()).unwrap().is_none());
        assert!(j.pending.is_some());
        assert!(reply(&mut j, "отмена", Instant::now()).unwrap().is_none());
        assert!(j.pending.is_none());
        j.pending = Some(pending());
        assert!(reply(&mut j, "Jarvis, undo", Instant::now()).is_none());
        assert!(j.pending.is_none());
        j.pending = Some(pending());
        assert_eq!(reply(&mut j, "two", Instant::now() + Duration::from_secs(600)).flatten().unwrap().params, json!({"choice":1}));
        assert!(j.pending.is_some());
    }
}
