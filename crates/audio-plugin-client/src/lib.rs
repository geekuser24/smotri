//! Safe IPC client for isolated audio-plugin workers; never loads native libraries.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#[cfg(not(target_arch = "wasm32"))]
use filmcraft_project::audio_plugin::{PluginRequest, PluginResponse};
use std::io::{Read, Write};
pub const MAX_PACKET: usize = 48 * 1024 * 1024;
pub const PROTOCOL: u32 = 1;

pub fn send<T: serde::Serialize>(stream: &mut impl Write, value: &T) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if data.len() > MAX_PACKET {
        return Err("Plugin IPC message too large".into());
    }
    stream.write_all(&(data.len() as u32).to_le_bytes()).and_then(|_| stream.write_all(&data)).map_err(|e| e.to_string())
}
pub fn receive<T: serde::de::DeserializeOwned>(stream: &mut impl Read) -> Result<T, String> {
    let mut size = [0; 4];
    stream.read_exact(&mut size).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(size) as usize;
    if len > MAX_PACKET {
        return Err("Plugin IPC message too large".into());
    }
    let mut data = vec![0; len];
    stream.read_exact(&mut data).map_err(|e| e.to_string())?;
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    static WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            WORKERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    pub struct Worker {
        _permit: Permit,
        child: Child,
        stream: TcpStream,
    }
    fn blocking_stream(stream: &TcpStream) -> Result<(), String> {
        // Accepted sockets can inherit O_NONBLOCK on macOS. Timeout-bound reads must block
        // through partial messages rather than spuriously fail with WouldBlock.
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
        Ok(())
    }
    impl Worker {
        pub fn start() -> Result<Self, String> {
            WORKERS
                .try_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| (n < 32).then_some(n + 1))
                .map_err(|_| "Audio plugin worker limit reached (32)")?;
            let permit = Permit;
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let filename = if cfg!(windows) { "filmcraft-audio-plugin-host.exe" } else { "filmcraft-audio-plugin-host" };
            let path: PathBuf = executable.parent().ok_or("No executable directory")?.join(filename);
            if !path.is_file() {
                return Err("Audio plugin host is missing from this installation".into());
            }
            let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let mut nonce = [0u8; 32];
            getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
            let token: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let mut command = Command::new(path);
            command.args(["--worker", &port.to_string()]).env("SMOTRI_PLUGIN_NONCE", &token).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: workers have no console UI.
            }
            let mut child = command.spawn().map_err(|e| format!("Cannot start audio plugin host: {e}"))?;
            let connected = (|| {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            blocking_stream(&stream)?;
                            let (version, proof): (u32, String) = receive(&mut stream)?;
                            if version != PROTOCOL || proof != token {
                                return Err("Invalid audio plugin host handshake".into());
                            }
                            stream.set_nodelay(true).map_err(|e| e.to_string())?;
                            return Ok(stream);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e.to_string()),
                    }
                    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                        return Err("Audio plugin host exited before connecting".into());
                    }
                    if Instant::now() >= deadline {
                        return Err("Audio plugin host connection timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })();
            match connected {
                Ok(stream) => Ok(Self { _permit: permit, child, stream }),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(e)
                }
            }
        }
        pub fn request(&mut self, request: &PluginRequest) -> Result<PluginResponse, String> {
            // Scanning/initialization may authorize a plugin or read its resources. Playback
            // has a much shorter deadline; a timeout kills this worker on drop.
            let seconds = if matches!(request, PluginRequest::Scan { .. } | PluginRequest::Open { .. }) { 20 } else { 2 };
            self.stream.set_read_timeout(Some(Duration::from_secs(seconds))).map_err(|e| e.to_string())?;
            send(&mut self.stream, request)?;
            match receive(&mut self.stream)? {
                PluginResponse::Error(e) => Err(e),
                response => Ok(response),
            }
        }
    }
    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn accepted_nonblocking_socket_waits_for_a_fragmented_reply() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let sender = std::thread::spawn(move || {
                let mut s = TcpStream::connect(address).unwrap();
                let bytes = serde_json::to_vec(&PluginResponse::Done).unwrap();
                s.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                s.write_all(&bytes).unwrap();
            });
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nonblocking(true).unwrap();
            blocking_stream(&stream).unwrap();
            assert!(matches!(receive::<PluginResponse>(&mut stream).unwrap(), PluginResponse::Done));
            sender.join().unwrap();
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::Worker;

pub mod catalog;
/// Failure latch: a failed enabled plugin never silently turns into a dry effect. Playback is
/// muted for that processor; export checks these errors after every pull and refuses delivery.
fn errors() -> &'static std::sync::Mutex<std::collections::BTreeMap<String, String>> {
    static E: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<String, String>>> = std::sync::OnceLock::new();
    E.get_or_init(Default::default)
}
pub fn failures() -> Vec<String> {
    errors().lock().unwrap_or_else(std::sync::PoisonError::into_inner).values().cloned().collect()
}
/// Collect only plugins reachable from the rendered sequence. Avoid serializing the project
/// (and unrelated sequences) on every playback frame; nested projects can contain cycles.
pub fn enabled_sequence_ids(project: &filmcraft_project::Project, root: filmcraft_project::ItemId) -> Vec<String> {
    let mut pending = vec![root];
    let mut seen = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(seq) = project.sequence(id) else {
            continue;
        };
        let mut collect = |effects: &[filmcraft_project::EffectInstance]| {
            for e in effects.iter().filter(|e| e.enabled && e.effect.starts_with("installed_audio_")) {
                ids.insert(e.effect.clone());
            }
        };
        collect(&seq.master_effects);
        for track in seq.all_tracks().chain(&seq.submix_tracks).filter(|t| t.enabled && !t.muted) {
            collect(&track.effects);
            for clip in track.items.iter().filter(|c| c.enabled) {
                collect(&clip.effects);
                if project.sequence(clip.item).is_some() && !seen.contains(&clip.item) {
                    pending.push(clip.item);
                }
            }
        }
    }
    ids.into_iter().collect()
}
pub fn check_ids(ids: &[String]) -> Result<(), String> {
    let errors = errors().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for id in ids {
        if let Some(error) = errors.get(id) {
            return Err(error.clone());
        }
    }
    Ok(())
}

