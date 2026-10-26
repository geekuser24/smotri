//! Installed audio effect catalog commands; analysis never edits the current project.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "audioPlugins.reset",
            label: "Restart Audio Plugins",
            menu: &[],
            shortcut: None,
            params: "{}",
            journal: false,
            enabled: |_| Ok(()),
            run: |_, _| {
                filmcraft_render::audio_fx::clear_cache();
                filmcraft_render::mixer::clear_cache();
                filmcraft_audio_plugin_client::clear_failures();
                Ok(Value::Null)
            },
        },
        CommandSpec {
            id: "audioPlugins.scan",
            label: "Scan Audio Plugins",
            menu: &[],
            shortcut: None,
            params: "{folders?: [absolute path]}",
            journal: false,
            enabled: |_| Ok(()),
            run: scan,
        },
        CommandSpec {
            id: "audioPlugins.list",
            label: "Installed Audio Plugins",
            menu: &[],
            shortcut: None,
            params: "{}",
            journal: false,
            enabled: |_| Ok(()),
            run: |_, _| serde_json::to_value(filmcraft_audio_plugin_client::catalog::status()).map_err(|e| EngineError::Other(e.to_string())),
        },
    ]
}
fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    let folders = if let Some(v) = p.get("folders") {
        serde_json::from_value::<Vec<std::path::PathBuf>>(v.clone()).map_err(|e| EngineError::Other(e.to_string()))?
    } else {
        s.prefs.plugins.audio_folders.lines().map(str::trim).filter(|p| !p.is_empty()).map(std::path::PathBuf::from).collect()
    };
    let path = s.prefs_path.as_ref().and_then(|p| p.parent()).map(|p| p.join("audio-plugins.json"));
    filmcraft_audio_plugin_client::catalog::start(folders, path).map_err(EngineError::Other)?;
    Ok(json!({"running":true}))
}
