//! yt-dlp/FFmpeg run as separate, bundled tools. Never invoke a shell or log cookies/URLs.
use filmcraft_engine::settings::DownloaderPrefs;
use filmcraft_ui_egui::{
    downloader::{Event, Host, MediaInfo, valid_quality, valid_url},
    i18n,
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct NativeDownloader {
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for NativeDownloader {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl NativeDownloader {
    fn start(&mut self, work: impl FnOnce(SyncSender<Event>, Arc<AtomicBool>) + Send + 'static) -> Result<(), String> {
        if self.receiver.is_some() {
            return Err("A download is already running".into());
        }
        let (tx, rx) = mpsc::sync_channel(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        thread::Builder::new()
            .name("filmcraft-downloader".into())
            .spawn(move || {
                let failure = tx.clone();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(tx, stop))).is_err() {
                    let _ = failure.send(Event::Finished(Err("Downloader worker failed; try again".into())));
                }
            })
            .map_err(|_| "Cannot start downloader worker")?;
        self.cancel = cancel;
        self.receiver = Some(rx);
        Ok(())
    }
}
impl Host for NativeDownloader {
    fn probe(&mut self, url: String, prefs: DownloaderPrefs) -> Result<(), String> {
        if !valid_url(&url) {
            return Err("Invalid URL".into());
        }
        self.start(move |tx, cancel| {
            let result = (|| {
                let mut command = base(&prefs)?;
                command.args(["--dump-single-json", "--skip-download", "--no-warnings", "--", &url]);
                let out = run(command, &cancel, &tx, Duration::from_secs(150), false)?;
                parse_info(&out)
            })();
            let _ = tx.send(Event::Info(result));
        })
    }
    fn download(&mut self, url: String, quality: String, folder: PathBuf, prefs: DownloaderPrefs) -> Result<(), String> {
        if !valid_url(&url) || !valid_quality(&quality) {
            return Err("Invalid download parameters".into());
        }
        self.start(move |tx, cancel| {
            let result = download(&url, &quality, &folder, &prefs, &cancel, &tx);
            let _ = tx.send(Event::Finished(result));
        })
    }
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    fn poll(&mut self) -> Option<Event> {
        let event = match self.receiver.as_ref()?.try_recv() {
            Ok(e) => e,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Event::Finished(Err("Downloader stopped unexpectedly".into())),
        };
        if matches!(event, Event::Info(_) | Event::Finished(_)) {
            self.receiver = None;
        }
        Some(event)
    }
}
fn tool(name: &str, custom: &str) -> Result<PathBuf, String> {
    if !custom.trim().is_empty() {
        let p = PathBuf::from(custom.trim());
        if p.is_absolute() && p.is_file() {
            return Ok(p);
        }
        return Err(format!("Invalid {name} executable path"));
    }
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        dirs.push(bin.join("DownloaderTools"));
        if let Some(contents) = bin.parent() {
            dirs.push(contents.join("Resources/DownloaderTools"));
            dirs.push(contents.join("lib/smotri/DownloaderTools"));
        }
    }
    if let Some(data) = filmcraft_engine::autosave::default_data_dir() {
        dirs.push(data.join("DownloaderTools/bin"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for dir in dirs {
        let p = dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.into() });
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(format!("{}: {name}", i18n::t("Download component is missing. Reinstall the complete Smotri application")))
}
fn base(prefs: &DownloaderPrefs) -> Result<Command, String> {
    let mut c = Command::new(tool("yt-dlp", &prefs.yt_dlp)?);
    c.args([
        "--ignore-config",
        "--no-playlist",
        "--no-colors",
        "--newline",
        "--socket-timeout",
        "20",
        "--retries",
        "3",
        "--fragment-retries",
        "3",
        "--extractor-retries",
        "2",
        "--max-filesize",
        "20G",
        "--no-cache-dir",
        "--no-mtime",
    ]);
    if let Ok(deno) = tool("deno", "") {
        c.arg("--js-runtimes").arg(format!("deno:{}", deno.display()));
    }
    match prefs.cookie_source.as_str() {
        "none" => {}
        "chrome" | "edge" | "firefox" | "safari" | "brave" => {
            c.args(["--cookies-from-browser", &prefs.cookie_source]);
        }
        "file" => {
            if !Path::new(&prefs.cookie_file).is_file() {
                return Err(i18n::t("Choose an existing Netscape cookies file in Settings").into());
            }
            c.arg("--cookies").arg(&prefs.cookie_file);
        }
        _ => return Err("Unsupported cookies source".into()),
    }
    c.stdin(Stdio::null());
    Ok(c)
}
fn parse_info(bytes: &[u8]) -> Result<MediaInfo, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Video information exceeds the size limit".into());
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "Could not read video information")?;
    if v.get("is_live").and_then(Value::as_bool) == Some(true) || v.get("_type").and_then(Value::as_str).is_some_and(|s| s == "playlist" || s == "multi_video")
    {
        return Err(i18n::t("Use a single recorded video link; playlists and live streams are not supported yet").into());
    }
    let mut heights = std::collections::BTreeSet::new();
    let mut audio = false;
    let mut video = false;
    let formats = v.get("formats").and_then(Value::as_array);
    if formats.is_some_and(|f| f.len() > 10_000) {
        return Err("Too many video formats".into());
    }
    for f in std::iter::once(&v).chain(formats.into_iter().flatten()) {
        if f.get("has_drm").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if f.get("acodec").and_then(Value::as_str).is_some_and(|s| s != "none") {
            audio = true;
        }
        if f.get("vcodec").and_then(Value::as_str).is_some_and(|s| s != "none") {
            video = true;
        }
        if let Some(h) = f.get("height").and_then(Value::as_u64).filter(|h| (1..=8640).contains(h)) {
            heights.insert(h as u32);
            video = true;
        }
    }
    // Generic direct-media links may omit codec metadata until the file is downloaded.
    if !audio && !video {
        match v.get("ext").and_then(Value::as_str) {
            Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" | "m3u8") => {
                video = true;
                audio = true;
            }
            Some("mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "flac") => audio = true,
            _ => {}
        }
    }
    if !audio && !video {
        return Err(i18n::t("No downloadable audio or video was found").into());
    }
    Ok(MediaInfo {
        title: v.get("title").and_then(Value::as_str).unwrap_or("Download").chars().filter(|c| !c.is_control()).take(240).collect(),
        heights: heights.into_iter().rev().collect(),
        audio,
        video,
        duration: v.get("duration").and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0),
    })
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        stop(&mut self.0);
    }
}
fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill").args(["-KILL", "--", &format!("-{}", child.id())]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
fn reader(mut input: impl Read + Send + 'static, tx: mpsc::SyncSender<(bool, Vec<u8>)>, stdout: bool) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name("download-output".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut buffer = [0u8; 4096];
                while let Ok(n) = input.read(&mut buffer) {
                    if n == 0 || tx.send((stdout, buffer[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }));
        })
        .map_err(|_| "Could not monitor download process".into())
}
fn classify_error(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).to_lowercase();
    i18n::t(
        if text.contains("cookie") || text.contains("sign in") || text.contains("login") || text.contains("age-restricted") || text.contains("confirm your age")
        {
            "This site requires sign-in. Choose a signed-in browser or a fresh cookies file in Downloader settings."
        } else if text.contains("unsupported url") {
            "This link is not supported by the bundled downloader."
        } else if text.contains("403") || text.contains("429") || text.contains("bot") {
            "The site blocked the request. Try your browser cookies or retry later."
        } else if text.contains("requested format") || text.contains("no video formats") {
            "This quality is not available. Try Best available or Audio only."
        } else if text.contains("space left") {
            "Not enough disk space for the download."
        } else if text.contains("drm") {
            "DRM-protected media cannot be downloaded."
        } else {
            "Download failed. Check the link, network and browser cookies, then retry."
        },
    )
    .into()
}
fn run(mut command: Command, cancel: &AtomicBool, events: &SyncSender<Event>, timeout: Duration, progress: bool) -> Result<Vec<u8>, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = Running(command.spawn().map_err(|_| i18n::t("Could not start the bundled download component"))?);
    let stdout = child.0.stdout.take().ok_or("No process output")?;
    let stderr = child.0.stderr.take().ok_or("No process diagnostics")?;
    // Readers are bounded by their OS pipes and a bounded intermediary channel below.
    let (tx, rx) = mpsc::sync_channel(32);
    let a = reader(stdout, tx.clone(), true)?;
    let b = reader(stderr, tx, false)?;
    let start = Instant::now();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut line = Vec::new();
    let mut exit = None;
    let mut closed = false;
    while exit.is_none() || !closed {
        if cancel.load(Ordering::Relaxed) {
            return Err(i18n::t("Download cancelled").into());
        }
        if start.elapsed() > timeout {
            return Err(i18n::t("Download timed out. Try again.").into());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok((is_out, bytes)) => {
                let buf = if is_out { &mut output } else { &mut errors };
                let limit = if is_out { 8 * 1024 * 1024 } else { 64 * 1024 };
                if is_out && buf.len().saturating_add(bytes.len()) > limit {
                    return Err("Download output exceeded the size limit".into());
                }
                buf.extend(bytes.iter().take(limit.saturating_sub(buf.len())));
                if progress && is_out {
                    for c in bytes {
                        if c == b'\n' || c == b'\r' {
                            let s = String::from_utf8_lossy(&line);
                            if let Some(n) =
                                s.strip_prefix("FC_PROGRESS:").and_then(|v| v.trim().trim_end_matches('%').trim().parse::<f32>().ok()).filter(|n| n.is_finite())
                            {
                                let _ = events.try_send(Event::Progress((n / 100.0).clamp(0.0, 1.0) * 0.8, i18n::t("Downloading…").into()));
                            }
                            line.clear();
                        } else if line.len() < 65536 {
                            line.push(c);
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => closed = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        exit = child.0.try_wait().map_err(|_| "Could not check download progress")?;
    }
    let _ = a.join();
    let _ = b.join();
    if !exit.is_some_and(|e| e.success()) {
        return Err(classify_error(&errors));
    }
    Ok(output)
}
struct WorkDir(PathBuf);
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn download(url: &str, quality: &str, folder: &Path, prefs: &DownloaderPrefs, cancel: &AtomicBool, events: &SyncSender<Event>) -> Result<PathBuf, String> {
    let ffmpeg = tool("ffmpeg", &prefs.ffmpeg)?;
    std::fs::create_dir_all(folder).map_err(|_| "Cannot create the download folder")?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%f").to_string();
    let work = WorkDir(folder.join(format!(".filmcraft-download-{stamp}")));
    std::fs::create_dir(&work.0).map_err(|_| "Cannot create a temporary download folder")?;
    let mut c = base(prefs)?;
    let format = if quality == "audio" {
        "bestaudio/best".into()
    } else {
        let cap = if quality == "best" { "8640" } else { quality };
        format!("bv*[height<=?{cap}]+ba/b[height<=?{cap}]/bv[height<=?{cap}]")
    };
    c.args([
        "--format",
        &format,
        "--merge-output-format",
        "mkv",
        "--no-simulate",
        "--progress",
        "--progress-delta",
        "0.5",
        "--progress-template",
        "download:FC_PROGRESS:%(progress._percent_str)s",
        "--print",
        "after_move:FC_FILE:%(filepath)s",
    ])
    .arg("--ffmpeg-location")
    .arg(&ffmpeg)
    .arg("--output")
    .arg(work.0.join("source.%(ext)s"))
    .arg("--")
    .arg(url);
    let output = run(c, cancel, events, Duration::from_secs(6 * 60 * 60), true)?;
    let source = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|l| l.strip_prefix("FC_FILE:"))
        .next_back()
        .map(PathBuf::from)
        .ok_or("No completed file returned by downloader")?;
    let source = source.canonicalize().map_err(|_| "Downloaded file is missing")?;
    let root = work.0.canonicalize().map_err(|_| "Download folder is missing")?;
    if !source.starts_with(&root) || !source.is_file() {
        return Err("Downloader returned an invalid file path".into());
    }
    let _ = events.try_send(Event::Progress(0.82, i18n::t("Preparing media for editing…").into()));
    let extension = if quality == "audio" { "wav" } else { "mp4" };
    let normalized = work.0.join(format!("prepared.{extension}"));
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"]).arg(&source);
    if quality == "audio" {
        c.args(["-vn", "-map", "0:a:0", "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"]);
    } else {
        let cap = if quality == "best" { "8640" } else { quality };
        let scale = format!("scale=-2:'min({cap},trunc(ih/2)*2)'");
        c.args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-sn",
            "-dn",
            "-vf",
            &scale,
            "-c:v",
            "h264_videotoolbox",
            "-q:v",
            "65",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-movflags",
            "+faststart",
        ]);
    }
    c.arg(&normalized);
    run(c, cancel, events, Duration::from_secs(6 * 60 * 60), false)?;
    if normalized.metadata().map_err(|_| "Prepared file missing")?.len() == 0 {
        return Err("Prepared file is empty".into());
    }
    let final_path = folder.join(format!("Download-{stamp}.{extension}"));
    // Copy into a new file, never overwrite an existing project asset.
    let mut dest = std::fs::OpenOptions::new().write(true).create_new(true).open(&final_path).map_err(|_| "Cannot save the downloaded file")?;
    let result = (|| {
        let mut src = std::fs::File::open(&normalized).map_err(|_| "Cannot read prepared media")?;
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(i18n::t("Download cancelled").into());
            }
            let n = src.read(&mut buffer).map_err(|_| "Cannot read prepared media")?;
            if n == 0 {
                break;
            }
            dest.write_all(&buffer[..n]).map_err(|_| "Cannot save media; check disk space")?;
        }
        dest.flush().map_err(|_| "Cannot flush downloaded media")?;
        dest.sync_all().map_err(|_| "Cannot finish saving downloaded media")?;
        Ok::<(), String>(())
    })();
    if let Err(e) = result {
        drop(dest);
        let _ = std::fs::remove_file(&final_path);
        return Err(e);
    }
    Ok(final_path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn cancellation_stops_child_and_output_is_bounded() {
        let (tx, _rx) = mpsc::sync_channel(32);
        let mut c = Command::new("/bin/sleep");
        c.arg("30");
        let started = Instant::now();
        assert!(run(c, &AtomicBool::new(true), &tx, Duration::from_secs(2), false).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        let c = Command::new("/usr/bin/yes");
        assert!(run(c, &AtomicBool::new(false), &tx, Duration::from_secs(3), false).is_err());
    }
    #[test]
    fn metadata_is_bounded_and_sanitized() {
        let x = parse_info(br#"{"title":"test","duration":2,"formats":[{"height":1080,"vcodec":"avc1","acodec":"none"},{"acodec":"aac","vcodec":"none"}]}"#)
            .unwrap();
        assert_eq!(x.heights, vec![1080]);
        assert!(x.audio && x.video);
        for data in [b"{}".as_slice(), b"{", br#"{"is_live":true}"#, br#"{"_type":"playlist"}"#] {
            assert!(parse_info(data).is_err());
        }
        assert!(!classify_error(b"ERROR cookie secret=abc https://secret").contains("abc"));
    }
}
