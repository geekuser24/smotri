//! Bounded MyInstants API transport; no audio assets or credentials are bundled.
use filmcraft_ui_egui::soundpad::{Event, Host, Sound};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
const SERVICE: &str = "Smotri.SoundPad.MyInstants";
pub struct NativeSoundPad {
    key: String,
    search: Option<Receiver<Event>>,
    download: Option<Receiver<Event>>,
    preview: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    last: Option<Instant>,
}
impl NativeSoundPad {
    pub fn new() -> Self {
        let mut key = std::env::var("MYINSTANTS_API_TOKEN").unwrap_or_default();
        #[cfg(target_os = "macos")]
        if key.is_empty() && std::env::var_os("SMOTRI_NO_SAVED_CREDENTIALS").is_none() {
            key = security_framework::passwords::get_generic_password(SERVICE, "api-key").ok().and_then(|v| String::from_utf8(v).ok()).unwrap_or_default();
        }
        Self { key, search: None, download: None, preview: None, cancel: Arc::new(AtomicBool::new(false)), last: None }
    }
}
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
                .build(),
        )
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}
fn error(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(429) => "MyInstants: rate limit reached. Try again in a minute.".into(),
        ureq::Error::StatusCode(c) => format!("MyInstants: HTTP {c}"),
        _ => "MyInstants connection failed. Check your network.".into(),
    }
}
fn trusted(url: &str) -> bool {
    let Ok(uri) = url.parse::<tungstenite::http::Uri>() else { return false };
    url.len() <= 2048
        && uri.scheme_str() == Some("https")
        && uri.host() == Some("www.myinstants.com")
        && uri.port_u16().is_none()
        && uri.path().starts_with("/media/sounds/")
        && uri.path().ends_with(".mp3")
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn parse(bytes: &[u8]) -> Result<(Vec<Sound>, bool), String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| "Invalid MyInstants response")?;
    let rows = v["results"].as_array().ok_or("Missing MyInstants results")?;
    let results = rows
        .iter()
        .take(50)
        .filter_map(|r| {
            let id = r["slug"].as_str().filter(|s| valid_id(s))?;
            let url = r["sound"].as_str().filter(|s| trusted(s))?;
            let title = r["name"].as_str().unwrap_or("Sound").chars().take(200).collect();
            Some(Sound { id: id.into(), title, url: url.into() })
        })
        .collect();
    Ok((results, v["next"].is_string()))
}
fn audio(sound: &Sound) -> Result<Vec<u8>, String> {
    if !trusted(&sound.url) || !valid_id(&sound.id) {
        return Err("Invalid MyInstants sound URL".into());
    }
    let mut response = agent().get(&sound.url).call().map_err(error)?;
    let bytes = response.body_mut().with_config().limit(20 * 1024 * 1024).read_to_vec().map_err(|_| "Sound exceeds 20 MB or download interrupted")?;
    if !bytes.starts_with(b"ID3") && !(bytes.first() == Some(&255) && bytes.get(1).is_some_and(|b| b & 0xe0 == 0xe0)) {
        return Err("MyInstants did not return MP3 audio".into());
    }
    Ok(bytes)
}
fn worker(f: impl FnOnce() -> Event + Send + 'static) -> Result<Receiver<Event>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("SoundPad".into())
        .spawn(move || {
            // The receiver reports a disconnected worker as an actionable error if a decoder panics.
            if let Ok(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                let _ = tx.send(e);
            }
        })
        .map_err(|_| "Could not start SoundPad worker")?;
    Ok(rx)
}
fn poll(rx: &mut Option<Receiver<Event>>, kind: u8) -> Option<Event> {
    let r = rx.as_ref()?.try_recv();
    match r {
        Ok(e) => {
            *rx = None;
            Some(e)
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            *rx = None;
            let e = Err("SoundPad worker stopped unexpectedly".into());
            Some(match kind {
                0 => Event::Results(Err("SoundPad search stopped unexpectedly".into())),
                1 => Event::Downloaded(Err("SoundPad download stopped unexpectedly".into())),
                _ => Event::PreviewFinished(e),
            })
        }
    }
}
impl Host for NativeSoundPad {
    fn configured(&self) -> bool {
        !self.key.is_empty()
    }
    fn save_key(&mut self, key: &str) -> Result<(), String> {
        if !(16..=256).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("Invalid MyInstants token".into());
        }
        #[cfg(target_os = "macos")]
        security_framework::passwords::set_generic_password(SERVICE, "api-key", key.as_bytes()).map_err(|_| "Could not save MyInstants token to Keychain")?;
        self.key = key.into();
        Ok(())
    }
    fn search(&mut self, query: &str, page: u32) -> Result<(), String> {
        if self.search.is_some() {
            return Err("Search already running".into());
        }
        if query.trim().is_empty() || query.chars().count() > 100 || !(1..=1000).contains(&page) {
            return Err("Invalid search parameters".into());
        }
        if self.last.is_some_and(|t| t.elapsed() < Duration::from_secs(7)) {
            return Err("MyInstants: wait 7 seconds between searches.".into());
        }
        let query = query.to_owned();
        let key = self.key.clone();
        self.search = Some(worker(move || {
            Event::Results((|| {
                let agent = agent();
                let mut req =
                    agent.get("https://www.myinstants.com/api/v1/instants/").query("name", query).query("page", page.to_string()).query("format", "json");
                if !key.is_empty() {
                    req = req.header("Authorization", format!("Token {key}"));
                }
                let mut res = req.call().map_err(error)?;
                let bytes = res.body_mut().with_config().limit(1024 * 1024).read_to_vec().map_err(|_| "MyInstants response too large")?;
                parse(&bytes)
            })())
        })?);
        self.last = Some(Instant::now());
        Ok(())
    }
    fn download(&mut self, sound: &Sound, folder: PathBuf) -> Result<(), String> {
        if self.download.is_some() {
            return Err("Download already running".into());
        }
        if !trusted(&sound.url) || !valid_id(&sound.id) {
            return Err("Invalid sound".into());
        }
        let sound = sound.clone();
        self.download = Some(worker(move || {
            Event::Downloaded((|| {
                let bytes = audio(&sound)?;
                std::fs::create_dir_all(&folder).map_err(|_| "Cannot create SoundPad folder")?;
                let path = folder.join(format!("MyInstants-{}-{}.mp3", sound.id, chrono::Utc::now().timestamp_millis()));
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| "Cannot create sound file")?;
                if file.write_all(&bytes).and_then(|_| file.sync_all()).is_err() {
                    let _ = std::fs::remove_file(&path);
                    return Err("Cannot save sound file".into());
                }
                Ok(path)
            })())
        })?);
        Ok(())
    }
    fn preview(&mut self, sound: &Sound) -> Result<(), String> {
        // Only one bounded preview worker; stopping cancels playback but keeps the receiver until it exits.
        if self.preview.is_some() {
            self.stop();
            return Err("Preview stopping. Press Listen again in a moment.".into());
        }
        if !trusted(&sound.url) || !valid_id(&sound.id) {
            return Err("Invalid sound".into());
        }
        let sound = sound.clone();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.preview = Some(worker(move || {
            Event::PreviewFinished((|| {
                let bytes = audio(&sound)?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                let pool = filmcraft_engine::media_pool::MediaPool::default();
                let source = pool.open_bytes("preview.mp3", bytes.into()).map_err(|e| format!("Cannot decode sound: {e}"))?;
                let frames = (source.info().duration.0 as i128 * 24000 / filmcraft_time::TICKS_PER_SECOND as i128).clamp(0, 24000 * 60) as usize;
                if frames == 0 {
                    return Err("Sound contains no audio".into());
                }
                let buffer = source.audio(0, frames, 24000).map_err(|e| format!("Cannot decode sound: {e}"))?;
                if buffer.channels.is_empty() || buffer.channels.len() > 32 {
                    return Err("Invalid audio channels".into());
                }
                let mut mono = vec![0.; frames];
                let gain = 1.0 / buffer.channels.len() as f32;
                for ch in &buffer.channels {
                    for (out, v) in mono.iter_mut().zip(ch) {
                        if !v.is_finite() {
                            return Err("Invalid audio sample".into());
                        }
                        *out += v * gain;
                    }
                }
                crate::jarvis_reply::play(mono, &cancel)
            })())
        })?);
        Ok(())
    }
    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
    fn poll(&mut self) -> Option<Event> {
        poll(&mut self.search, 0).or_else(|| poll(&mut self.download, 1)).or_else(|| poll(&mut self.preview, 2))
    }
}
impl Drop for NativeSoundPad {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_hostile_inputs() {
        for u in [
            "http://www.myinstants.com/media/sounds/x.mp3",
            "https://www.myinstants.com.evil.test/media/sounds/x.mp3",
            "https://127.0.0.1/media/sounds/x.mp3",
            "https://www.myinstants.com:443/media/sounds/x.mp3",
            "https://www.myinstants.com/media/sounds/x.html",
        ] {
            assert!(!trusted(u));
        }
        assert!(trusted("https://www.myinstants.com/media/sounds/x.mp3"));
        for bytes in [b"null".as_slice(), b"{}", b"{", b"{\"results\":42}"] {
            assert!(std::panic::catch_unwind(|| parse(bytes)).unwrap().is_err());
        }
        assert!(parse(br#"{"results":[{"slug":"../bad","sound":"https://www.myinstants.com/media/sounds/x.mp3"}]}"#).unwrap().0.is_empty());
    }
    #[test]
    #[ignore = "Opt-in live MyInstants network verification"]
    fn live_search_and_decode() {
        let mut r = agent().get("https://www.myinstants.com/api/v1/instants/").query("name", "notification").query("format", "json").call().unwrap();
        let bytes = r.body_mut().with_config().limit(1024 * 1024).read_to_vec().unwrap();
        let (rows, _) = parse(&bytes).unwrap();
        assert!(!rows.is_empty());
        let sound = &rows[0];
        let bytes = audio(sound).unwrap();
        let source = filmcraft_engine::media_pool::MediaPool::default().open_bytes("preview.mp3", bytes.into()).unwrap();
        assert!(source.info().has_audio());
        assert!(source.audio(0, 2400, 24000).unwrap().frames() > 0);
    }
}
