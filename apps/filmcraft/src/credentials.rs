//! Shared service credentials, never part of projects, preferences or logs.
use std::sync::{Mutex, OnceLock};
static ELEVEN: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();
static OPENAI: OnceLock<Mutex<Option<Option<String>>>> = OnceLock::new();
fn cache(openai: bool) -> &'static Mutex<Option<Option<String>>> {
    if openai { OPENAI.get_or_init(Default::default) } else { ELEVEN.get_or_init(Default::default) }
}
#[cfg(target_os = "macos")]
fn service(openai: bool) -> &'static str {
    if openai { "Smotri.VerticalVideos.OpenAI" } else { "Smotri.Jarvis.ElevenLabs" }
}
pub fn load(openai: bool) -> Option<String> {
    let mut guard = cache(openai).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(k) = &*guard {
        return k.clone();
    }
    #[cfg(target_os = "macos")]
    let key = {
        let mut key = None;
        if std::env::var_os("SMOTRI_NO_SAVED_CREDENTIALS").is_none() {
            key = security_framework::passwords::get_generic_password(service(openai), "api-key").ok().and_then(|v| String::from_utf8(v).ok());
            if key.is_none() && !openai {
                key =
                    security_framework::passwords::get_generic_password("FilmCraft.Jarvis.ElevenLabs", "api-key").ok().and_then(|v| String::from_utf8(v).ok());
            }
        }
        key
    };
    #[cfg(not(target_os = "macos"))]
    let key: Option<String> = None;
    let key = key.or_else(|| std::env::var(if openai { "OPENAI_API_KEY" } else { "ELEVENLABS_API_KEY" }).ok()).filter(|v| !v.trim().is_empty());
    *guard = Some(key.clone());
    key
}
pub fn save(openai: bool, value: &str) -> Result<(), String> {
    let value = value.trim();
    if !(20..=512).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("Invalid API key".into());
    }
    #[cfg(target_os = "macos")]
    security_framework::passwords::set_generic_password(service(openai), "api-key", value.as_bytes()).map_err(|_| "Could not save API key in Keychain")?;
    *cache(openai).lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(value.into()));
    Ok(())
}
