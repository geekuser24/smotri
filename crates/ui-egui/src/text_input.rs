//! Native menu key equivalents bypass egui's keyboard handling on macOS.
//! Deliver text editing actions to the focused field instead of the timeline.

use egui::{Event, Key, Modifiers};

/// Also accept the user's Control editing chords on macOS, where the usual accelerator
/// is Command. Only a text editor owns these aliases; timeline bindings remain untouched.
pub(crate) fn control_aliases(app: &mut crate::FilmcraftApp, ctx: &egui::Context, input: &mut egui::RawInput) {
    if !ctx.text_edit_focused() {
        return;
    }
    for event in &mut input.events {
        let Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
        if !modifiers.ctrl || modifiers.mac_cmd || modifiers.command || modifiers.alt {
            continue;
        }
        let replacement = match *key {
            Key::A | Key::Z => {
                modifiers.ctrl = false;
                modifiers.command = true;
                modifiers.mac_cmd = true;
                None
            }
            Key::C if !modifiers.shift => Some(Event::Copy),
            Key::X if !modifiers.shift => Some(Event::Cut),
            Key::V if !modifiers.shift => app.hooks.clipboard_text.as_mut().and_then(|f| f()).map(Event::Paste),
            _ => None,
        };
        if let Some(replacement) = replacement {
            *event = replacement;
        }
    }
}

pub(crate) fn editing_command(id: &str) -> bool {
    matches!(id, "edit.selectAll" | "edit.deselectAll" | "edit.copy" | "edit.cut" | "edit.paste" | "edit.clear" | "edit.delete" | "edit.undo" | "edit.redo")
}

pub(crate) fn route(app: &mut crate::FilmcraftApp, ctx: &egui::Context, id: &str) -> bool {
    if !ctx.text_edit_focused() || !id.starts_with("edit.") {
        return false;
    }
    let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let event = match id {
        "edit.selectAll" => Some(key(Key::A, Modifiers::COMMAND)),
        "edit.deselectAll" => Some(key(Key::ArrowRight, Modifiers::NONE)),
        "edit.copy" => Some(Event::Copy),
        "edit.cut" => Some(Event::Cut),
        "edit.paste" => {
            app.v_down = true;
            app.hooks.clipboard_text.as_mut().and_then(|f| f()).map(Event::Paste)
        }
        "edit.clear" | "edit.delete" => Some(key(Key::Delete, Modifiers::NONE)),
        "edit.undo" => Some(key(Key::Z, Modifiers::COMMAND)),
        "edit.redo" => Some(key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)),
        // Ripple Delete, Paste Attributes, etc. must not mutate the timeline while typing.
        _ => None,
    };
    if let Some(event) = event {
        ctx.input_mut(|i| i.events.push(event));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilmcraftApp;
    use filmcraft_engine::Session;
    use serde_json::json;

    #[test]
    fn control_aliases_apply_only_inside_text_fields() {
        let mut app = FilmcraftApp::new(Session::default());
        app.hooks.clipboard_text = Some(Box::new(|| Some("Текст".into())));
        let ctx = egui::Context::default();
        let event = |key| Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::CTRL };
        let mut input = egui::RawInput { events: vec![event(Key::A)], ..Default::default() };
        control_aliases(&mut app, &ctx, &mut input);
        assert_eq!(input.events, vec![event(Key::A)]);
        let mut text = "Поиск".to_string();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            });
            output.textures_delta.clear();
        }
        let mut input = egui::RawInput { events: vec![event(Key::A), event(Key::C), event(Key::X), event(Key::V), event(Key::Z)], ..Default::default() };
        control_aliases(&mut app, &ctx, &mut input);
        assert!(matches!(&input.events[0], Event::Key { modifiers, .. } if modifiers.command && !modifiers.ctrl));
        assert_eq!(input.events[1], Event::Copy);
        assert_eq!(input.events[2], Event::Cut);
        assert_eq!(input.events[3], Event::Paste("Текст".into()));
        assert!(matches!(&input.events[4], Event::Key { modifiers, .. } if modifiers.command && !modifiers.ctrl));
    }

    #[test]
    fn native_edit_actions_change_text_not_timeline() {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        let mut app = FilmcraftApp::new(session);
        let before = serde_json::to_value(&app.session.project).unwrap();
        let selected = app.session.state.selection.clone();
        let ctx = egui::Context::default();
        let id = egui::Id::new("search");
        let mut text = "Шаблоны camera".to_string();
        let mut frame = |app: &mut FilmcraftApp, action: Option<&str>| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                if let Some(action) = action {
                    assert!(route(app, ui.ctx(), action));
                }
                ui.add(egui::TextEdit::singleline(&mut text).id(id)).request_focus();
            });
            output.textures_delta.clear();
            output
        };
        frame(&mut app, None);
        frame(&mut app, None);
        frame(&mut app, Some("edit.selectAll"));
        let output = frame(&mut app, Some("edit.copy"));
        assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s == "Шаблоны camera")));
        frame(&mut app, Some("edit.cut"));
        app.hooks.clipboard_text = Some(Box::new(|| Some("Новый поиск".into())));
        frame(&mut app, Some("edit.paste"));
        frame(&mut app, Some("edit.rippleDelete"));
        assert_eq!(text, "Новый поиск");
        assert_eq!(serde_json::to_value(&app.session.project).unwrap(), before);
        assert_eq!(app.session.state.selection, selected);
    }
}
