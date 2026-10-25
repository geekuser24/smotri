//! Native Jarvis adapter: Keychain, bounded microphone capture and Scribe v2 Realtime.
use base64::Engine as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filmcraft_ui_egui::jarvis::{Event, VoiceHost};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};
use tungstenite::{Message, client::IntoClientRequest, stream::MaybeTlsStream};

pub struct NativeVoice {
    key: String,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
    events: Option<Receiver<Event>>,
    speech: Option<Receiver<Result<(), String>>>,
    speech_cancel: Arc<AtomicBool>,
}
impl NativeVoice {
    pub fn new() -> Self {
        let saved = crate::credentials::load(false);
        Self {
            key: saved.or_else(|| std::env::var("ELEVENLABS_API_KEY").ok()).unwrap_or_default(),
            stop: Arc::new(AtomicBool::new(true)),
            socket: Arc::default(),
            events: None,
            speech: None,
            speech_cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl VoiceHost for NativeVoice {
    fn has_key(&self) -> bool {
        !self.key.is_empty()
    }
    fn save_key(&mut self, key: &str) -> Result<(), String> {
        let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        if key.len() < 20 || key.len() > 256 {
            return Err("Invalid ElevenLabs API key".into());
        }
        crate::credentials::save(false, &key)?;
        self.key = key;
        Ok(())
    }
    fn start(&mut self) -> Result<(), String> {
        self.stop();
        if self.key.is_empty() {
            return Err("Enter an ElevenLabs API key first".into());
        }
        self.stop = Arc::new(AtomicBool::new(false));
        self.socket = Arc::default();
        let stop = self.stop.clone();
        let socket = self.socket.clone();
        let key = self.key.clone();
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("Jarvis Scribe".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(key, &stop, &socket, &tx)));
                if !stop.load(Ordering::Acquire) {
                    let error = match outcome {
                        Ok(Err(e)) => e,
                        Err(_) => "Jarvis worker failed; microphone stopped".into(),
                        Ok(Ok(())) => "Voice connection closed".into(),
                    };
                    let _ = tx.try_send(Event::Error(error));
                }
                stop.store(true, Ordering::Release);
            })
            .map_err(|_| "Could not start Jarvis worker")?;
        self.events = Some(rx);
        Ok(())
    }
    fn stop(&mut self) {
        self.speech_cancel.store(true, Ordering::Release);
        self.speech = None;
        self.stop.store(true, Ordering::Release);
        if let Some(socket) = self.socket.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.events = None;
    }
    fn cancel_speech(&mut self) {
        self.speech_cancel.store(true, Ordering::Release);
        self.speech = None;
    }
    fn speak(&mut self, text: &str) -> Result<(), String> {
        self.cancel_speech();
        self.speech_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.speech_cancel.clone();
        let key = self.key.clone();
        let text = text.to_owned();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("Jarvis speech reply".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::jarvis_reply::speak(&key, &text, &cancel)))
                    .unwrap_or_else(|_| Err("Speech reply worker failed".into()));
                let _ = tx.try_send(result);
            })
            .map_err(|_| "Could not start speech reply")?;
        self.speech = Some(rx);
        Ok(())
    }
    fn poll(&mut self) -> Option<Event> {
        if let Some(rx) = self.speech.as_ref() {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(mpsc::TryRecvError::Empty) => {
                    return self.poll_microphone();
                }
                Err(_) => Err("Speech reply worker disconnected".into()),
            };
            self.speech = None;
            return Some(Event::SpeechFinished(result));
        }
        self.poll_microphone()
    }
}
impl NativeVoice {
    fn poll_microphone(&mut self) -> Option<Event> {
        let rx = self.events.as_ref()?;
        if self.stop.load(Ordering::Acquire) {
            // A failed/disconnected worker cannot leave queued edit commands behind.
            let error = rx.try_iter().find_map(|event| match event {
                Event::Error(e) => Some(e),
                _ => None,
            });
            self.events = None;
            return Some(Event::Error(error.unwrap_or_else(|| "Voice connection closed; click Jarvis to reconnect".into())));
        }
        match rx.try_recv() {
            Ok(e) => Some(e),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.events = None;
                Some(Event::Error("Voice connection closed; click Jarvis to reconnect".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
impl Drop for NativeVoice {
    fn drop(&mut self) {
        self.stop();
    }
}

fn input_stream<T: cpal::SizedSample>(
    dev: &cpal::Device,
    cfg: &cpal::StreamConfig,
    tx: SyncSender<Vec<u8>>,
    failed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(cfg.channels);
    let bytes_per_chunk = (cfg.sample_rate.0 as usize / 10).saturating_mul(2);
    let mut chunk = Vec::with_capacity(bytes_per_chunk);
    let callback_failed = failed.clone();
    dev.build_input_stream(
        cfg,
        move |data: &[T], _| {
            let process = || {
                if stop.load(Ordering::Acquire) || callback_failed.load(Ordering::Relaxed) {
                    return;
                }
                if channels == 0 || data.len() > 1_048_576 {
                    callback_failed.store(true, Ordering::Release);
                    return;
                }
                for frame in data.chunks_exact(channels) {
                    let mono = frame
                        .iter()
                        .map(|s| {
                            let s = s.to_sample::<f32>();
                            if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }
                        })
                        .sum::<f32>()
                        / channels as f32;
                    chunk.extend_from_slice(&((mono * 32767.0).round() as i16).to_le_bytes());
                    if chunk.len() >= bytes_per_chunk {
                        let ready = std::mem::replace(&mut chunk, Vec::with_capacity(bytes_per_chunk));
                        if tx.try_send(ready).is_err() {
                            callback_failed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(process)).is_err() {
                callback_failed.store(true, Ordering::Release);
            }
        },
        move |_| {
            failed.store(true, Ordering::Release);
        },
        None,
    )
    .map_err(|e| format!("Microphone: {e}"))
}
fn run(key: String, stop: &Arc<AtomicBool>, socket_slot: &Arc<Mutex<Option<TcpStream>>>, events: &SyncSender<Event>) -> Result<(), String> {
    // The capture device is opened only after an explicit toggle, never during app initialization.
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No microphone found")?;
    let default = device.default_input_config().map_err(|e| format!("Microphone: {e}"))?;
    let rates = [16000, 48000, 44100, 24000, 22050, 8000];
    let supported = if rates.contains(&default.sample_rate().0) {
        default
    } else {
        device
            .supported_input_configs()
            .map_err(|e| e.to_string())?
            .find_map(|c| rates.iter().find_map(|&rate| c.try_with_sample_rate(cpal::SampleRate(rate))))
            .ok_or("Microphone does not support a Scribe sample rate")?
    };
    if supported.channels() == 0 || supported.channels() > 32 {
        return Err("Invalid microphone channel count".into());
    }
    let rate = supported.sample_rate().0;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let _ = rustls_rustcrypto::provider().install_default();
    let url = format!(
        "wss://api.elevenlabs.io/v1/speech-to-text/realtime?model_id=scribe_v2_realtime&audio_format=pcm_{rate}&commit_strategy=vad&vad_silence_threshold_secs=0.5&secondary_languages=ru&secondary_languages=en&include_timestamps=false"
    );
    let mut request = url.into_client_request().map_err(|_| "Invalid Scribe endpoint")?;
    request.headers_mut().insert("xi-api-key", key.parse().map_err(|_| "Invalid API key format")?);
    drop(key);
    let mut tcp = None;
    for addr in ("api.elevenlabs.io", 443).to_socket_addrs().map_err(|_| "Cannot resolve ElevenLabs; check your connection")?.take(4) {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            tcp = Some(stream);
            break;
        }
    }
    let tcp = tcp.ok_or("Cannot connect to ElevenLabs")?;
    tcp.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    *socket_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(tcp.try_clone().map_err(|e| e.to_string())?);
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let ws_config = tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(65536)).max_frame_size(Some(65536));
    let (mut ws, _) = tungstenite::client_tls_with_config(request, tcp, Some(ws_config), None).map_err(|error| {
        if let tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response)) = error
            && response.headers().get("location").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("https://geo-restriction.elevenlabs.io"))
        {
            return "ElevenLabs is unavailable in this network region";
        }
        "ElevenLabs connection rejected; check the API key, Scribe access and connection"
    })?;
    match ws.get_mut() {
        MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(Some(Duration::from_millis(30))),
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(Duration::from_millis(30))),
        _ => return Err("Unsupported TLS transport".into()),
    }
    .map_err(|e| e.to_string())?;
    let (audio_tx, audio_rx) = mpsc::sync_channel(20);
    let failed = Arc::new(AtomicBool::new(false));
    let mut stream = None;
    let mut audio_tx = Some(audio_tx);
    let connected = Instant::now();
    let mut last_audio = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if failed.load(Ordering::Acquire) {
            return Err("Microphone disconnected or audio queue overflowed; voice mode stopped".into());
        }
        for _ in 0..4 {
            let Ok(audio) = audio_rx.try_recv() else {
                break;
            };
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let msg = serde_json::json!({"message_type":"input_audio_chunk","audio_base_64":base64::engine::general_purpose::STANDARD.encode(audio),"sample_rate":rate,"commit":false});
            ws.send(Message::Text(msg.to_string().into())).map_err(|_| "Scribe audio upload failed")?;
            last_audio = Instant::now();
        }
        if stream.is_none() && connected.elapsed() > Duration::from_secs(12) {
            return Err("Scribe session timed out".into());
        }
        if stream.is_some() && last_audio.elapsed() > Duration::from_secs(5) {
            return Err("Microphone stopped providing audio".into());
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| "Invalid Scribe response")?;
                let kind = value.get("message_type").and_then(|v| v.as_str()).unwrap_or("");
                match kind {
                    "session_started" if stream.is_none() => {
                        if stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let tx = audio_tx.take().ok_or("Microphone already started")?;
                        let mic = match format {
                            cpal::SampleFormat::F32 => input_stream::<f32>(&device, &config, tx, failed.clone(), stop.clone()),
                            cpal::SampleFormat::I16 => input_stream::<i16>(&device, &config, tx, failed.clone(), stop.clone()),
                            cpal::SampleFormat::I32 => input_stream::<i32>(&device, &config, tx, failed.clone(), stop.clone()),
                            cpal::SampleFormat::U16 => input_stream::<u16>(&device, &config, tx, failed.clone(), stop.clone()),
                            cpal::SampleFormat::F64 => input_stream::<f64>(&device, &config, tx, failed.clone(), stop.clone()),
                            _ => Err("Unsupported microphone sample format".into()),
                        }?;
                        mic.play().map_err(|e| format!("Cannot start microphone: {e}"))?;
                        stream = Some(mic);
                        last_audio = Instant::now();
                        events.try_send(Event::Ready).map_err(|_| "Jarvis event queue unavailable")?;
                    }
                    "committed_transcript" => {
                        if let Some(text) = value.get("text").and_then(|v| v.as_str()).filter(|t| !t.is_empty() && t.len() <= 2048) {
                            events.try_send(Event::Transcript(text.into(), Instant::now())).map_err(|_| "Jarvis event queue overflowed")?;
                        }
                    }
                    "error" | "auth_error" | "quota_exceeded" | "rate_limited" | "session_time_limit_exceeded" => {
                        return Err(format!("ElevenLabs: {kind}; check your key and account quota"));
                    }
                    k if k.ends_with("_error") => return Err(format!("ElevenLabs: {k}")),
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => return Err("ElevenLabs closed the session; click Jarvis to reconnect".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return Err("Scribe connection interrupted".into()),
        }
    }
    drop(stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speech_does_not_starve_microphone_events_and_cancel_keeps_session() {
        let (tx, rx) = mpsc::sync_channel(4);
        let (_speech_tx, speech_rx) = mpsc::sync_channel(1);
        let mut host = NativeVoice {
            key: String::new(),
            stop: Arc::new(AtomicBool::new(false)),
            socket: Arc::default(),
            events: Some(rx),
            speech: Some(speech_rx),
            speech_cancel: Arc::new(AtomicBool::new(false)),
        };
        tx.send(Event::Transcript("two".into(), Instant::now())).unwrap();
        assert!(matches!(host.poll(), Some(Event::Transcript(t, _)) if t == "two"));
        host.cancel_speech();
        assert!(host.speech_cancel.load(Ordering::Acquire));
        assert!(!host.stop.load(Ordering::Acquire));
        tx.send(Event::Ready).unwrap();
        assert!(matches!(host.poll(), Some(Event::Ready)));
        assert!(host.poll().is_none());
    }
}
