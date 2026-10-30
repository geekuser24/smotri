//! Background rendered-audio transcription and conservative, waveform-checked pause removal.
use crate::{
    EngineError, Result, Session,
    commands::{CommandSpec, has_seq},
};
use filmcraft_project::{ItemId, Project, Transcript};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};
fn err(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
pub struct Pending {
    job: u64,
    snapshot: Arc<Project>,
    sequence: ItemId,
    start: Tick,
    remove: bool,
    result: Arc<Mutex<Option<Output>>>,
}
struct Output {
    transcript: Transcript,
    ranges: Vec<TimeRange>,
    wav: PathBuf,
    json: PathBuf,
}
fn transcribe(s: &mut Session, p: &Value) -> Result<Value> {
    start(s, p, false)
}
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    start(s, p, true)
}
fn start(s: &mut Session, p: &Value, remove: bool) -> Result<Value> {
    if !s.voice_transcripts.is_empty() {
        return Err(err("A Jarvis audio analysis is already running"));
    }
    let recognizer = s.transcriber.clone().ok_or_else(|| err("Configure Scribe in Settings > Jarvis"))?;
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let mut q = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let mut all_layers = q.clone();
    all_layers.settings.sample_rate = 16000;
    if remove && q.all_tracks().any(|t| t.locked) {
        return Err(err("Unlock all tracks before removing pauses, so audio and video stay synchronized"));
    }
    if remove && q.audio_tracks.iter().flat_map(|t| &t.items).any(|c| c.reverse || c.speed <= 0.) {
        return Err(err("Pause removal currently requires forward audio clips"));
    }
    let scope = p.get("scope").and_then(Value::as_str).unwrap_or("timeline");
    if !matches!(scope, "timeline" | "clip") {
        return Err(err("scope must be timeline or clip"));
    }
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("all");
    if !matches!(mode, "all" | "voice") {
        return Err(err("mode must be all or voice"));
    }
    let (begin, end) = if scope == "clip" {
        let ids = crate::voice_edit::targets(s, p, false)?;
        if ids.len() != 1 {
            return Err(err("Select one clip for clip transcription"));
        }
        let (_, c) = q.find_item(ids[0]).ok_or_else(|| err("clip missing"))?;
        let range = (c.start, c.end());
        let item = c.item;
        let link = c.link;
        let id = c.id;
        // Include linked audio; never send unrelated tracks for a clip request.
        for t in &mut q.audio_tracks {
            t.items.retain(|c| c.id == id || (link.is_some() && c.link == link) || (c.item == item && c.start < range.1 && c.end() > range.0));
        }
        range
    } else {
        (Tick::ZERO, q.duration())
    };
    let frames = (end - begin).to_units_floor(16000);
    if !(1..=16000 * 60 * 60 * 2).contains(&frames) {
        return Err(err("Analysis requires audio between one sample and two hours"));
    }
    if q.audio_tracks.iter().all(|t| t.items.is_empty()) {
        return Err(err("There is no audio to transcribe"));
    }
    q.settings.sample_rate = 16000;
    let snapshot = s.project.clone();
    let provider = s.media.full_res_provider(snapshot.clone(), s.services.clone()).recording();
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1);
    let job = crate::Job {
        id,
        label: if remove { "Jarvis: remove pauses" } else { "Jarvis: Scribe v2 transcription" }.into(),
        progress: Default::default(),
        result: Default::default(),
    };
    let prog = job.progress.clone();
    let res = job.result.clone();
    let output: Arc<Mutex<Option<Output>>> = Arc::default();
    let out = output.clone();
    let base = s
        .path
        .as_ref()
        .and_then(|p| std::path::Path::new(p).parent().map(|p| p.join("Transcripts")))
        .or_else(|| crate::autosave::default_data_dir().map(|p| p.join("Transcripts")))
        .ok_or_else(|| err("No transcript output directory"))?;
    let voice_only = mode == "voice";
    let run = move || {
        let work = || -> Result<Output> {
            let mut audio = Vec::with_capacity(frames as usize);
            let mut waveform = Vec::with_capacity(frames as usize);
            prog.total.store(100, Ordering::Relaxed);
            let offset = begin.to_units_floor(16000);
            while audio.len() < frames as usize {
                if prog.cancel.load(Ordering::Relaxed) {
                    return Err(err("Cancelled; project unchanged"));
                }
                let n = (frames as usize - audio.len()).min(16000);
                let buf = filmcraft_render::mixer::mix_graph(&snapshot, &q, offset + audio.len() as i64, n, &provider, None);
                let check = if remove && !voice_only {
                    filmcraft_render::mixer::mix_graph(&snapshot, &all_layers, offset + audio.len() as i64, n, &provider, None)
                } else {
                    buf.clone()
                };
                for i in 0..n {
                    waveform.push(check.channels.iter().filter_map(|c| c.get(i)).map(|v| v.abs()).fold(0.0f32, f32::max));
                }
                audio.extend(filmcraft_speech::downmix(&buf.channels));
                if buf.frames() != n {
                    return Err(err("Audio render returned an incomplete buffer"));
                }
                prog.done.store((audio.len() * 30 / frames as usize) as u64, Ordering::Relaxed);
                *lock(&prog.status) = "Rendering timeline audio".into();
            }
            if !provider.offline_used().is_empty() {
                return Err(err("Some audio is offline; restore the media before analysing pauses"));
            }
            if audio.iter().any(|v| !v.is_finite()) {
                return Err(err("Non-finite audio samples"));
            }
            let tr = recognizer
                .transcribe(&audio, &Default::default(), &mut |f, status| {
                    prog.done.store(30 + (f.clamp(0., 1.) * 60.) as u64, Ordering::Relaxed);
                    *lock(&prog.status) = status.into();
                    !prog.cancel.load(Ordering::Relaxed)
                })
                .map_err(|e| err(e.to_string()))?;
            if tr.words.is_empty() {
                return Err(err("No speech recognized; nothing was cut"));
            }
            let ranges = if remove { pause_ranges(&waveform, &tr, begin, q.settings.frame_rate.frame_duration(), voice_only) } else { Vec::new() };
            std::fs::create_dir_all(&base).map_err(|e| err(e.to_string()))?;
            let now: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
            let stem = format!("Transcript-{}-{id}", now.format("%Y-%m-%d_%H-%M-%S%.3f"));
            let wav = base.join(format!("{stem}.wav"));
            let path = base.join(format!("{stem}.json"));
            std::fs::write(&wav, filmcraft_media::wav::write_wav16(&audio, 1, 16000)).map_err(|e| err(e.to_string()))?;
            let words: Vec<_> = tr
                .words
                .iter()
                .map(|w| json!({"text":w.text,"startMs":(begin+w.start).to_units_floor(1000),"endMs":(begin+w.end).to_units_floor(1000)}))
                .collect();
            let data = json!({"schema":1,"createdAt":now.to_rfc3339(),"model":tr.source,"sequence":seq,"timelineStartMs":begin.to_units_floor(1000),"words":words,"transcript":tr,"audio":wav.file_name().map(|n|n.to_string_lossy()),"proposedRemovalRangesMs":ranges.iter().map(|r|json!([r.start.to_units_floor(1000),r.end().to_units_floor(1000)])).collect::<Vec<_>>()});
            std::fs::write(&path, serde_json::to_vec_pretty(&data).map_err(|e| err(e.to_string()))?).map_err(|e| err(e.to_string()))?;
            Ok(Output { transcript: tr, ranges, wav, json: path })
        };
        match work() {
            Ok(output) => {
                let path = output.json.to_string_lossy().into_owned();
                let extra_files = vec![output.wav.to_string_lossy().into_owned()];
                *lock(&out) = Some(output);
                prog.done.store(100, Ordering::Relaxed);
                *lock(&prog.status) = "Analysis complete; applying results".into();
                *lock(&res) = Some(Ok(filmcraft_export::Report { path, frames: 0, seconds: 0., bytes: 0, render_fps: 0., extra_files }));
            }
            Err(e) => {
                *lock(&res) = Some(Err(e.to_string()));
                *lock(&prog.status) = e.to_string();
            }
        }
        prog.finished.store(true, Ordering::Release);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    // Spawn before registering so a spawn failure leaves no stuck job.
    std::thread::Builder::new().name("Jarvis batch transcription".into()).spawn(run).map_err(|e| err(e.to_string()))?;
    s.voice_transcripts.push(Pending { job: id, snapshot: s.project.clone(), sequence: seq, start: begin, remove, result: output });
    s.jobs.push(job);
    Ok(json!({"job":id}))
}
/// Protect 100 ms before / 160 ms after speech. All-layer mode removes only sustained quiet
/// windows; voice-only mode uses waveform energy to extend speech edges by at most 150 ms.
fn pause_ranges(audio: &[f32], tr: &Transcript, start: Tick, frame: Tick, voice_only: bool) -> Vec<TimeRange> {
    let step = 160usize;
    let mut protected = vec![false; audio.len().div_ceil(step)];
    for w in &tr.words {
        let a = (w.start.to_units_floor(16000).max(0) as usize).saturating_sub(1600) / step;
        let b = (w.end.to_units_floor(16000).max(0) as usize).saturating_add(2560).div_ceil(step).min(protected.len());
        for p in protected.get_mut(a.min(b)..b).into_iter().flatten() {
            *p = true;
        }
    }
    let audible: Vec<_> = audio.chunks(step).map(|c| c.iter().any(|v| v.abs() > 0.008)).collect();
    let initial = protected.clone();
    for (i, on) in audible.iter().enumerate() {
        if !voice_only || (i.saturating_sub(15)..(i + 16).min(initial.len())).any(|j| initial[j]) {
            protected[i] |= *on;
        }
    }
    let fd = frame.0.max(1);
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < protected.len() {
        if protected[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < protected.len() && !protected[i] {
            i += 1;
        }
        // Leave 40 ms at each edge even when no words were nearby; skip short pauses.
        if i - a < 40 {
            continue;
        }
        let from = start + Tick((a as i64 * 160 + 640) * TICKS_PER_SECOND / 16000);
        let to = start + Tick(((i as i64 * 160 - 640).min(audio.len() as i64)) * TICKS_PER_SECOND / 16000);
        let from = Tick(from.0.saturating_add(fd - 1) / fd * fd);
        let to = Tick(to.0 / fd * fd);
        if to > from {
            ranges.push(TimeRange::from_bounds(from, to));
        }
    }
    ranges
}
pub(crate) fn poll(s: &mut Session) {
    let mut i = 0;
    while i < s.voice_transcripts.len() {
        let pending = &s.voice_transcripts[i];
        let job = s.jobs.iter().find(|j| j.id == pending.job);
        if job.is_some_and(|j| !j.progress.finished.load(Ordering::Acquire)) {
            i += 1;
            continue;
        }
        let cancelled = job.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        let pending = s.voice_transcripts.remove(i);
        let output = lock(&pending.result).take();
        if cancelled {
            continue;
        }
        let Some(output) = output else {
            if let Some(Err(e)) = s.jobs.iter().find(|j| j.id == pending.job).and_then(|j| lock(&j.result).clone()) {
                s.error_toast("voice.transcribe", e);
            }
            continue;
        };
        let result = apply_output(s, &pending, output);
        if let Err(e) = result {
            if let Some(j) = s.jobs.iter().find(|j| j.id == pending.job) {
                *lock(&j.result) = Some(Err(e.to_string()));
            }
            s.error_toast("voice.transcribe", e.to_string());
        }
    }
}
fn apply_output(s: &mut Session, p: &Pending, o: Output) -> Result<()> {
    if !Arc::ptr_eq(&p.snapshot, &s.project) || s.state.active_sequence != Some(p.sequence) {
        return Err(err(format!("Project changed during analysis; no edits applied. Transcript saved: {}", o.json.display())));
    }
    let n = o.ranges.len();
    s.edit_group("Jarvis: transcription and pause removal", |s| {
        let item = crate::commands::import_streamed(s, &o.wav.to_string_lossy(), None)?;
        s.edit("Save transcript", |pr, state| {
            pr.transcripts.insert(item, Arc::new(o.transcript.clone()));
            let settings = pr.sequence(p.sequence).ok_or(EngineError::NoSequence)?.settings.clone();
            let duration = pr.item(item).and_then(|i| i.as_media()).ok_or_else(|| err("Imported transcript audio is missing"))?.duration();
            let name = o.wav.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Transcript".into());
            let sequence = pr.new_sequence(&name, settings.clone(), 0, 1, None);
            let clip = pr
                .make_track_item(item, filmcraft_project::TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, duration), settings.frame_rate)
                .ok_or_else(|| err("Cannot create transcript audio clip"))?;
            let track = pr.sequence_mut(sequence).and_then(|q| q.audio_tracks.first_mut()).ok_or_else(|| err("Transcript track missing"))?;
            track.items.push(clip);
            state.project_selection = vec![sequence];
            Ok(())
        })?;
        if p.remove && !o.ranges.is_empty() {
            s.edit_sequence("Remove pauses", |q, ctx, _| {
                if q.all_tracks().any(|t| t.locked) {
                    return Err(err("A track is locked"));
                }
                let mut joins = Vec::new();
                let mut removed = Tick::ZERO;
                for r in &o.ranges {
                    joins.push(r.start - removed);
                    removed += r.duration;
                }
                filmcraft_edit::transcript::ripple_delete_ranges(q, o.ranges.clone(), ctx);
                // Independent gain envelopes avoid overwriting the user's Volume automation.
                for t in &mut q.audio_tracks {
                    for c in &mut t.items {
                        let fade_in = joins.contains(&c.start);
                        let fade_out = joins.contains(&c.end());
                        filmcraft_project::cleanup_fades::add(c, fade_in, fade_out);
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    })?;
    s.toast(format!(
        "Scribe v2: {} words, {n} pauses removed. {} (timeline start {} ms)",
        o.transcript.words.len(),
        o.json.display(),
        p.start.to_units_floor(1000)
    ));
    Ok(())
}
pub fn commands() -> Vec<CommandSpec> {
    [
        ("voice.transcribe", "Transcribe with Jarvis", transcribe as fn(&mut Session, &Value) -> Result<Value>),
        ("voice.removePauses", "Remove Pauses with Jarvis", remove as fn(&mut Session, &Value) -> Result<Value>),
    ]
    .into_iter()
    .map(|(id, label, run)| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params: r#"{"scope":"timeline|clip","mode":"all|voice","target":object?}"#,
        enabled: has_seq,
        run,
        journal: true,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_job_imports_transcript_and_undo_restores_project() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let dir = std::env::temp_dir().join(format!("filmcraft-voice-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.path = Some(dir.join("test.fcproj").to_string_lossy().into_owned());
        s.transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber {
            id: "fixed-test".into(),
            transcript: Transcript {
                words: vec![filmcraft_project::transcript::Word::new("hello", Tick(TICKS_PER_SECOND), Tick(TICKS_PER_SECOND * 2))],
                ..Default::default()
            },
        }));
        let before = s.project.clone();
        let undo = s.history.undo.len();
        s.execute("voice.transcribe", json!({"scope":"timeline"})).unwrap();
        let until = web_time::Instant::now() + std::time::Duration::from_secs(60);
        while !s.voice_transcripts.is_empty() && web_time::Instant::now() < until {
            poll(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(s.voice_transcripts.is_empty());
        assert!(lock(&s.jobs.last().unwrap().result).as_ref().unwrap().is_ok(), "{:?}", s.jobs.last().unwrap().to_json());
        assert!(!s.project.transcripts.is_empty());
        assert_eq!(s.history.undo.len(), undo + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ripple_application_is_atomic_and_stale_results_are_rejected() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let dir = std::env::temp_dir().join(format!("filmcraft-voice-ripple-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("transcript.wav");
        std::fs::write(&wav, filmcraft_media::wav::write_wav16(&vec![0.; 16000 * 3], 1, 16000)).unwrap();
        let original = s.project.clone();
        let before_duration = s.active_sequence().unwrap().duration();
        let p = Pending {
            job: 0,
            snapshot: original.clone(),
            sequence: s.state.active_sequence.unwrap(),
            start: Tick::ZERO,
            remove: true,
            result: Default::default(),
        };
        let output = || Output {
            transcript: Transcript::default(),
            ranges: vec![TimeRange::from_bounds(Tick(TICKS_PER_SECOND), Tick(TICKS_PER_SECOND * 2))],
            wav: wav.clone(),
            json: dir.join("transcript.json"),
        };
        apply_output(&mut s, &p, output()).unwrap();
        assert_eq!(s.active_sequence().unwrap().duration(), before_duration - Tick(TICKS_PER_SECOND));
        s.active_sequence().unwrap().check().unwrap();
        assert!(s.active_sequence().unwrap().audio_tracks.iter().flat_map(|t| &t.items).any(|c| c.effects.iter().any(|e| e.effect == "volume_a")));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, original);
        s.project = Arc::new((*s.project).clone());
        let before = s.project.clone();
        assert!(apply_output(&mut s, &p, output()).is_err());
        assert_eq!(s.project, before);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn waveform_preserves_sound_and_speech_padding() {
        let mut audio = vec![0.; 16000 * 4];
        let mut tr = Transcript::default();
        for (a, b) in [(0.3, 0.8), (2.7, 3.2)] {
            tr.words.push(filmcraft_project::transcript::Word::new("word", filmcraft_speech::seconds_tick(a), filmcraft_speech::seconds_tick(b)));
        }
        let frame = Tick(TICKS_PER_SECOND / 25);
        let ranges = pause_ranges(&audio, &tr, Tick::ZERO, frame, false);
        assert!(!ranges.is_empty());
        for r in &ranges {
            for w in &tr.words {
                assert!(r.end() <= w.start - Tick(TICKS_PER_SECOND / 10) || r.start >= w.end + Tick(TICKS_PER_SECOND * 16 / 100));
            }
        }
        audio.fill(0.02);
        assert!(pause_ranges(&audio, &tr, Tick::ZERO, frame, false).is_empty());
        assert!(!pause_ranges(&audio, &tr, Tick::ZERO, frame, true).is_empty());
    }
}
