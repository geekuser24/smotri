//! Disposable plugin worker. The editor never loads these native modules.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[cfg(target_os = "macos")]
mod au_host;
mod vst2_host;
mod vst3_host;
use filmcraft_audio_plugin_client::{PROTOCOL, receive, send};
use filmcraft_project::audio_plugin::*;
use std::net::TcpStream;
enum Processor {
    Vst3(vst3_host::Processor),
    Vst2(vst2_host::Processor),
    #[cfg(target_os = "macos")]
    Au(au_host::Processor),
}
impl Processor {
    fn process(&mut self, audio: Vec<Vec<f32>>, values: &std::collections::BTreeMap<String, f64>) -> Result<Vec<Vec<f32>>, String> {
        match self {
            Self::Vst3(p) => p.process(audio, values),
            Self::Vst2(p) => p.process(audio, values),
            #[cfg(target_os = "macos")]
            Self::Au(p) => p.process(audio, values),
        }
    }
    fn save(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Vst3(p) => p.save(),
            Self::Vst2(p) => p.save(),
            #[cfg(target_os = "macos")]
            Self::Au(p) => p.save(),
        }
    }
}
fn main() {
    let result = std::panic::catch_unwind(run).unwrap_or_else(|_| Err("Audio plugin worker panicked".into()));
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--worker") {
        return Err("This helper is started by Smotri".into());
    }
    let port = args.next().ok_or("Missing worker port")?.parse::<u16>().map_err(|e| e.to_string())?;
    let nonce = std::env::var("SMOTRI_PLUGIN_NONCE").map_err(|_| "Missing worker nonce")?;
    if port == 0 || nonce.len() != 64 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid worker handshake".into());
    }
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5))).map_err(|e| e.to_string())?;
    send(&mut stream, &(PROTOCOL, nonce))?;
    let mut processor: Option<Processor> = None;
    loop {
        let request: PluginRequest = receive(&mut stream)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<PluginResponse, String> {
            match request {
                PluginRequest::Scan { path, format: PluginFormat::Vst3 } => vst3_host::scan(&path).map(PluginResponse::Catalog),
                PluginRequest::Scan { path, format: PluginFormat::Vst2 } => vst2_host::scan(&path).map(PluginResponse::Catalog),
                #[cfg(target_os = "macos")]
                PluginRequest::Scan { path, format: PluginFormat::AudioUnit } => au_host::scan(&path).map(PluginResponse::Catalog),
                #[cfg(not(target_os = "macos"))]
                PluginRequest::Scan { .. } => Err("This plugin format is not implemented yet".into()),
                PluginRequest::Open { state, sample_rate, channels } => {
                    if !(8000..=384000).contains(&sample_rate) {
                        return Err("Invalid audio sample rate".into());
                    }
                    let (p, latency, tail) = match state.plugin.format {
                        PluginFormat::Vst2 => {
                            let p = vst2_host::Processor::open(&state, sample_rate, channels)?;
                            let (l, t) = (p.latency, p.tail);
                            (Processor::Vst2(p), l, t)
                        }
                        PluginFormat::Vst3 => {
                            let p = vst3_host::Processor::open(&state, sample_rate, channels)?;
                            let (l, t) = (p.latency, p.tail);
                            (Processor::Vst3(p), l, t)
                        }
                        #[cfg(target_os = "macos")]
                        PluginFormat::AudioUnit => {
                            let p = au_host::Processor::open(&state, sample_rate, channels)?;
                            let (l, t) = (p.latency, p.tail);
                            (Processor::Au(p), l, t)
                        }
                        #[cfg(not(target_os = "macos"))]
                        _ => return Err("Unsupported plugin format on this platform".into()),
                    };
                    processor = Some(p);
                    Ok(PluginResponse::Opened { latency, tail })
                }
                PluginRequest::Process { audio, values } => processor.as_mut().ok_or("Plugin not opened")?.process(audio, &values).map(PluginResponse::Audio),
                PluginRequest::SaveState => processor.as_ref().ok_or("Plugin not opened")?.save().map(PluginResponse::State),
                PluginRequest::Reset => {
                    processor = None;
                    Ok(PluginResponse::Done)
                }
            }
        }))
        .unwrap_or_else(|_| Err("Audio plugin operation panicked".into()));
        send(&mut stream, &result.unwrap_or_else(PluginResponse::Error))?;
    }
}
