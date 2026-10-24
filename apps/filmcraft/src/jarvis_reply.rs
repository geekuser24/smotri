//! ElevenLabs clarification speech. Playback is cancellable independently of the live microphone transport.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";
pub fn speak(key: &str, text: &str, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if text.chars().count() > 600 || key.is_empty() {
        return Err("Invalid speech reply request".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(45)))
        .build()
        .into();
    let body = serde_json::to_vec(&serde_json::json!({"text":text,"model_id":"eleven_flash_v2_5"})).map_err(|_| "Invalid speech reply")?;
    let mut response = agent
        .post(format!("https://api.elevenlabs.io/v1/text-to-speech/{VOICE}?output_format=pcm_24000"))
        .header("xi-api-key", key)
        .header("Content-Type", "application/json")
        .send(&body)
        .map_err(|e| match e {
            ureq::Error::StatusCode(c) => format!("ElevenLabs speech: HTTP {c}; the question is available as text"),
            _ => "ElevenLabs speech connection failed; the question is available as text".into(),
        })?;
    let bytes = response.body_mut().with_config().limit(4 * 1024 * 1024).read_to_vec().map_err(|_| "Speech audio is too large or unreadable")?;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return Err("Invalid PCM speech response".into());
    }
    let samples: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.).collect();
    play(samples, cancel)
}
fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Vec<f32>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let mut cursor = 0.0;
    let step = 24000.0 / config.sample_rate.0 as f64;
    let channels = config.channels as usize;
    let failure = failed.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let process = || {
                    for frame in data.chunks_mut(channels) {
                        let i = cursor as usize;
                        let value = if cancel.load(Ordering::Acquire) || i >= samples.len() {
                            done.store(true, Ordering::Release);
                            0.
                        } else {
                            let a = samples.get(i).copied().unwrap_or(0.);
                            let b = samples.get(i + 1).copied().unwrap_or(a);
                            let v = a + (b - a) * (cursor - i as f64) as f32;
                            cursor += step;
                            v
                        };
                        for sample in frame {
                            *sample = T::from_sample(value);
                        }
                    }
                };
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(process)).is_err() {
                    failure.store(true, Ordering::Release);
                }
            },
            move |_| {
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| format!("Speech output: {e}"))
}
fn play(samples: Vec<f32>, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No audio output device for Jarvis")?;
    let supported = device.default_output_config().map_err(|_| "Cannot open the audio output device")?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    if config.channels == 0 || config.channels > 32 || config.sample_rate.0 == 0 {
        return Err("Invalid speech output configuration".into());
    }
    let done = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match format {
        cpal::SampleFormat::F32 => output::<f32>(&device, &config, samples, cancel.clone(), done.clone(), failed.clone()),
        cpal::SampleFormat::F64 => output::<f64>(&device, &config, samples, cancel.clone(), done.clone(), failed.clone()),
        cpal::SampleFormat::I16 => output::<i16>(&device, &config, samples, cancel.clone(), done.clone(), failed.clone()),
        cpal::SampleFormat::I32 => output::<i32>(&device, &config, samples, cancel.clone(), done.clone(), failed.clone()),
        cpal::SampleFormat::U16 => output::<u16>(&device, &config, samples, cancel.clone(), done.clone(), failed.clone()),
        _ => return Err("Unsupported speech output sample format".into()),
    }?;
    stream.play().map_err(|_| "Could not play Jarvis speech")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done.load(Ordering::Acquire) && !failed.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    if failed.load(Ordering::Acquire) {
        return Err("Speech output device failed".into());
    }
    if Instant::now() >= deadline {
        return Err("Speech playback timed out".into());
    }
    Ok(())
}
