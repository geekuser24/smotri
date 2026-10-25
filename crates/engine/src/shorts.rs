//! Reviewed short-form suggestions. Analysis never edits a project; creation is one undo step.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use filmcraft_project::{
    ItemId, ParamValue, Project, TrackKind, Transcript,
    caption::{Caption, CaptionAnchor, CaptionFormat, CaptionTrack},
    effect::find_effect,
};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
fn err(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
    pub score: u8,
    pub reason: String,
}
pub trait Analyst: Send + Sync {
    fn configured(&self) -> bool;
    fn save_key(&self, key: &str) -> std::result::Result<(), String>;
    fn analyze(&self, tr: &Transcript, max_seconds: u32, cancel: &AtomicBool) -> std::result::Result<Vec<Proposal>, String>;
}
#[derive(Clone)]
pub struct Analysis {
    pub transcript: Transcript,
    pub proposals: Vec<Proposal>,
}
pub struct Review {
    pub max_seconds: u32,
    pub job: u64,
    pub source: ItemId,
    pub snapshot: Arc<Project>,
    pub result: Arc<Mutex<Option<Analysis>>>,
}
/// Only changes to the analyzed source or its dependencies invalidate the transcript.
/// Saving, creating shorts and undoing their creation must not trigger another paid transcription.
fn context_matches(left: &Arc<Project>, right: &Arc<Project>, source: ItemId) -> bool {
    if Arc::ptr_eq(left, right) {
        return true;
    }
    if left.settings != right.settings || left.luts != right.luts {
        return false;
    }
    let mut pending = vec![source];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if seen.len() > 10_000 {
            return false;
        }
        let (Some(a), Some(b)) = (left.item(id), right.item(id)) else {
            return false;
        };
        if a.kind != b.kind || left.source_graphics.get(&id) != right.source_graphics.get(&id) {
            return false;
        }
        match &a.kind {
            filmcraft_project::ItemKind::Sequence(q) => {
                for c in q.all_tracks().flat_map(|t| &t.items) {
                    if pending.len() > 100_000 {
                        return false;
                    }
                    pending.push(c.item);
                }
            }
            filmcraft_project::ItemKind::Subclip { parent, .. } => pending.push(*parent),
            _ => {}
        }
    }
    true
}
pub fn inspect(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(r) = s.shorts.as_mut()
        && context_matches(&s.project, &r.snapshot, r.source)
    {
        r.snapshot = s.project.clone();
    }
    let Some(r) = &s.shorts else { return Ok(Value::Null) };
    let j = s.jobs.iter().find(|j| j.id == r.job).ok_or_else(|| err("Shorts job missing"))?;
    let a = lock(&r.result);
    let failure = lock(&j.result).as_ref().and_then(|r| r.as_ref().err()).cloned();
    Ok(
        json!({"job":r.job,"duration":r.max_seconds,"source":r.source,"finished":j.progress.finished.load(Ordering::Acquire),"progress":j.progress.done.load(Ordering::Relaxed),"status":lock(&j.progress.status).clone(),"error":failure,"valid":Arc::ptr_eq(&s.project,&r.snapshot),"proposals":a.as_ref().map(|a|&a.proposals)}),
    )
}
pub fn validate(proposals: &[Proposal], duration_ms: i64, max_seconds: u32) -> Result<()> {
    if proposals.is_empty() || proposals.len() > 20 {
        return Err(err("Model must return 1–20 fragments"));
    }
    for p in proposals {
        if p.start_ms < 0
            || p.end_ms <= p.start_ms
            || p.end_ms > duration_ms
            || p.end_ms.saturating_sub(p.start_ms) > i64::from(max_seconds) * 1000
            || p.score > 100
            || p.title.trim().is_empty()
            || p.title.len() > 256
            || p.reason.len() > 2000
        {
            return Err(err("Model returned invalid fragment bounds or score"));
        }
    }
    Ok(())
}

