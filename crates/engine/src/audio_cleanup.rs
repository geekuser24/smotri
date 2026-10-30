//! Reviewable, selection-bound audio cleanup. Analysis never edits the project.
use crate::{
    EngineError, Result, Session,
    commands::{CommandSpec, has_seq},
};
use filmcraft_project::{ClipId, ItemId, Project, TrackId, Transcript};
use filmcraft_time::{Tick, TimeRange};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, atomic::Ordering};
mod matching;

fn err(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}
fn lock<T>(v: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    v.lock().unwrap_or_else(|e| e.into_inner())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Take {
    pub text: String,
    pub start: Tick,
    pub end: Tick,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    pub takes: Vec<Take>,
    pub resolved: bool,
    #[serde(default)]
    pub similarity_percent: f64,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Analysis {
    pub ranges: Vec<TimeRange>,
    pub groups: Vec<Group>,
    pub threshold_db: f32,
    pub noise_db: f32,
    pub peak_db: f32,
    #[serde(default)]
    pub similarity_percent: f64,
    #[serde(skip)]
    pub candidates: Vec<Take>,
}
pub struct Review {
    duplicates: bool,
    selection: Vec<ClipId>,
    snapshot: Arc<Project>,
    sequence: ItemId,
    tracks: Vec<TrackId>,
    clips: Vec<ClipId>,
    pub job: u64,
    result: Arc<Mutex<Option<Analysis>>>,
    pub analysis: Option<Analysis>,
    removed: Vec<TimeRange>,
}
fn number(p: &Value, key: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let v = match p.get(key) {
        None => default,
        Some(v) => v.as_f64().ok_or_else(|| err(format!("Invalid {key}")))?,
    };
    if !v.is_finite() || v < min || v > max {
        return Err(err(format!("{key} must be between {min} and {max}")));
    }
    Ok(v)
}
fn merged(mut ranges: Vec<TimeRange>) -> Vec<TimeRange> {
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<TimeRange> = Vec::new();
    for r in ranges {
        if let Some(last) = out.last_mut()
            && r.start <= last.end()
        {
            *last = TimeRange::from_bounds(last.start, last.end().max(r.end()));
        } else {
            out.push(r);
        }
    }
    out
}
fn start(s: &mut Session, p: &Value) -> Result<Value> {
    if s.jobs.iter().any(|j| matches!(j.label.as_str(), "Find duplicate takes" | "Analyze silence") && !j.progress.finished.load(Ordering::Acquire)) {
        return Err(err("Wait for the previous analysis to stop"));
    }
    if s.audio_cleanup.as_ref().is_some_and(|r| s.jobs.iter().any(|j| j.id == r.job && !j.progress.finished.load(Ordering::Acquire))) {
        return Err(err("Analysis is already running"));
    }
    let similarity = number(p, "similarityPercent", matching::DEFAULT_SIMILARITY, 50., 100.)?;
    let threshold = number(p, "thresholdDb", -42., -80., -10.)? as f32;
    let minimum = number(p, "minimumMs", 400., 100., 10000.)? as usize;
    let padding = number(p, "paddingMs", 120., 40., 1000.)? as usize;
    let duplicates = p.get("duplicates").and_then(Value::as_bool).unwrap_or(false);
    let protect = duplicates || p.get("protectSpeech").and_then(Value::as_bool).unwrap_or(false);
    let auto = p.get("autoThreshold").and_then(Value::as_bool).unwrap_or(true);
    let recognizer = if protect { Some(s.transcriber.clone().ok_or_else(|| err("Configure ElevenLabs in Settings → Jarvis"))?) } else { None };
    if s.state.selection.is_empty() {
        return Err(err("Select clips first"));
    }
    let mut ids = s.state.selection.clone();
    let sequence = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let mut q = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let links: Vec<_> = ids.iter().filter_map(|id| q.find_item(*id).and_then(|(_, c)| c.link)).collect();
    for c in q.all_tracks().flat_map(|t| &t.items) {
        if c.link.is_some_and(|l| links.contains(&l)) && !ids.contains(&c.id) {
            ids.push(c.id);
        }
    }
    if q.audio_tracks.iter().any(|t| (!t.enabled || t.muted) && t.items.iter().any(|c| ids.contains(&c.id))) {
        return Err(err("Unmute and enable the selected audio tracks before analysis"));
    }
    // The analysis audition contains only selected audio, irrespective of unrelated solo buttons.
    for t in q.all_tracks_mut() {
        t.solo = false;
    }

    let mut tracks = Vec::new();
    let mut spans = Vec::new();
    for id in &ids {
        let (track, c) = q.find_item(*id).ok_or_else(|| err("Selected clip missing"))?;
        if q.all_tracks().any(|t| t.id == track && t.locked) || !c.enabled || c.reverse || c.speed <= 0. || c.frame_hold.is_some() {
            return Err(err("Select enabled, forward-playing clips on unlocked tracks"));
        }
        if !tracks.contains(&track) {
            tracks.push(track);
        }
        spans.push(c.range());
    }
    let excluded: Vec<_> =
        q.all_tracks().filter(|t| tracks.contains(&t.id)).flat_map(|t| t.items.iter()).filter(|c| !ids.contains(&c.id)).map(|c| c.id).collect();
    let spans = merged(spans);
    let begin = spans.first().ok_or_else(|| err("No clips"))?.start;
    let end = spans.last().ok_or_else(|| err("No clips"))?.end();
    let frames = (end - begin).to_units_floor(16000);
    if !(1..=16000 * 60 * 60 * 2).contains(&frames) {
        return Err(err("Select up to two hours of audio"));
    }
    for t in &mut q.audio_tracks {
        t.items.retain(|c| ids.contains(&c.id));
    }
    if q.audio_tracks.iter().all(|t| t.items.is_empty()) {
        return Err(err("Select the audio clip or its linked video and audio"));
    }
    q.settings.sample_rate = 16000;
    let snapshot = s.project.clone();
    let provider = s.media.full_res_provider(snapshot.clone(), s.services.clone()).recording();
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1);
    let job = crate::Job {
        id,
        label: if duplicates { "Find duplicate takes" } else { "Analyze silence" }.into(),
        progress: Default::default(),
        result: Default::default(),
    };
    let progress = job.progress.clone();
    let result = job.result.clone();
    let output: Arc<Mutex<Option<Analysis>>> = Arc::default();
    let out = output.clone();
    let work = move || {
        let run = || -> Result<Analysis> {
            let mut audio = Vec::with_capacity(frames as usize);
            let mut peak = Vec::with_capacity(frames as usize);
            progress.total.store(100, Ordering::Relaxed);
            while audio.len() < frames as usize {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(err("Cancelled"));
                }
                let n = (frames as usize - audio.len()).min(16000);
                let buf = filmcraft_render::mixer::mix_graph(&snapshot, &q, begin.to_units_floor(16000) + audio.len() as i64, n, &provider, None);
                if buf.frames() != n {
                    return Err(err("Incomplete audio render"));
                }
                for i in 0..n {
                    peak.push(buf.channels.iter().filter_map(|c| c.get(i)).map(|v| v.abs()).fold(0f32, f32::max));
                }
                audio.extend(filmcraft_speech::downmix(&buf.channels));
                progress.done.store((audio.len() * 60 / frames as usize) as u64, Ordering::Relaxed);
                *lock(&progress.status) = "Rendering selected audio".into();
            }
            if !provider.offline_used().is_empty() || audio.iter().chain(&peak).any(|v| !v.is_finite()) {
                return Err(err("Offline or invalid audio; nothing changed"));
            }
            let tr = if let Some(recognizer) = recognizer {
                let tr = recognizer
                    .transcribe(&audio, &Default::default(), &mut |f, status| {
                        progress.done.store(60 + (f.clamp(0., 1.) * 35.) as u64, Ordering::Relaxed);
                        *lock(&progress.status) = status.into();
                        !progress.cancel.load(Ordering::Relaxed)
                    })
                    .map_err(|e| err(e.to_string()))?;
                if tr.words.len() > 50000
                    || tr.words.iter().any(|w| w.text.len() > 4096 || w.start < Tick::ZERO || w.end < w.start || w.end > Tick::from_units(frames, 16000))
                {
                    return Err(err("Invalid transcription timing or size; nothing changed"));
                }
                if tr.words.is_empty() {
                    return Err(err("No speech recognized; nothing changed"));
                }
                Some(tr)
            } else {
                None
            };
            let mut analysis = analyze(&peak, tr.as_ref(), begin, q.settings.frame_rate.frame_duration(), &spans, threshold, auto, minimum, padding);
            if duplicates {
                analysis.ranges.clear();
                analysis.similarity_percent = similarity;
                analysis.candidates = find_takes(
                    tr.as_ref().ok_or_else(|| err("No transcript"))?,
                    &peak,
                    begin,
                    q.settings.frame_rate.frame_duration(),
                    &spans,
                    analysis.threshold_db,
                );
                analysis.groups = matching::group(&analysis.candidates, similarity)?;
            }
            Ok(analysis)
        };
        match run() {
            Ok(a) => {
                *lock(&out) = Some(a);
                *lock(&result) =
                    Some(Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: 0., bytes: 0, render_fps: 0., extra_files: vec![] }));
            }
            Err(e) => {
                *lock(&result) = Some(Err(e.to_string()));
            }
        }
        progress.done.store(100, Ordering::Relaxed);
        progress.finished.store(true, Ordering::Release);
    };
    let guarded = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), work);
    std::thread::Builder::new().name("Audio cleanup".into()).spawn(guarded).map_err(|e| err(e.to_string()))?;
    s.audio_cleanup = Some(Review {
        duplicates,
        selection: s.state.selection.clone(),
        snapshot: s.project.clone(),
        sequence,
        tracks,
        clips: excluded,
        job: id,
        result: output,
        analysis: None,
        removed: vec![],
    });
    s.jobs.push(job);
    Ok(json!({"job":id}))
}
fn db(v: f32) -> f32 {
    20. * v.max(0.000001).log10()
}
#[allow(clippy::too_many_arguments)]
fn analyze(
    audio: &[f32],
    tr: Option<&Transcript>,
    begin: Tick,
    frame: Tick,
    spans: &[TimeRange],
    threshold: f32,
    auto: bool,
    minimum: usize,
    padding: usize,
) -> Analysis {
    let peaks: Vec<f32> = audio.chunks(160).map(|c| c.iter().copied().fold(0f32, f32::max)).collect();
    let mut sorted = peaks.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = db(sorted.get(sorted.len() / 10).copied().unwrap_or(0.));
    let high = db(sorted.get(sorted.len() * 9 / 10).copied().unwrap_or(0.));
    let threshold = if auto { (noise + 6.).min(high - 18.).clamp(-65., -30.) } else { threshold };
    let mut keep: Vec<bool> = peaks.iter().map(|p| db(*p) > threshold).collect();
    if let Some(tr) = tr {
        for w in &tr.words {
            let a = (w.start.to_units_floor(100).max(0) as usize).saturating_sub(15);
            let b = (w.end.to_units_floor(100).max(0) as usize).saturating_add(20).min(keep.len());
            for v in keep.get_mut(a.min(b)..b).into_iter().flatten() {
                *v = true;
            }
        }
    }
    let mut ranges = vec![];
    let mut i = 0;
    while i < keep.len() {
        if keep[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < keep.len() && !keep[i] {
            i += 1;
        }
        if (i - a) * 10 < minimum + padding * 2 {
            continue;
        }
        let from = begin + Tick::from_units((a * 10 + padding) as i64, 1000);
        let to = begin + Tick::from_units((i * 10 - padding) as i64, 1000);
        for span in spans {
            let a = from.max(span.start);
            let b = to.min(span.end());
            let fd = frame.0.max(1);
            let a = Tick(a.0.saturating_add(fd - 1) / fd * fd);
            let b = Tick(b.0 / fd * fd);
            if b > a && (b - a).to_units_floor(1000) >= minimum as i64 {
                ranges.push(TimeRange::from_bounds(a, b));
            }
        }
    }
    Analysis { ranges: merged(ranges), groups: vec![], threshold_db: threshold, noise_db: noise, peak_db: high, ..Default::default() }
}
fn find_takes(tr: &Transcript, audio: &[f32], begin: Tick, frame: Tick, spans: &[TimeRange], threshold: f32) -> Vec<Take> {
    let mut phrases = vec![];
    let mut first = 0;
    for i in 0..tr.words.len().min(50000) {
        let w = &tr.words[i];
        let end = i + 1 == tr.words.len() || tr.words.get(i + 1).is_some_and(|n| (n.start - w.end).to_units_floor(1000) > 450);
        if !end {
            continue;
        }
        let words = &tr.words[first..=i];
        first = i + 1;
        if words.len() < 3 || words.len() > 100 {
            continue;
        }
        let start = words[0].start;
        let finish = w.end;
        // Search outward from padded speech boundaries for a quiet 10ms window, never inward.
        let edge = |time: Tick, before: bool| {
            let sample = time.to_units_floor(16000).max(0) as usize;
            let mut found = None;
            for step in 0..=25 {
                let offset = 1600 + step * 160;
                let at = if before { sample.saturating_sub(offset) } else { sample.saturating_add(offset).min(audio.len()) };
                let a = at.saturating_sub(80);
                let b = at.saturating_add(80).min(audio.len());
                if a < b && audio[a..b].iter().all(|v| db(*v) < threshold) {
                    found = Some(at);
                    break;
                }
            }
            found.map(|n| begin + Tick::from_units(n as i64, 16000))
        };
        let (Some(a), Some(b)) = (edge(start, true), edge(finish, false)) else {
            continue;
        };
        let fd = frame.0.max(1);
        let a = Tick(a.0 / fd * fd);
        let b = Tick(b.0.saturating_add(fd - 1) / fd * fd);
        // No neighboring words may be swallowed by waveform-adjusted cut boundaries.
        if tr.words.iter().enumerate().any(|(j, w)| (j < i + 1 - words.len() || j > i) && begin + w.start < b && begin + w.end > a) {
            continue;
        }
        if !spans.iter().any(|r| a >= r.start && b <= r.end()) {
            continue;
        }
        phrases.push(Take { text: words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "), start: a, end: b });
        if phrases.len() >= 2000 {
            break;
        }
    }
    phrases
}
/// Reuse safe waveform-bounded candidates; never send audio to the transcriber again.
fn regroup(s: &mut Session, p: &Value) -> Result<Value> {
    let minimum = number(p, "similarityPercent", matching::DEFAULT_SIMILARITY, 50., 100.)?;
    poll(s);
    let r = s.audio_cleanup.as_mut().ok_or_else(|| err("Analyze selected clips first"))?;
    if !r.duplicates || !Arc::ptr_eq(&r.snapshot, &s.project) || s.state.active_sequence != Some(r.sequence) || s.state.selection != r.selection {
        return Err(err("Project changed; analyze duplicate takes again"));
    }
    if !r.removed.is_empty() {
        return Err(err("Takes have already been removed; analyze again before changing similarity"));
    }
    let a = r.analysis.as_mut().ok_or_else(|| err("Analysis is not complete"))?;
    let groups = matching::group(&a.candidates, minimum)?;
    a.groups = groups;
    a.similarity_percent = minimum;
    Ok(json!({"groups":a.groups.len(), "similarityPercent":minimum}))
}

pub fn poll(s: &mut Session) {
    if let Some(r) = s.audio_cleanup.as_mut()
        && r.analysis.is_none()
    {
        r.analysis = lock(&r.result).take();
    }
}
pub fn inspect(s: &mut Session, _: &Value) -> Result<Value> {
    poll(s);
    let Some(r) = s.audio_cleanup.as_ref() else { return Ok(Value::Null) };
    let job = s.jobs.iter().find(|j| j.id == r.job);
    let error = job.and_then(|j| lock(&j.result).as_ref().and_then(|v| v.as_ref().err().cloned()));
    Ok(
        json!({"canRegroup":r.duplicates && r.removed.is_empty() && r.analysis.is_some(),"duplicates":r.duplicates,"job":r.job,"analysis":r.analysis,"error":error,"finished":job.is_none_or(|j|j.progress.finished.load(Ordering::Acquire)),"progress":job.map(|j|j.progress.done.load(Ordering::Relaxed)),"valid":Arc::ptr_eq(&r.snapshot,&s.project)&&s.state.active_sequence==Some(r.sequence)&&s.state.selection==r.selection}),
    )
}
fn mapped(t: Tick, removed: &[TimeRange]) -> Tick {
    t - removed.iter().filter(|r| r.end() <= t).map(|r| r.duration).fold(Tick::ZERO, |a, b| a + b)
}
fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    poll(s);
    let review = s.audio_cleanup.as_ref().ok_or_else(|| err("Analyze selected clips first"))?;
    if !Arc::ptr_eq(&review.snapshot, &s.project) || s.state.active_sequence != Some(review.sequence) || s.state.selection != review.selection {
        return Err(err("Project changed; analyze again before cutting"));
    }
    let analysis = review.analysis.as_ref().ok_or_else(|| err("Analysis is not complete"))?;
    let group = if let Some(g) = p.get("group") { Some(g.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| err("Invalid group"))?) } else { None };
    let original = if let Some(g) = group {
        let group = analysis.groups.get(g).filter(|g| !g.resolved).ok_or_else(|| err("Group already resolved or missing"))?;
        let keep = p
            .get("keep")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|i| *i < group.takes.len())
            .ok_or_else(|| err("Choose a take to keep"))?;
        group.takes.iter().enumerate().filter(|(i, _)| *i != keep).map(|(_, t)| TimeRange::from_bounds(t.start, t.end)).collect()
    } else {
        analysis.ranges.clone()
    };
    let original = merged(original);
    let ranges: Vec<_> = original.iter().map(|r| TimeRange::from_bounds(mapped(r.start, &review.removed), mapped(r.end(), &review.removed))).collect();
    let tracks = review.tracks.clone();
    let clips = review.clips.clone();
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if seq.all_tracks().any(|t| tracks.contains(&t.id) && t.locked) {
        return Err(err("Unlock the selected tracks"));
    }
    // Never cut another clip that the user did not select (linked partners are included).
    if seq
        .all_tracks()
        .filter(|t| tracks.contains(&t.id))
        .flat_map(|t| &t.items)
        .any(|c| clips.contains(&c.id) && ranges.iter().any(|r| c.start < r.end() && c.end() > r.start))
    {
        return Err(err("A cut overlaps an unselected clip; select it too or narrow the selection"));
    }
    s.edit_sequence("Audio cleanup", |q, ctx, _| {
        let saved: Vec<_> = q.all_tracks().map(|t| (t.id, t.sync_lock)).collect();
        let captions: Vec<_> = q.caption_tracks.iter().map(|t| t.sync_lock).collect();
        for t in q.all_tracks_mut() {
            t.sync_lock = false;
        }
        for t in &mut q.caption_tracks {
            t.sync_lock = false;
        }
        for r in ranges.iter().rev() {
            filmcraft_edit::extract(q, &tracks, *r, ctx);
        }
        for t in q.all_tracks_mut() {
            t.sync_lock = saved.iter().find(|(id, _)| *id == t.id).is_some_and(|(_, v)| *v);
        }
        for (t, v) in q.caption_tracks.iter_mut().zip(captions) {
            t.sync_lock = v;
        }
        // 5 ms edge fades prevent clicks without modifying existing volume automation.
        let mut deleted = Tick::ZERO;
        let mut joins = vec![];
        for r in &ranges {
            joins.push(r.start - deleted);
            deleted += r.duration;
        }
        for t in &mut q.audio_tracks {
            if !tracks.contains(&t.id) {
                continue;
            }
            for c in &mut t.items {
                let fi = joins.contains(&c.start);
                let fo = joins.contains(&c.end());
                if !fi && !fo {
                    continue;
                }
                filmcraft_project::cleanup_fades::add(c, fi, fo);
            }
        }
        Ok(())
    })?;
    if let Some(r) = s.audio_cleanup.as_mut() {
        r.snapshot = s.project.clone();
        r.selection = s.state.selection.clone();
        r.removed.extend(original);
        r.removed = merged(std::mem::take(&mut r.removed));
        if let Some(a) = r.analysis.as_mut() {
            if let Some(g) = group {
                if let Some(g) = a.groups.get_mut(g) {
                    g.resolved = true;
                }
            } else {
                a.ranges.clear();
            }
        }
    }
    Ok(json!({"removed":ranges.len()}))
}
fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let r = s.audio_cleanup.as_ref().ok_or_else(|| err("Analyze first"))?;
    if !Arc::ptr_eq(&r.snapshot, &s.project) || s.state.active_sequence != Some(r.sequence) || s.state.selection != r.selection {
        return Err(err("Project changed; analyze again"));
    }
    let g = p.get("group").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| err("Invalid group"))?;
    let t = p.get("take").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| err("Invalid take"))?;
    let take = r.analysis.as_ref().and_then(|a| a.groups.get(g)).filter(|g| !g.resolved).and_then(|g| g.takes.get(t)).ok_or_else(|| err("Take missing"))?;
    Ok(json!({"start":mapped(take.start,&r.removed),"end":mapped(take.end,&r.removed)}))
}
fn cancel(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(r) = s.audio_cleanup.take()
        && let Some(j) = s.jobs.iter().find(|j| j.id == r.job)
    {
        j.progress.cancel.store(true, Ordering::Relaxed);
    }
    Ok(Value::Null)
}
pub fn commands() -> Vec<CommandSpec> {
    [
    ("audioCleanup.analyze","Analyze selected audio",start as fn(&mut Session,&Value)->Result<Value>),
    ("audioCleanup.inspect","Inspect audio cleanup",inspect), ("audioCleanup.apply","Apply audio cleanup",apply),
    ("audioCleanup.regroup","Regroup duplicate takes",regroup),("audioCleanup.preview","Preview take",preview),("audioCleanup.cancel","Cancel audio cleanup",cancel),
].into_iter().map(|(id,label,run)|CommandSpec{id,label,menu:&[],shortcut:None,params:r#"{"similarityPercent":number?,"thresholdDb":number?,"minimumMs":number?,"paddingMs":number?,"autoThreshold":bool?,"protectSpeech":bool?,"duplicates":bool?,"group":integer?,"keep":integer?,"take":integer?}"#,enabled:has_seq,run,journal:false}).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::Word;
    fn ms(n: i64) -> Tick {
        Tick::from_units(n, 1000)
    }
    #[test]
    fn silence_respects_quiet_voice_padding_and_selected_spans() {
        let mut audio = vec![0f32; 16000 * 4];
        audio[0..8000].fill(0.05);
        audio[32000..40000].fill(0.003);
        let spans = vec![TimeRange::from_bounds(ms(0), ms(3000))];
        let a = analyze(&audio, None, Tick::ZERO, ms(10), &spans, -60., false, 200, 120);
        assert!(!a.ranges.is_empty());
        assert!(a.ranges.iter().all(|r| r.start >= ms(620) && r.end() <= ms(3000)));
        assert!(!a.ranges.iter().any(|r| r.start < ms(2500) && r.end() > ms(2000)));
        let tr = Transcript { words: vec![Word::new("quiet", ms(1000), ms(1300))], ..Default::default() };
        let a = analyze(&audio, Some(&tr), Tick::ZERO, ms(10), &spans, -30., false, 200, 120);
        assert!(!a.ranges.iter().any(|r| r.start < ms(1500) && r.end() > ms(850)));
    }
    #[test]
    fn duplicates_are_ordered_and_waveform_bounded() {
        let mut tr = Transcript::default();
        let mut audio = vec![0f32; 16000 * 6];
        for offset in [500, 3000] {
            for (i, text) in ["это", "наш", "новый", "дубль."].iter().enumerate() {
                let a = offset + i as i64 * 300;
                tr.words.push(Word::new(*text, ms(a), ms(a + 250)));
                audio[(a * 16) as usize..((a + 250) * 16) as usize].fill(0.2);
            }
        }
        let spans = [TimeRange::from_bounds(ms(0), ms(6000))];
        let takes = find_takes(&tr, &audio, Tick::ZERO, ms(40), &spans, -42.);
        let g = matching::group(&takes, 80.).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].takes.len(), 2);
        assert!(g[0].takes[0].start <= ms(400));
        assert!(g[0].takes[0].end >= ms(1750));
        assert!(find_takes(&tr, &vec![0.2; audio.len()], Tick::ZERO, ms(40), &spans, -42.).is_empty());
    }
    #[test]
    fn sentence_punctuation_does_not_split_a_continuous_take() {
        let mut tr = Transcript::default();
        let mut audio = vec![0f32; 16000 * 6];
        for (offset, words) in [(500, ["Это.", "Наш,", "новый", "дубль!"]), (3000, ["это", "наш", "новый", "дубль"])] {
            for (i, text) in words.iter().enumerate() {
                let a = offset + i as i64 * 300;
                tr.words.push(Word::new(*text, ms(a), ms(a + 250)));
                audio[(a * 16) as usize..((a + 250) * 16) as usize].fill(0.2);
            }
        }
        let takes = find_takes(&tr, &audio, Tick::ZERO, ms(40), &[TimeRange::from_bounds(ms(0), ms(6000))], -42.);
        let groups = matching::group(&takes, 100.).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].takes.len(), 2);
        assert_eq!(groups[0].similarity_percent, 100.);
    }
    fn review(s: &mut Session, analysis: Analysis) {
        let q = s.active_sequence().unwrap();
        let c = q.video_tracks[0].items[0].id;
        let ids = crate::commands::with_links(s, &[c]);
        let tracks: Vec<_> = q.all_tracks().filter(|t| t.items.iter().any(|c| ids.contains(&c.id))).map(|t| t.id).collect();
        let excluded = q.all_tracks().filter(|t| tracks.contains(&t.id)).flat_map(|t| &t.items).filter(|c| !ids.contains(&c.id)).map(|c| c.id).collect();
        s.audio_cleanup = Some(Review {
            duplicates: !analysis.groups.is_empty(),
            selection: s.state.selection.clone(),
            snapshot: s.project.clone(),
            sequence: s.state.active_sequence.unwrap(),
            tracks,
            clips: excluded,
            job: 0,
            result: Arc::default(),
            analysis: Some(analysis),
            removed: vec![],
        });
    }
    #[test]
    fn selected_cleanup_preserves_other_tracks_and_undo() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let before = s.project.clone();
        let unrelated = s.active_sequence().unwrap().audio_tracks[1].clone();
        review(&mut s, Analysis { ranges: vec![TimeRange::from_bounds(ms(1000), ms(2000))], ..Default::default() });
        s.execute("audioCleanup.apply", json!({})).unwrap();
        assert_eq!(s.active_sequence().unwrap().audio_tracks[1], unrelated);
        assert_ne!(s.project, before);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        assert!(s.execute("audioCleanup.apply", json!({})).is_err());
    }
    #[test]
    fn duplicate_pages_remap_and_each_decision_is_undoable() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let take = |a, b| Take { text: "same take words".into(), start: ms(a), end: ms(b) };
        review(
            &mut s,
            Analysis {
                groups: vec![
                    Group { takes: vec![take(100, 500), take(1000, 1400)], resolved: false, similarity_percent: 100. },
                    Group { takes: vec![take(2000, 2400), take(3000, 3400)], resolved: false, similarity_percent: 100. },
                ],
                ..Default::default()
            },
        );
        s.execute("audioCleanup.apply", json!({"group":0,"keep":0})).unwrap();
        let first = s.project.clone();
        let v = s.execute("audioCleanup.preview", json!({"group":1,"take":0})).unwrap();
        assert_eq!(v["start"], json!(ms(1600)));
        assert!(s.execute("audioCleanup.apply", json!({"group":0,"keep":1})).is_err());
        s.execute("audioCleanup.apply", json!({"group":1,"keep":1})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, first);
    }
    #[test]
    fn threshold_regroups_cached_takes_without_transcription_or_project_edits() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s.transcriber = None;
        let takes = vec![
            Take { text: "сегодня мы покажем новый редактор".into(), start: ms(100), end: ms(500) },
            Take { text: "сегодня мы покажем крутой редактор".into(), start: ms(1000), end: ms(1400) },
        ];
        review(&mut s, Analysis { candidates: takes, similarity_percent: 100., ..Default::default() });
        s.audio_cleanup.as_mut().unwrap().duplicates = true;
        let before = s.project.clone();
        let jobs = s.jobs.len();
        for value in [json!(-1), json!(101), json!("80"), Value::Null, json!([])] {
            assert!(s.execute("audioCleanup.regroup", json!({"similarityPercent":value})).is_err());
        }
        assert_eq!(s.execute("audioCleanup.regroup", json!({"similarityPercent":80})).unwrap()["groups"], 1);
        assert_eq!(s.execute("audioCleanup.regroup", json!({"similarityPercent":81})).unwrap()["groups"], 0);
        assert_eq!(s.execute("audioCleanup.regroup", json!({"similarityPercent":80})).unwrap()["groups"], 1);
        assert_eq!(s.jobs.len(), jobs);
        assert!(Arc::ptr_eq(&s.project, &before));
        let p = s.execute("audioCleanup.preview", json!({"group":0,"take":1})).unwrap();
        assert_eq!(p["start"], json!(ms(1000)));
        s.execute("audioCleanup.apply", json!({"group":0,"keep":1})).unwrap();
        let after = s.project.clone();
        assert!(!Arc::ptr_eq(&after, &before));
        assert!(s.execute("audioCleanup.regroup", json!({"similarityPercent":90})).is_err());
        assert_eq!(s.project, after);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        assert!(s.execute("audioCleanup.regroup", json!({"similarityPercent":80})).is_err());
    }
    #[test]
    fn local_analysis_needs_no_transcriber_and_includes_linked_audio() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
        s.state.selection = vec![clip];
        s.state.linked_selection = false;
        s.transcriber = None;
        let before = s.project.clone();
        s.execute("audioCleanup.analyze", json!({})).unwrap();
        let until = web_time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let v = inspect(&mut s, &json!({})).unwrap();
            if v["finished"] == true {
                assert!(v["error"].is_null(), "{v}");
                assert!(v["analysis"].is_object());
                break;
            }
            assert!(web_time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(s.project, before);
        assert_eq!(s.audio_cleanup.as_ref().unwrap().tracks.len(), 2);
    }
    #[test]
    fn hostile_params_and_stale_analysis_never_edit() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let before = s.project.clone();
        for p in [
            json!({"thresholdDb":100}),
            json!({"minimumMs":-1}),
            json!({"paddingMs":"bad"}),
            json!({"similarityPercent":49}),
            json!({"similarityPercent":101}),
            json!({"similarityPercent":"80"}),
            json!({"similarityPercent":null}),
        ] {
            assert!(s.execute("audioCleanup.analyze", p).is_err());
        }
        review(&mut s, Analysis::default());
        for p in [json!({"group":-1}), json!({"group":999999,"keep":0})] {
            assert!(s.execute("audioCleanup.apply", p).is_err());
        }
        assert_eq!(s.project, before);
    }
}
