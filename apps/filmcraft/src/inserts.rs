//! Bounded, asynchronous GIPHY transport; keys never enter URLs shown in errors or saved UI state.
use filmcraft_ui_egui::inserts::{Event, Gif, Host};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};
const SERVICE: &str = "FilmCraft.Inserts.GIPHY";
const ACCOUNT: &str = "api-key";
pub struct NativeInserts {
    key: String,
    search: Option<Receiver<Event>>,
    download: Option<Receiver<Event>>,
}
impl NativeInserts {
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        let key = if std::env::var_os("SMOTRI_NO_SAVED_CREDENTIALS").is_some() {
            None
        } else {
            security_framework::passwords::get_generic_password(SERVICE, ACCOUNT).ok()
        }
        .and_then(|v| String::from_utf8(v).ok())
        .unwrap_or_default();
        #[cfg(not(target_os = "macos"))]
        let key = std::env::var("GIPHY_API_KEY").unwrap_or_default();
        Self { key, search: None, download: None }
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
        ureq::Error::StatusCode(429) => "GIPHY: лимит запросов исчерпан. Повтори поиск позже.".into(),
        ureq::Error::StatusCode(401 | 403) => "GIPHY: доступ запрещён. Проверь API-ключ в настройках «Вставки».".into(),
        ureq::Error::StatusCode(c) => format!("GIPHY: HTTP {c}"),
        _ => "Не удалось подключиться к GIPHY. Проверь сеть и повтори поиск.".into(),
    }
}
fn trusted(url: &str) -> bool {
    if url.len() > 2048 {
        return false;
    }
    let Ok(uri) = url.parse::<tungstenite::http::Uri>() else { return false };
    uri.scheme_str() == Some("https") && uri.port_u16().is_none() && uri.host().is_some_and(|h| h == "giphy.com" || h.ends_with(".giphy.com"))
}
fn parse(bytes: &[u8]) -> Result<Vec<Gif>, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| "GIPHY вернул некорректный ответ")?;
    let data = value.get("data").and_then(|v| v.as_array()).ok_or("GIPHY: нет списка результатов")?;
    let mut out = Vec::new();
    for g in data.iter().take(24) {
        let Some(id) = g["id"].as_str().filter(|s| !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric())) else { continue };
        let Some(download) = g["images"]["original"]["url"].as_str().filter(|s| trusted(s)) else { continue };
        let Some(thumbnail) = g["images"]["fixed_width_small_still"]["url"].as_str().filter(|s| trusted(s)) else { continue };
        let page = g["url"].as_str().filter(|s| trusted(s)).unwrap_or("https://giphy.com/");
        let title = g["title"].as_str().unwrap_or("GIF").chars().take(200).collect();
        out.push(Gif { id: id.into(), title, page: page.into(), thumbnail: thumbnail.into(), download: download.into() });
    }
    Ok(out)
}
impl Host for NativeInserts {
    fn has_key(&self) -> bool {
        !self.key.is_empty()
    }
    fn save_key(&mut self, key: &str) -> Result<(), String> {
        if !(16..=256).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("Некорректный API-ключ GIPHY".into());
        }
        #[cfg(target_os = "macos")]
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, key.as_bytes()).map_err(|_| "Не удалось сохранить ключ GIPHY в связке ключей")?;
        self.key = key.into();
        Ok(())
    }
    fn search(&mut self, query: &str, offset: u32) -> Result<(), String> {
        if self.search.is_some() {
            return Err("Дождись завершения текущего поиска".into());
        }
        if self.key.is_empty() {
            return Err("Введи API-ключ GIPHY в Настройки → Вставки".into());
        }
        if query.trim().is_empty() || query.chars().count() > 50 || offset > 4992 {
            return Err("Некорректные параметры поиска".into());
        }
        let key = self.key.clone();
        let query = query.to_string();
        let (tx, rx) = mpsc::sync_channel(32);
        std::thread::Builder::new()
            .name("GIPHY search".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
                    let agent = agent();
                    let mut res = agent
                        .get("https://api.giphy.com/v1/gifs/search")
                        .query("api_key", &key)
                        .query("q", &query)
                        .query("limit", "24")
                        .query("offset", offset.to_string())
                        .query("rating", "pg-13")
                        .query("lang", if query.chars().any(|c| ('А'..='я').contains(&c)) { "ru" } else { "en" })
                        .call()
                        .map_err(error)?;
                    let bytes = res.body_mut().with_config().limit(4 * 1024 * 1024).read_to_vec().map_err(|_| "Ответ GIPHY слишком большой")?;
                    let rows = parse(&bytes)?;
                    if tx.send(Event::Results(Ok(rows.clone()))).is_err() {
                        return Ok(());
                    }
                    let started = std::time::Instant::now();
                    for row in rows {
                        if started.elapsed() > Duration::from_secs(30) {
                            break;
                        }
                        if let Ok(mut res) = agent.get(&row.thumbnail).call()
                            && let Ok(bytes) = res.body_mut().with_config().limit(1024 * 1024).read_to_vec()
                            && tx.send(Event::Thumbnail(row.id, bytes)).is_err()
                        {
                            break;
                        }
                    }
                    let _ = tx.send(Event::SearchFinished);
                    Ok(())
                }))
                .unwrap_or_else(|_| Err("Ошибка фонового поиска GIPHY".into()));
                if let Err(e) = result {
                    let _ = tx.send(Event::Results(Err(e)));
                }
            })
            .map_err(|_| "Не удалось запустить поиск")?;
        self.search = Some(rx);
        Ok(())
    }
    fn download(&mut self, gif: &Gif, folder: PathBuf) -> Result<(), String> {
        if !trusted(&gif.download) || gif.id.is_empty() || gif.id.len() > 100 || !gif.id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("Некорректная ссылка GIPHY".into());
        }
        let gif = gif.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("GIPHY download".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<PathBuf, String> {
                    let mut res = agent().get(&gif.download).call().map_err(error)?;
                    let bytes =
                        res.body_mut().with_config().limit(25 * 1024 * 1024).read_to_vec().map_err(|_| "GIF превышает лимит 25 МБ или загрузка прервана")?;
                    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
                        return Err("GIPHY вернул файл не в формате GIF".into());
                    }
                    std::fs::create_dir_all(&folder).map_err(|_| "Не удалось создать папку вставок")?;
                    let path = folder.join(format!("GIPHY-{}-{}.gif", gif.id, chrono::Utc::now().timestamp_millis()));
                    use std::io::Write;
                    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| "Не удалось создать файл GIF")?;
                    if let Err(_e) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                        let _ = std::fs::remove_file(&path);
                        return Err("Не удалось сохранить GIF".into());
                    }
                    Ok(path)
                }))
                .unwrap_or_else(|_| Err("Ошибка фоновой загрузки GIPHY".into()));
                let _ = tx.send(Event::Downloaded(result));
            })
            .map_err(|_| "Не удалось запустить загрузку")?;
        self.download = Some(rx);
        Ok(())
    }
    fn poll(&mut self) -> Option<Event> {
        if let Some(rx) = self.download.as_ref() {
            match rx.try_recv() {
                Ok(e) => {
                    self.download = None;
                    return Some(e);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.download = None;
                    return Some(Event::Downloaded(Err("Загрузка прервана".into())));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = self.search.as_ref() {
            match rx.try_recv() {
                Ok(e) => return Some(e),
                Err(mpsc::TryRecvError::Disconnected) => self.search = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inserts_rejects_untrusted_urls_and_hostile_results() {
        for u in ["http://media.giphy.com/a.gif", "https://giphy.com.evil.test/a", "https://127.0.0.1/a", "https://giphy.com:443/a", "file:///tmp/a"] {
            assert!(!trusted(u), "{u}");
        }
        assert!(trusted("https://media2.giphy.com/media/abc/giphy.gif"));
        for b in [b"null".as_slice(), b"{}", b"{\"data\":false}"] {
            assert!(parse(b).is_err());
        }
        assert!(parse(br#"{"data":[{"id":"../../bad"}]}"#).unwrap().is_empty());
    }
}
