//! Batch Scribe v2; independent of the microphone/realtime connection.
use filmcraft_project::{Transcript, transcript::Word};
use filmcraft_speech::{Options, ProgressFn, SpeechError, Transcriber};
pub struct Scribe;
impl Transcriber for Scribe {
    fn id(&self) -> String {
        "elevenlabs-scribe-v2".into()
    }
    fn transcribe(&self, audio: &[f32], _: &Options, progress: ProgressFn) -> Result<Transcript, SpeechError> {
        let fail = |s: &str| SpeechError::Io(s.into());
        if audio.len() > 16000 * 60 * 60 * 2 {
            return Err(fail("Scribe: maximum duration is two hours"));
        }
        if !progress(0.05, "Scribe v2: uploading audio") {
            return Err(SpeechError::Cancelled);
        }
        let key = crate::credentials::load(false).ok_or_else(|| fail("Configure the shared ElevenLabs key in Settings"))?;
        let boundary = "SmotriScribeAudioBoundary2026";
        let mut body = Vec::new();
        for (k, v) in [("model_id", "scribe_v2"), ("timestamps_granularity", "word"), ("tag_audio_events", "false"), ("diarize", "false")] {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"timeline.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(&filmcraft_media::wav::write_wav16(audio, 1, 16000));
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::Rustls)
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls_rustcrypto::provider()))
                    .build(),
            )
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build()
            .into();
        let mut response = agent
            .post("https://api.elevenlabs.io/v1/speech-to-text")
            .header("xi-api-key", &key)
            .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send(&body)
            .map_err(|e| match e {
                ureq::Error::StatusCode(code) => SpeechError::Io(format!("Scribe v2: HTTP {code}")),
                ureq::Error::RedirectFailed => fail("Scribe v2: service redirected the upload; check regional availability or VPN"),
                ureq::Error::Timeout(_) => fail("Scribe v2: connection timed out"),
                ureq::Error::HostNotFound => fail("Scribe v2: DNS lookup failed"),
                ureq::Error::Io(e) => SpeechError::Io(format!("Scribe v2: network error ({:?})", e.kind())),
                ureq::Error::Http(_) => fail("Scribe v2: invalid request headers; re-enter the API key"),
                _ => fail("Scribe v2: TLS or connection failed"),
            })?;
        if !progress(0.95, "Scribe v2: reading word timestamps") {
            return Err(SpeechError::Cancelled);
        }
        let bytes = response.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().map_err(|_| fail("Scribe v2: response too large or unreadable"))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| fail("Scribe v2: invalid response"))?;
        let words = value.get("words").and_then(|v| v.as_array()).ok_or_else(|| fail("Scribe v2: no word timestamps"))?;
        let duration = audio.len() as f64 / 16000.;
        let mut tr = Transcript { source: self.id(), language: value["language_code"].as_str().unwrap_or("auto").into(), ..Default::default() };
        for w in words {
            if w["type"].as_str() != Some("word") {
                continue;
            }
            let text = w["text"].as_str().ok_or_else(|| fail("Scribe v2: invalid word"))?;
            let a = w["start"].as_f64().filter(|n| n.is_finite() && *n >= 0. && *n <= duration).ok_or_else(|| fail("Scribe v2: invalid start time"))?;
            let b = w["end"].as_f64().filter(|n| n.is_finite() && *n >= a && *n <= duration + 0.1).ok_or_else(|| fail("Scribe v2: invalid end time"))?;
            tr.words.push(Word::new(text, filmcraft_speech::seconds_tick(a), filmcraft_speech::seconds_tick(b.min(duration))));
        }
        tr.normalize();
        Ok(tr)
    }
}