/// Accept only real spoken ranges. Align millisecond model estimates to word boundaries.
fn align_proposals(proposals: &mut [Proposal], tr: &Transcript, max_seconds: u32) -> Result<()> {
    if proposals.len() > 20 || tr.words.len() > 100_000 {
        return Err(err("Analysis is too large"));
    }
    for p in proposals {
        if p.start_ms < 0 || p.end_ms <= p.start_ms {
            return Err(err("Invalid model timestamps"));
        }
        let first = tr
            .words
            .iter()
            .filter(|w| w.end > w.start && w.start >= Tick::ZERO)
            .min_by_key(|w| w.start.to_units_floor(1000).abs_diff(p.start_ms))
            .ok_or_else(|| err("No usable words"))?;
        let last = tr
            .words
            .iter()
            .filter(|w| w.end > w.start && w.end > first.start)
            .min_by_key(|w| w.end.to_units_floor(1000).abs_diff(p.end_ms))
            .ok_or_else(|| err("No usable end word"))?;
        let a = first.start.to_units_floor(1000);
        let b = last.end.to_units_floor(1000);
        if a.abs_diff(p.start_ms) > 500 || b.abs_diff(p.end_ms) > 500 || b - a > i64::from(max_seconds) * 1000 {
            return Err(err("Model timestamps do not match complete words; retry analysis"));
        }
        p.start_ms = a;
        p.end_ms = b;
    }
    Ok(())
}

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    if s.shorts.as_ref().is_some_and(|r| s.jobs.iter().any(|j| j.id == r.job && !j.progress.finished.load(Ordering::Acquire))) {
        return Err(err("Analysis is already running"));
    }
    let source = p.get("sequence").and_then(Value::as_u64).map(ItemId).or(s.state.active_sequence).ok_or(EngineError::NoSequence)?;
    if p.get("sequence").is_some_and(|v| !v.is_null() && !v.is_u64()) {
        return Err(err("Invalid sequence"));
    }
    let mut seq = s.project.sequence(source).ok_or(EngineError::NoSequence)?.clone();
    let max = match p.get("duration") {
        None => 60,
        Some(v) => v.as_u64().ok_or_else(|| err("Invalid duration"))?,
    };
    if ![30, 60, 90].contains(&max) {
        return Err(err("Duration must be 30, 60 or 90 seconds"));
    }
    let frames = seq.duration().to_units_floor(16000);
    if !(16000..=16000 * 7200).contains(&frames) {
        return Err(err("Select a sequence between one second and two hours"));
    }
    if seq.video_tracks.iter().all(|t| t.items.is_empty()) {
        return Err(err("Sequence has no video"));
    }
    if seq.audio_tracks.iter().all(|t| t.items.is_empty()) {
        return Err(err("Sequence has no audio"));
    }
    let transcriber = s.transcriber.clone().ok_or_else(|| err("Configure ElevenLabs in Settings > Vertical Videos"))?;
    let analyst = s.shorts_analyst.clone().filter(|a| a.configured()).ok_or_else(|| err("Configure OpenAI in Settings > Vertical Videos"))?;
    let cached = s
        .shorts
        .as_ref()
        .filter(|r| r.source == source && context_matches(&r.snapshot, &s.project, source))
        .and_then(|r| lock(&r.result).as_ref().map(|a| a.transcript.clone()));
    seq.settings.sample_rate = 16000;
    let snapshot = s.project.clone();
    let worker_snapshot = snapshot.clone();
    let provider = s.media.full_res_provider(snapshot.clone(), s.services.clone()).recording();
    let job = crate::Job {
        id: s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1),
        label: "Vertical Videos: analysis".into(),
        progress: Default::default(),
        result: Default::default(),
    };
    let prog = job.progress.clone();
    let result = job.result.clone();
    let output = Arc::new(Mutex::new(None));
    let out = output.clone();
    let run = move || {
        let work = || -> Result<Analysis> {
            prog.total.store(100, Ordering::Relaxed);
            let tr = if let Some(tr) = cached {
                tr
            } else {
                let mut audio = Vec::with_capacity(frames as usize);
                while audio.len() < frames as usize {
                    if prog.cancel.load(Ordering::Acquire) {
                        return Err(err("Cancelled"));
                    }
                    let n = (frames as usize - audio.len()).min(16000);
                    let buf = filmcraft_render::mixer::mix_graph(&worker_snapshot, &seq, audio.len() as i64, n, &provider, None);
                    if buf.frames() != n {
                        return Err(err("Incomplete audio render"));
                    }
                    audio.extend(filmcraft_speech::downmix(&buf.channels));
                    prog.done.store((audio.len() * 30 / frames as usize) as u64, Ordering::Relaxed);
                    *lock(&prog.status) = "Rendering sequence audio".into();
                }
                if !provider.offline_used().is_empty() || audio.iter().any(|v| !v.is_finite()) {
                    return Err(err("Restore offline media before analysis"));
                }
                transcriber
                    .transcribe(&audio, &Default::default(), &mut |f, status| {
                        prog.done.store(30 + (f.clamp(0., 1.) * 40.) as u64, Ordering::Relaxed);
                        *lock(&prog.status) = status.into();
                        !prog.cancel.load(Ordering::Acquire)
                    })
                    .map_err(|e| err(e.to_string()))?
            };
            if tr.words.is_empty() {
                return Err(err("No speech found"));
            }
            *lock(&prog.status) = "OpenAI: evaluating fragments".into();
            let mut proposals = analyst.analyze(&tr, max as u32, &prog.cancel).map_err(err)?;
            align_proposals(&mut proposals, &tr, max as u32)?;
            if prog.cancel.load(Ordering::Acquire) {
                return Err(err("Cancelled"));
            }
            proposals.sort_by_key(|p| std::cmp::Reverse(p.score));
            validate(&proposals, seq.duration().to_units_floor(1000), max as u32)?;
            Ok(Analysis { transcript: tr, proposals })
        };
        match work() {
            Ok(a) => {
                *lock(&out) = Some(a);
                *lock(&result) =
                    Some(Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: 0., bytes: 0, render_fps: 0., extra_files: vec![] }));
                prog.done.store(100, Ordering::Relaxed);
            }
            Err(e) => {
                *lock(&result) = Some(Err(e.to_string()));
            }
        }
        prog.finished.store(true, Ordering::Release);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    std::thread::Builder::new().name("Shorts analysis".into()).spawn(run).map_err(|e| err(e.to_string()))?;
    let id = job.id;
    s.jobs.push(job);
    s.shorts = Some(Review { max_seconds: max as u32, job: id, source, snapshot, result: output });
    Ok(json!({"job":id}))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub format: String,
    pub layout: String,
    pub captions: bool,
    pub caption_preset: String,
    pub font: String,
    pub position: String,
    pub animation: String,
    pub versions: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: "9:16".into(),
            layout: "auto".into(),
            captions: true,
            caption_preset: "bold".into(),
            font: "Inter".into(),
            position: "bottom".into(),
            animation: "none".into(),
            versions: 1,
        }
    }
}
fn fx(id: &str) -> Result<filmcraft_project::EffectInstance> {
    find_effect(id).map(|d| d.instance()).ok_or_else(|| err(format!("Effect unavailable: {id}")))
}
fn set(e: &mut filmcraft_project::EffectInstance, k: &str, v: ParamValue) {
    if let Some(p) = e.params.get_mut(k) {
        p.value = v;
        p.keyframes.clear();
    }
}
fn motion(c: &mut filmcraft_project::TrackItem, scale: f64, x: f64, y: f64) {
    if let Some(m) = c.effects.iter_mut().find(|e| e.effect == "motion") {
        set(m, "scale", ParamValue::Float(scale));
        set(m, "position", ParamValue::Vec2(filmcraft_geom::Vec2::new(x, y)));
    }
}
fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let review = s.shorts.as_ref().ok_or_else(|| err("Analyze first"))?;
    if !context_matches(&s.project, &review.snapshot, review.source) {
        return Err(err("Project changed; analyze again"));
    }
    let analysis = lock(&review.result).clone().ok_or_else(|| err("Analysis is not finished"))?;
    let source = review.source;
    let max_seconds = review.max_seconds;
    validate(&analysis.proposals, s.project.sequence(source).ok_or(EngineError::NoSequence)?.duration().to_units_floor(1000), max_seconds)?;
    let indices = p.get("selected").and_then(Value::as_array).ok_or_else(|| err("Select fragments"))?;
    if indices.is_empty() || indices.len() > 20 {
        return Err(err("Select 1–20 fragments"));
    }
    let mut selected = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for v in indices {
        let i = v.as_u64().filter(|i| *i < 20).ok_or_else(|| err("Invalid selection"))? as usize;
        if !seen.insert(i) {
            return Err(err("Duplicate selection"));
        }
        selected.push(analysis.proposals.get(i).cloned().ok_or_else(|| err("Invalid selection"))?);
    }
    let o: Options = serde_json::from_value(p.get("options").cloned().unwrap_or(json!({}))).map_err(|e| err(e.to_string()))?;
    if !(1..=5).contains(&o.versions) || selected.len() * o.versions as usize > 40 || o.font.len() > 128 || o.font.trim().is_empty() {
        return Err(err("Limit: 5 versions per short, 40 sequences per batch"));
    }
    let (w, h) = match o.format.as_str() {
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        "4:5" => (1080, 1350),
        "16:9" => (1920, 1080),
        _ => return Err(err("Invalid frame format")),
    };
    if !["bold", "minimal", "boxed", "accent"].contains(&o.caption_preset.as_str())
        || !["auto", "fill", "background", "split"].contains(&o.layout.as_str())
        || !["top", "center", "bottom"].contains(&o.position.as_str())
        || !["none", "pop", "fade"].contains(&o.animation.as_str())
    {
        return Err(err("Invalid layout or captions"));
    }
    let original = s.project.sequence(source).ok_or(EngineError::NoSequence)?;
    let source_settings = original.settings.clone();
    let mut natural_cuts = original.edit_points();
    natural_cuts.extend(analysis.transcript.words.windows(2).filter(|w| w[1].start - w[0].end >= Tick(TICKS_PER_SECOND / 4)).map(|w| w[0].end));
    let ids = s.edit("Create vertical videos", |pr, _| {
        let existing = pr.root.children.iter().find_map(|e| match e {
            filmcraft_project::BinEntry::Bin(b) if b.name == "Shorts" => Some(b.id),
            _ => None,
        });
        let bin = existing.unwrap_or_else(|| pr.add_bin("Shorts", None));
        let mut ids = Vec::new();
        for (index, proposal) in selected.iter().enumerate() {
            for version in 0..o.versions {
                let rate = source_settings.frame_rate;
                let fd = rate.frame_duration();
                let start = rate.snap(Tick(proposal.start_ms * TICKS_PER_SECOND / 1000));
                let requested_end = Tick(proposal.end_ms * TICKS_PER_SECOND / 1000);
                let mut end = rate.tick_of(rate.frame_at(requested_end - Tick(1)).saturating_add(1));
                let limit = start + rate.snap(Tick(i64::from(max_seconds) * TICKS_PER_SECOND));
                if end > limit {
                    let last = analysis
                        .transcript
                        .words
                        .iter()
                        .filter(|word| word.end <= limit && word.end > start)
                        .map(|word| word.end)
                        .max()
                        .ok_or_else(|| err("No complete phrase fits the duration limit"))?;
                    end = rate.tick_of(rate.frame_at(last - Tick(1)).saturating_add(1)).min(limit);
                }

                let duration = (end - start).max(fd).min(rate.snap(Tick(i64::from(max_seconds) * TICKS_PER_SECOND)));
                let end = start + duration;
                let speed = if version == 0 { 1. } else { 1. + f64::from(version) * 0.01 };
                let speed_percent = 100 + i64::from(version);
                let output_duration = rate.snap_nearest(Tick(((i128::from(duration.0) * 100) / i128::from(speed_percent)) as i64)).max(fd);
                let mut settings = source_settings.clone();
                settings.width = w;
                settings.height = h;
                settings.par = (1, 1);
                let name = format!("Short {:02} — {}{}", index + 1, proposal.title, if o.versions > 1 { format!(" v{}", version + 1) } else { String::new() });
                let id = pr.new_sequence(&name, settings, 2, 1, Some(bin));
                let mut video = vec![Vec::new(), Vec::new()];
                let segment = if o.layout == "auto" { rate.snap_nearest(Tick(8 * TICKS_PER_SECOND)) } else { output_duration };
                let mut at = Tick::ZERO;
                let mut part = 0;
                while at < output_duration {
                    let mut len = (output_duration - at).min(segment);
                    if o.layout == "auto" {
                        if output_duration - at <= segment + Tick(2 * TICKS_PER_SECOND) {
                            len = output_duration - at;
                        } else {
                            let target = at + segment;
                            if let Some(cut) = natural_cuts
                                .iter()
                                .filter(|c| **c > start && **c < end)
                                .map(|c| rate.snap_nearest(Tick(((i128::from((*c - start).0) * 100) / i128::from(speed_percent)) as i64)))
                                .filter(|c| {
                                    *c > at + Tick(5 * TICKS_PER_SECOND)
                                        && *c < output_duration - Tick(TICKS_PER_SECOND)
                                        && (*c - target).0.abs() < 2 * TICKS_PER_SECOND
                                })
                                .min_by_key(|c| (*c - target).0.abs())
                            {
                                len = cut - at;
                            }
                        }
                    }
                    let src = start + Tick(((i128::from(at.0) * i128::from(speed_percent)) / 100) as i64);
                    let range = TimeRange::new(src, Tick(((i128::from(len.0) * i128::from(speed_percent)) / 100) as i64));
                    let mode = if o.layout == "auto" {
                        ["fill", "background", "split"].get((part + version as usize) % 3).copied().unwrap_or("fill")
                    } else {
                        o.layout.as_str()
                    };
                    let layers = if mode == "fill" { 1 } else { 2 };
                    for layer in 0..layers {
                        let mut c = pr.make_track_item(source, TrackKind::Video, at, range, rate).ok_or_else(|| err("Cannot create source nest"))?;
                        c.duration = len;
                        c.speed = speed;
                        if let Some(m) = c.effects.iter_mut().find(|e| e.effect == "motion") {
                            set(m, "nested_captions", ParamValue::Bool(false));
                        }
                        let sw = f64::from(source_settings.width.max(1));
                        let sh = f64::from(source_settings.height.max(1));
                        let fw = f64::from(w);
                        let fh = f64::from(h);
                        if mode == "background" {
                            let scale = if layer == 0 { (fw / sw).max(fh / sh) } else { (fw / sw).min(fh / sh) };
                            motion(&mut c, scale * 100., fw / 2., fh / 2.);
                            if layer == 0 {
                                let mut blur = fx("gaussian_blur")?;
                                set(&mut blur, "blurriness", ParamValue::Float(45.));
                                c.effects.push(blur);
                            }
                        } else if mode == "split" {
                            let scale = (fw / sw).max(fh / 2. / sh) * if layer == 0 { 1.0 } else { 1.45 };
                            motion(&mut c, scale * 100., fw / 2., if layer == 0 { fh / 4. } else { fh * 0.75 });
                            let mut crop = fx("crop")?;
                            let retained = (fh / 2. / scale / sh).min(1.);
                            let margin = (1. - retained) * 50.;
                            set(&mut crop, "top", ParamValue::Float(margin));
                            set(&mut crop, "bottom", ParamValue::Float(margin));
                            c.effects.push(crop);
                        } else {
                            motion(&mut c, (fw / sw).max(fh / sh) * 100., fw / 2., fh / 2.);
                            let mut reframe = fx("auto_reframe")?;
                            set(
                                &mut reframe,
                                "aspect",
                                ParamValue::Choice(match o.format.as_str() {
                                    "9:16" => 1,
                                    "1:1" => 2,
                                    "4:5" => 3,
                                    _ => 4,
                                }),
                            );
                            c.effects.push(reframe);
                        }
                        if version > 0 {
                            let mut color = fx("brightness_contrast")?;
                            set(&mut color, "brightness", ParamValue::Float(f64::from(version) * 2.));
                            c.effects.push(color);
                        }
                        video.get_mut(layer).ok_or_else(|| err("Missing layer"))?.push(c);
                    }
                    at += len;
                    part += 1;
                }
                let mut audio = pr
                    .make_track_item(source, TrackKind::Audio, Tick::ZERO, TimeRange::new(start, duration), rate)
                    .ok_or_else(|| err("Cannot create audio"))?;
                audio.speed = speed;
                audio.duration = output_duration;
                let mut captions = CaptionTrack::new(filmcraft_project::TrackId(pr.alloc_id()), "Word captions".into(), CaptionFormat::Subtitle);
                captions.style.font = o.font.clone();
                captions.style.size = if o.caption_preset == "minimal" { 36. } else { 52. } + version as f32 * 2.;
                captions.style.background = o.caption_preset == "boxed";
                captions.style.outline = 3.;
                captions.style.anchor = match o.position.as_str() {
                    "top" => CaptionAnchor::Top,
                    "center" => CaptionAnchor::Middle,
                    _ => CaptionAnchor::Bottom,
                };
                if version % 2 == 1 || o.caption_preset == "accent" {
                    captions.style.color = [255, 226, 90, 255];
                }
                if o.captions {
                    for word in &analysis.transcript.words {
                        let a = word.start.max(start);
                        let b = word.end.min(end);
                        if b <= a {
                            continue;
                        }
                        let from = rate.snap_nearest(Tick(((i128::from((a - start).0) * 100) / i128::from(speed_percent)) as i64));
                        let to = rate.snap_nearest(Tick(((i128::from((b - start).0) * 100) / i128::from(speed_percent)) as i64)).min(output_duration);
                        if to <= from {
                            continue;
                        }
                        captions.captions.push(Caption {
                            id: filmcraft_project::ClipId(pr.alloc_id()),
                            start: from,
                            duration: to - from,
                            text: word.text.clone(),
                            speaker: None,
                            cue_id: None,
                            settings: format!("smotri-animation:{}", o.animation),
                        });
                    }
                }
                let q = pr.sequence_mut(id).ok_or(EngineError::NoSequence)?;
                for (t, clips) in q.video_tracks.iter_mut().zip(video) {
                    t.items = clips;
                }
                if let Some(t) = q.audio_tracks.first_mut() {
                    t.items.push(audio)
                }
                if o.captions {
                    q.caption_tracks.push(captions)
                }
                if let Some(item) = pr.item_mut(id) {
                    item.metadata.insert("SmotriShort".into(), "1".into());
                    item.metadata.insert("ShortSource".into(), source.0.to_string());
                    item.metadata.insert("ShortStartMs".into(), proposal.start_ms.to_string());
                    item.metadata.insert("ShortEndMs".into(), proposal.end_ms.to_string());
                    item.metadata.insert(
                        "Description".into(),
                        format!(
                            "Short from sequence {}. Editorial score {}/100 (not a virality guarantee). {}. Variant {}.",
                            source.0,
                            proposal.score,
                            proposal.reason,
                            version + 1
                        ),
                    );
                }
                ids.push(id);
            }
        }
        Ok(ids)
    })?;
    if let Some(review) = s.shorts.as_mut() {
        review.snapshot = s.project.clone();
    }
    Ok(json!({"sequences":ids}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = p.get("sequences").and_then(Value::as_array).ok_or_else(|| err("Select sequences"))?;
    let folder = p.get("folder").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| err("Choose an export folder"))?;
    if !std::path::Path::new(folder).is_dir() {
        return Err(err("Export folder does not exist"));
    }
    if ids.is_empty() || ids.len() > 40 {
        return Err(err("Select 1–40 sequences"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut inputs = Vec::new();
    for v in ids {
        let id = v.as_u64().map(ItemId).ok_or_else(|| err("Invalid sequence"))?;
        if !seen.insert(id) {
            return Err(err("Duplicate sequence"));
        }
        let q = s.project.sequence(id).ok_or(EngineError::NoSequence)?;
        let item = s.project.item(id).ok_or(EngineError::NoSequence)?;
        if !item.metadata.contains_key("SmotriShort") {
            return Err(err("Select sequences created by Vertical Videos"));
        }
        let name = item.name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).take(100).collect::<String>();
        let base = std::path::Path::new(folder).join(format!("{name}-{}", id.0));
        let mut path = base.with_extension("mp4");
        let mut n = 1;
        while path.exists() || s.export_queue.items.iter().any(|i| std::path::Path::new(&i.settings.path) == path) {
            if n > 1000 {
                return Err(err("Too many existing exports with this name"));
            }
            path = std::path::PathBuf::from(format!("{}-{n}.mp4", base.display()));
            n += 1;
        }
        inputs.push(json!({"sequences":[id],"path":path,"format":"h264","burnCaptions":true,"hardwareEncoding":"auto","settings":{"frameSize":[q.settings.width,q.settings.height],"metadata":{"title":item.name,"description":item.metadata.get("Description").cloned().unwrap_or_default()}}}));
    }
    let mut added = Vec::new();
    for input in inputs {
        added.push(s.execute("export.queue.add", input)?);
    }
    if p.get("start").and_then(Value::as_bool).unwrap_or(true) {
        s.execute("export.queue.start", json!({}))?;
    }
    Ok(json!({"queued":added}))
}

pub fn commands() -> Vec<CommandSpec> {
    [
        ("shorts.analyze", "Analyze vertical videos", analyze as fn(&mut Session, &Value) -> Result<Value>),
        ("shorts.status", "Vertical videos analysis status", inspect),
        ("shorts.create", "Create vertical video sequences", create),
        ("shorts.export", "Export vertical videos", export),
    ]
    .into_iter()
    .map(|(id, label, run)| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params: r#"{sequence:id?,duration:30|60|90?,selected:[index]?,options:{format,layout,captions,font,position,animation,versions}?}"#,
        enabled: crate::commands::always,
        run,
        journal: id == "shorts.create",
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::transcript::Word;
    fn fixture() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let source = s.state.active_sequence.unwrap();
        let transcript = Transcript {
            words: vec![
                Word::new("Привет", Tick(TICKS_PER_SECOND), Tick(2 * TICKS_PER_SECOND)),
                Word::new("монтаж!", Tick(2 * TICKS_PER_SECOND), Tick(3 * TICKS_PER_SECOND)),
            ],
            ..Default::default()
        };
        s.shorts = Some(Review {
            max_seconds: 30,
            job: 0,
            source,
            snapshot: s.project.clone(),
            result: Arc::new(Mutex::new(Some(Analysis {
                transcript,
                proposals: vec![Proposal {
                    start_ms: 1000, end_ms: 3000, title: "Тест".into(), score: 72, reason: "Законченная мысль".into()
                }],
            }))),
        });
        s
    }
    #[test]
    fn create_is_atomic_undoable_and_preserves_source() {
        let mut s = fixture();
        let before = s.project.clone();
        let source = s.state.active_sequence.unwrap();
        let undo = s.history.undo.len();
        let ids = s.execute("shorts.create", json!({"selected":[0],"options":{"versions":3,"animation":"pop"}})).unwrap();
        assert_eq!(ids["sequences"].as_array().unwrap().len(), 3);
        for id in ids["sequences"].as_array().unwrap() {
            let q = s.project.sequence(ItemId(id.as_u64().unwrap())).unwrap();
            q.check().unwrap();
            assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
            assert!(q.duration() <= Tick(30 * TICKS_PER_SECOND));
            assert_eq!(q.caption_tracks[0].captions.len(), 2);
            assert_eq!(q.audio_tracks[0].items[0].duration, q.duration());
            for c in &q.caption_tracks[0].captions {
                assert!(c.start >= Tick::ZERO && c.start + c.duration <= q.duration());
            }
        }
        assert_eq!(s.project.sequence(source), before.sequence(source));
        assert_eq!(s.history.undo.len(), undo + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        assert!(s.execute("shorts.create", json!({"selected":[0]})).is_ok(), "Undo should preserve the source analysis");
    }
    #[test]
    fn hostile_parameters_never_edit_or_panic() {
        let mut s = fixture();
        let before = s.project.clone();
        for args in [
            json!({}),
            json!({"selected":[]}),
            json!({"selected":[-1]}),
            json!({"selected":[0,0]}),
            json!({"selected":[99]}),
            json!({"selected":[0],"options":{"versions":0}}),
            json!({"selected":[0],"options":{"versions":4294967295u32}}),
            json!({"selected":[0],"options":{"format":"bad"}}),
            json!({"selected":[0],"options":{"captionPreset":"bad"}}),
            json!({"selected":[0],"options":{"font":""}}),
        ] {
            assert!(s.execute("shorts.create", args).is_err());
            assert_eq!(s.project, before);
        }
        for args in [json!({"duration":-1}), json!({"duration":"30"}), json!({"duration":4000}), json!({"sequence":"x"})] {
            assert!(s.execute("shorts.analyze", args).is_err());
            assert_eq!(s.project, before);
        }
    }
    #[test]
    fn stale_review_is_rejected() {
        let mut s = fixture();
        s.execute("sequence.addMarker", json!({})).ok();
        let source = s.state.active_sequence.unwrap();
        Arc::make_mut(&mut s.project).sequence_mut(source).unwrap().master_volume_db = -9.;
        let before = s.project.clone();
        assert!(s.execute("shorts.create", json!({"selected":[0]})).is_err());
        assert_eq!(s.project, before);
    }
    #[test]
    fn model_times_are_validated_against_words() {
        let s = fixture();
        let a = lock(&s.shorts.as_ref().unwrap().result).clone().unwrap();
        let mut p = a.proposals.clone();
        p[0].start_ms = 1100;
        p[0].end_ms = 2900;
        align_proposals(&mut p, &a.transcript, 30).unwrap();
        assert_eq!((p[0].start_ms, p[0].end_ms), (1000, 3000));
        for (start, end) in [(-1, 3000), (0, i64::MAX), (3000, 2000), (1000, 31000)] {
            p[0].start_ms = start;
            p[0].end_ms = end;
            assert!(align_proposals(&mut p, &a.transcript, 30).is_err());
        }
        p[0].start_ms = 1000;
        p[0].end_ms = 3000;
        p[0].score = 101;
        assert!(validate(&p, 10000, 30).is_err());
    }
    #[test]
    fn every_layout_renders_all_frame_formats() {
        let mut s = fixture();
        for format in ["9:16", "1:1", "4:5", "16:9"] {
            for layout in ["fill", "background", "split", "auto"] {
                let v = s.execute("shorts.create", json!({"selected":[0],"options":{"format":format,"layout":layout}})).unwrap();
                let id = v["sequences"][0].as_u64().unwrap();
                s.execute("sequence.open", json!({"item":id})).unwrap();
                s.execute("playhead.set", json!({"seconds":0.4})).unwrap();
                let image = s.render_program(0.1).unwrap();
                assert!(image.px.iter().all(|x| x.is_finite()));
                assert!(image.px.chunks(4).any(|p| p[0] > 0.02 || p[1] > 0.02 || p[2] > 0.02));
                if std::env::var_os("SMOTRI_SHORTS_ARTIFACTS").is_some() && format == "9:16" {
                    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target").join(format!("shorts-{layout}.png"));
                    image::save_buffer(path, &image.to_rgba8(), image.w as u32, image.h as u32, image::ColorType::Rgba8).unwrap();
                }
            }
        }
        if std::env::var_os("SMOTRI_SHORTS_ARTIFACTS").is_some() {
            s.execute("file.saveAs", json!({"path":std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shorts-verification.fcproj")}))
                .unwrap();
        }
    }
    #[test]
    fn batch_export_uses_individual_size_metadata_and_unique_paths() {
        let mut s = fixture();
        let v = s.execute("shorts.create", json!({"selected":[0],"options":{"versions":2}})).unwrap();
        let folder = std::env::temp_dir().join(format!("smotri-shorts-export-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for _ in 0..2 {
            s.execute("shorts.export", json!({"sequences":v["sequences"],"folder":folder,"start":false})).unwrap();
        }
        let q = &s.export_queue.items;
        assert_eq!(q.len(), 4);
        let paths: std::collections::BTreeSet<_> = q.iter().map(|i| &i.settings.path).collect();
        assert_eq!(paths.len(), 4);
        for i in q {
            assert_eq!(i.settings.frame_size, Some((1080, 1920)));
            assert!(i.settings.burn_captions);
            assert!(!i.settings.metadata.title.is_empty());
            assert!(i.settings.metadata.description.contains("Variant"));
        }
        std::fs::remove_dir(folder).unwrap();
    }
    struct FixedAnalyst;
    impl Analyst for FixedAnalyst {
        fn configured(&self) -> bool {
            true
        }
        fn save_key(&self, _: &str) -> std::result::Result<(), String> {
            Ok(())
        }
        fn analyze(&self, _: &Transcript, _: u32, _: &AtomicBool) -> std::result::Result<Vec<Proposal>, String> {
            Ok(vec![Proposal { start_ms: 100, end_ms: 600, title: "Test".into(), score: 50, reason: "Complete phrase".into() }])
        }
    }
    #[test]
    fn analysis_background_is_read_only_and_reuses_transcript() {
        let mut s = fixture();
        s.shorts = None;
        s.shorts_analyst = Some(Arc::new(FixedAnalyst));
        s.transcriber = Some(Arc::new(filmcraft_speech::FixedTranscriber {
            id: "fixed".into(),
            transcript: Transcript { words: vec![Word::new("Hello", Tick(TICKS_PER_SECOND / 10), Tick(TICKS_PER_SECOND * 6 / 10))], ..Default::default() },
        }));
        let source = s.state.active_sequence.unwrap();
        let pr = Arc::make_mut(&mut s.project);
        let q = pr.sequence_mut(source).unwrap();
        for t in q.all_tracks_mut() {
            t.items.truncate(1);
            for c in &mut t.items {
                c.start = Tick::ZERO;
                c.duration = Tick(TICKS_PER_SECOND);
            }
        }
        q.markers.clear();
        let before = s.project.clone();
        for duration in [30, 60] {
            s.execute("shorts.analyze", json!({"duration":duration})).unwrap();
            let deadline = web_time::Instant::now() + std::time::Duration::from_secs(30);
            while !s.jobs.last().unwrap().progress.finished.load(Ordering::Acquire) && web_time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let state = s.execute("shorts.status", json!({})).unwrap();
            assert_eq!(state["finished"], true);
            assert!(state["error"].is_null(), "{state}");
            assert_eq!(state["proposals"].as_array().unwrap().len(), 1);
            assert!(Arc::ptr_eq(&s.project, &before));
        }
    }
}