struct Effect {
    id: String,
    state: filmcraft_project::audio_plugin::AudioPluginState,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
    latency: usize,
    failed: bool,
}
impl Effect {
    fn fail(&mut self, error: String) {
        self.failed = true;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.worker = None;
        }
        let mut e = errors().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if e.len() < 512 {
            e.insert(self.id.clone(), format!("{}: {error}. Disable or remove the failed plugin before exporting.", self.state.plugin.name));
        }
    }
}
pub fn create(instance: &filmcraft_project::EffectInstance, sr: u32, channels: usize) -> Box<dyn filmcraft_audio_dsp::AudioEffect> {
    use filmcraft_project::audio_plugin::*;
    let parsed = instance_state(instance);
    let parse_error = parsed.as_ref().err().cloned();
    let mut state = parsed.unwrap_or_else(|_| AudioPluginState {
        plugin: AudioPlugin {
            format: PluginFormat::Vst3,
            path: String::new(),
            native_id: instance.effect.clone(),
            name: instance.effect.clone(),
            vendor: String::new(),
            category: String::new(),
            parameters: vec![],
        },
        values: Default::default(),
        state: vec![],
    });
    // Resolve the native installation from the local catalog when projects move machines.
    let local = catalog::local(&instance.effect);
    if let Some(local) = &local {
        state.plugin = local.clone();
    }
    let unavailable = parse_error.or_else(|| local.is_none().then(|| "Plugin is not in the local catalog. Scan installed plugins in Settings.".to_string()));
    let mut effect = Effect {
        id: instance.effect.clone(),
        state,
        #[cfg(not(target_arch = "wasm32"))]
        worker: None,
        latency: 0,
        failed: false,
    };
    if let Some(error) = unavailable {
        effect.fail(error);
        return Box::new(effect);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let opened = (|| {
            let mut w = Worker::start()?;
            let PluginResponse::Opened { latency, .. } = w.request(&PluginRequest::Open { state: effect.state.clone(), sample_rate: sr, channels })? else {
                return Err("Unexpected plugin open response".into());
            };
            Ok((w, latency))
        })();
        match opened {
            Ok((w, latency)) => {
                effect.worker = Some(w);
                effect.latency = latency;
            }
            Err(e) => effect.fail(e),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (sr, channels);
        effect.fail("Installed plugins need the desktop editor".into());
    }
    Box::new(effect)
}
impl filmcraft_audio_dsp::AudioEffect for Effect {
    fn id(&self) -> &'static str {
        "installed_audio"
    }
    fn params(&self) -> &'static [filmcraft_audio_dsp::ParamSpec] {
        &[]
    }
    fn param(&self, id: &str) -> Option<f32> {
        self.state.values.get(id).map(|v| *v as f32)
    }
    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if let Some(p) = self.state.plugin.parameters.iter().find(|p| p.id == id)
            && value.is_finite()
        {
            self.state.values.insert(id.into(), f64::from(value).clamp(p.min, p.max));
            true
        } else {
            false
        }
    }
    fn reset(&mut self) {
        self.fail("The plugin processor must be recreated after a seek".into());
    }
    fn latency(&self) -> usize {
        self.latency
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        if !self.failed {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(worker) = &mut self.worker {
                let audio = channels.iter().map(|c| c.to_vec()).collect();
                match worker.request(&PluginRequest::Process { audio, values: self.state.values.clone() }) {
                    Ok(PluginResponse::Audio(audio))
                        if audio.len() == channels.len()
                            && audio.iter().zip(channels.iter()).all(|(a, b)| a.len() == b.len())
                            && filmcraft_project::audio_plugin::validate_audio(&audio).is_ok() =>
                    {
                        for (out, input) in channels.iter_mut().zip(audio) {
                            out.copy_from_slice(&input);
                        }
                        return;
                    }
                    Ok(_) => self.fail("Invalid plugin output".into()),
                    Err(e) => self.fail(e),
                }
            }
        }
        for c in channels {
            c.fill(0.0);
        }
    }
}

pub fn clear_failures() {
    errors().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_oversized_packets_fail_without_panics() {
        for bytes in [vec![], vec![255; 4], vec![3, 0, 0, 0, b'x'], vec![2, 0, 0, 0, b'{', b'}']] {
            assert!(receive::<PluginResponse>(&mut &bytes[..]).is_err());
        }
        let mut bytes = vec![];
        send(&mut bytes, &PluginResponse::Done).unwrap();
        assert!(matches!(receive::<PluginResponse>(&mut &bytes[..]).unwrap(), PluginResponse::Done));
    }
}
