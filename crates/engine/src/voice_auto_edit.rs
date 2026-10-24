//! Beat-driven video assembly. Analysis is background work; publishing is one undoable edit.
use crate::{
    EngineError, Result, Session,
    commands::{CommandSpec, has_seq},
};
use filmcraft_project::{ClipId, ItemId, ParamValue, Project, TrackItem, TrackKind, effect::find_effect};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};
const SR: u32 = 16000;
const MAX_SECONDS: i64 = 180;
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
    selection: Vec<ClipId>,
    videos: Vec<TrackItem>,
    duration: Tick,
    output: Arc<Mutex<Option<Output>>>,
}
struct Output {
    cuts: Vec<(Tick, f32)>,
    bpm: Option<f64>,
    wav: PathBuf,
}
fn selection(s: &Session) -> Result<(Vec<TrackItem>, Vec<ClipId>, Tick, Tick)> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let mut videos = Vec::new();
    let mut audio = Vec::new();
    let mut begin = None;
    let mut end = Tick::ZERO;
    for id in &s.state.selection {
        let (tid, c) = q.find_item(*id).ok_or_else(|| err("Jarvis Edit: выделение устарело"))?;
        let t = q.track(tid).ok_or(EngineError::NoSequence)?;
        if !c.enabled {
            continue;
        }
        if c.reverse
            || c.frame_hold.is_some()
            || !c.speed.is_finite()
            || c.speed <= 0.
            || c.effects.iter().any(|e| {
                e.effect == "time_remap" && (e.params.values().any(|p| !p.keyframes.is_empty()) || (e.f64_at("speed", c.source_in) - 100.).abs() > 0.001)
            })
        {
            return Err(err("Jarvis Edit: нужны клипы без реверса, стоп-кадра и Time Remapping"));
        }
        if t.kind == TrackKind::Video {
            videos.push(c.clone());
        } else if !t.muted && t.enabled {
            audio.push(*id);
            begin = Some(begin.map_or(c.start, |b: Tick| b.min(c.start)));
            end = end.max(c.end());
        }
    }
    if videos.is_empty() {
        return Err(err("Jarvis Edit: выдели хотя бы один видеоклип и аудио с музыкой"));
    }
    if audio.is_empty() {
        return Err(err("Jarvis Edit: не хватает выделенного аудиоклипа с музыкой или ударами"));
    }
    if videos.len() > 100 || audio.len() > 100 {
        return Err(err("Jarvis Edit: максимум 100 видео и 100 аудиоклипов"));
    }
    videos.sort_by_key(|c| (c.start, c.id));
    let begin = begin.ok_or_else(|| err("Jarvis Edit: нет аудио"))?;
    if end - begin < Tick(TICKS_PER_SECOND * 2) || end - begin > Tick(TICKS_PER_SECOND * MAX_SECONDS) {
        return Err(err("Jarvis Edit: выбери от 2 секунд до 3 минут аудио"));
    }
    if videos.iter().any(|c| c.duration < q.settings.frame_rate.tick_of(6)) {
        return Err(err("Jarvis Edit: видеоклипы должны быть не короче 6 кадров"));
    }
    Ok((videos, audio, begin, end))
}
fn start(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.as_object().is_some_and(|p| p.is_empty()) {
        return Err(err("Jarvis Edit uses the current selection and no parameters"));
    }
    if !s.voice_auto_edits.is_empty() {
        return Err(err("Jarvis Edit: анализ уже выполняется"));
    }
    let (videos, audio, begin, end) = selection(s)?;
    let sequence = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let mut q = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    for t in &mut q.audio_tracks {
        t.items.retain(|c| audio.contains(&c.id));
    }
    q.settings.sample_rate = 48000;
    let duration = q.settings.frame_rate.snap_nearest(end - begin);
    let snapshot = s.project.clone();
    let provider = s.media.full_res_provider(snapshot.clone(), s.services.clone()).recording();
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).saturating_add(1);
    let job = crate::Job { id, label: "Jarvis: rhythmic edit".into(), progress: Default::default(), result: Default::default() };
    let progress = job.progress.clone();
    let result = job.result.clone();
    let output: Arc<Mutex<Option<Output>>> = Arc::default();
    let out = output.clone();
    let base = s
        .path
        .as_ref()
        .and_then(|p| std::path::Path::new(p).parent().map(|p| p.join("Jarvis Edits")))
        .or_else(|| crate::autosave::default_data_dir().map(|p| p.join("Jarvis Edits")))
        .ok_or_else(|| err("No output folder"))?;
    let run = move || {
        let work = || -> Result<Output> {
            let frames = duration.to_units_floor(48000).max(1) as usize;
            let mut stereo = Vec::with_capacity(frames * 2);
            progress.total.store(100, Ordering::Relaxed);
            *lock(&progress.status) = "Jarvis Edit: анализ выбранной музыки".into();
            let offset = begin.to_units_floor(48000);
            let mut at = 0;
            while at < frames {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(err("Jarvis Edit отменён"));
                }
                let n = (frames - at).min(48000);
                let buf = filmcraft_render::mixer::mix_graph(&snapshot, &q, offset + at as i64, n, &provider, None);
                if buf.channels.len() > 2 {
                    return Err(err("Jarvis Edit: для этой версии нужен моно или стерео мастер-аудио"));
                }
                if buf.frames() != n || buf.channels.is_empty() {
                    return Err(err("Jarvis Edit: аудио не удалось прочитать"));
                }
                for i in 0..n {
                    let l = buf.channels.first().and_then(|c| c.get(i)).copied().unwrap_or(0.);
                    let r = buf.channels.get(1).and_then(|c| c.get(i)).copied().unwrap_or(l);
                    if !l.is_finite() || !r.is_finite() {
                        return Err(err("Jarvis Edit: некорректные аудиосэмплы"));
                    }
                    stereo.extend([l, r]);
                }
                at += n;
                progress.done.store((at * 35 / frames) as u64, Ordering::Relaxed);
            }
            if !provider.offline_used().is_empty() {
                return Err(err("Jarvis Edit: аудиофайл недоступен; переподключи медиа"));
            }
            *lock(&progress.status) = "Jarvis Edit: поиск ударов и темпа".into();
            // Analyse the more energetic channel, avoiding stereo phase cancellation.
            let energies = stereo.as_chunks::<2>().0.iter().fold([0.0f64; 2], |mut e, f| {
                e[0] += f[0] as f64 * f[0] as f64;
                e[1] += f[1] as f64 * f[1] as f64;
                e
            });
            let channel = usize::from(energies[1] > energies[0]);
            let mono: Vec<f32> = stereo.as_chunks::<6>().0.iter().map(|f| (f[channel] + f[channel + 2] + f[channel + 4]) / 3.).collect();
            let (cuts, bpm) = rhythm(&mono, q.settings.frame_rate, duration)?;
            if progress.cancel.load(Ordering::Relaxed) {
                return Err(err("Jarvis Edit отменён"));
            }
            std::fs::create_dir_all(&base).map_err(|e| err(e.to_string()))?;
            let date: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
            let wav = base.join(format!("Jarvis-Edit-{}-{id}.wav", date.format("%Y-%m-%d_%H-%M-%S%.3f")));
            std::fs::write(&wav, filmcraft_media::wav::write_wav16(&stereo, 2, 48000)).map_err(|e| err(e.to_string()))?;
            Ok(Output { cuts, bpm, wav })
        };
        match work() {
            Ok(o) => {
                let path = o.wav.to_string_lossy().into_owned();
                *lock(&out) = Some(o);
                *lock(&result) = Some(Ok(filmcraft_export::Report { path, frames: 0, seconds: 0., bytes: 0, render_fps: 0., extra_files: vec![] }));
                progress.done.store(100, Ordering::Relaxed);
            }
            Err(e) => {
                *lock(&result) = Some(Err(e.to_string()));
            }
        }
        progress.finished.store(true, Ordering::Release);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    std::thread::Builder::new().name("Jarvis rhythmic edit".into()).spawn(run).map_err(|e| err(e.to_string()))?;
    s.voice_auto_edits.push(Pending { job: id, snapshot: s.project.clone(), sequence, selection: s.state.selection.clone(), videos, duration, output });
    s.jobs.push(job);
    s.toast("Jarvis Edit: анализирую удары; результат откроется отдельным эпизодом");
    Ok(json!({"job":id}))
}
/// Spectral attacks with local thresholds: quiet attacks can pass next to loud sections.
fn rhythm(audio: &[f32], rate: filmcraft_time::FrameRate, duration: Tick) -> Result<(Vec<(Tick, f32)>, Option<f64>)> {
    if audio.len() > SR as usize * MAX_SECONDS as usize || audio.iter().any(|v| !v.is_finite()) {
        return Err(err("Invalid rhythm audio"));
    }
    let (env, n, hop) = filmcraft_audio_dsp::remix::onset_envelope(audio, SR);
    let mut peaks = Vec::new();
    for i in 2..env.len().saturating_sub(2) {
        let a = i.saturating_sub(25);
        let b = (i + 26).min(env.len());
        let avg = env[a..b].iter().sum::<f32>() / (b - a) as f32;
        if env[i] > 0.18 && env[i] > avg * 1.35 && env[i] >= env[i - 1] && env[i] > env[i + 1] {
            let sample = (i * hop + n).saturating_sub(3 * hop / 4);
            let t = rate.snap_nearest(Tick(sample as i64 * TICKS_PER_SECOND / SR as i64));
            if t > Tick::ZERO && t < duration {
                peaks.push((t, env[i] / avg.max(0.01)));
            }
        }
    }
    if peaks.len() < 4 {
        return Err(err("Jarvis Edit: не удалось найти достаточно ударов; выбери ритмичную музыку"));
    }
    let analysis = filmcraft_audio_dsp::remix::analyze(&[audio], SR).ok();
    let bpm = analysis.as_ref().map(|a| a.bpm);
    let mut cuts = vec![(Tick::ZERO, 0.)];
    for (t, strength) in peaks {
        let on_grid = analysis.as_ref().is_none_or(|a| a.beats.iter().any(|b| (Tick(*b * TICKS_PER_SECOND / SR as i64) - t).0.abs() < TICKS_PER_SECOND / 10));
        let last = cuts.last().map(|c| c.0).unwrap_or(Tick::ZERO);
        // Allow strong off-beat fills, but avoid a cut on every hi-hat.
        if t - last >= Tick(TICKS_PER_SECOND / 3) && (on_grid || strength > 3.5 || t - last > Tick(TICKS_PER_SECOND * 2)) && duration - t >= rate.tick_of(6) {
            cuts.push((t, strength));
        }
    }
    if cuts.len() < 4 {
        return Err(err("Jarvis Edit: ритм слишком неопределённый для монтажа"));
    }
    if cuts.len() > 600 {
        return Err(err("Jarvis Edit: слишком много склеек"));
    }
    cuts.push((duration, 0.));
    Ok((cuts, bpm))
}
fn pulse(c: &mut TrackItem, id: &str, param: &str, peak: f64, base: f64, ms: i64) -> Result<()> {
    let mut e = find_effect(id).ok_or_else(|| err(format!("Effect unavailable: {id}")))?.instance();
    let end = (c.start + Tick(ms * TICKS_PER_SECOND / 1000)).min(c.end());
    let a = c.source_time_at(c.start);
    let b = c.source_time_at(end);
    let v = e.params.get_mut(param).ok_or_else(|| err("Effect parameter missing"))?;
    v.value = ParamValue::Float(base);
    v.put_keyframe(a, ParamValue::Float(peak));
    v.put_keyframe(b, ParamValue::Float(base));
    let pos = c.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(c.effects.len());
    c.effects.insert(pos, e);
    Ok(())
}
fn apply(s: &mut Session, p: &Pending, o: Output) -> Result<()> {
    if !Arc::ptr_eq(&s.project, &p.snapshot) || s.state.active_sequence != Some(p.sequence) || s.state.selection != p.selection {
        return Err(err("Jarvis Edit: проект или выделение изменились во время анализа; повтори команду"));
    }
    let mut count = 0;
    s.edit_group("Jarvis: rhythmic edit", |s| {
        let audio = crate::commands::import_streamed(s, &o.wav.to_string_lossy(), None)?;
        let id = s.edit("Build rhythmic sequence", |pr, st| {
            let settings = pr.sequence(p.sequence).ok_or(EngineError::NoSequence)?.settings.clone();
            let rate = settings.frame_rate;
            let dimensions = (settings.width, settings.height);
            let date: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
            let id = pr.new_sequence(&format!("Jarvis Edit {}", date.format("%d.%m %H:%M:%S UTC")), settings, 1, 1, None);
            let mut clips = Vec::new();
            let mut at = Tick::ZERO;
            let mut index = 0;
            let mut previous = None;
            for window in o.cuts.windows(2) {
                let end = window[1].0;
                while at < end {
                    if clips.len() >= 1200 {
                        return Err(err("Jarvis Edit: слишком много фрагментов"));
                    }
                    let mut source_index = index % p.videos.len();
                    if let Some(last) = previous
                        && let Some(other) = (0..p.videos.len()).map(|d| (source_index + d) % p.videos.len()).find(|i| p.videos[*i].item != last)
                    {
                        source_index = other;
                    }
                    let source = &p.videos[source_index];
                    let len = (end - at).min(source.duration);
                    let spare = (source.duration - len).0.max(0);
                    let steps = spare / rate.frame_duration().0.max(1);
                    let offset = Tick((if steps > 0 { (index as i64 * 7919) % (steps + 1) } else { 0 }) * rate.frame_duration().0.max(1));
                    let mut c = source.clone();
                    c.id = ClipId(pr.alloc_id());
                    c.start = at;
                    c.duration = len;
                    c.source_in = source.source_time_at(source.start + offset);
                    c.link = None;
                    c.group = None;
                    c.markers.clear();
                    // Existing framing/effects are preserved; accents are separate effects.
                    let strong = window[0].1 >= 2.0;
                    if index % 4 == 0 || strong && index % 2 == 0 {
                        pulse(&mut c, "camera_shake", "amount", if strong { 24. } else { 10. }, 0., 240)?;
                        if let Some(e) = c.effects.iter_mut().find(|e| e.effect == "camera_shake") {
                            for (k, v) in [("seed", (index * 7919 % 10000) as f64), ("frequency", 12.), ("zoom", 3.), ("rotation", 0.)] {
                                if let Some(p) = e.params.get_mut(k) {
                                    p.value = ParamValue::Float(v);
                                }
                            }
                        }
                    }
                    // A separate Transform keeps the user's Motion scale intact.
                    if index % 2 == 0 {
                        pulse(&mut c, "transform", "scale_height", 108., 100., 220)?;
                    }
                    if strong && index % 4 == 0 {
                        pulse(&mut c, "brightness_contrast", "brightness", 24., 0., 120)?;
                    }
                    if strong && index % 8 == 4 {
                        pulse(&mut c, "wonder_glow", "intensity", 45., 0., 280)?;
                    }
                    for e in &mut c.effects {
                        filmcraft_project::resolve_auto_points(e, dimensions, dimensions);
                    }
                    previous = Some(source.item);
                    clips.push(c);
                    at += len;
                    index += 1;
                }
            }
            count = clips.len();
            let a = pr
                .make_track_item(audio, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, p.duration), rate)
                .ok_or_else(|| err("Cannot place music"))?;
            let q = pr.sequence_mut(id).ok_or(EngineError::NoSequence)?;
            q.video_tracks.first_mut().ok_or_else(|| err("Video track missing"))?.items = clips;
            q.audio_tracks.first_mut().ok_or_else(|| err("Audio track missing"))?.items.push(a);
            q.check().map_err(err)?;
            st.project_selection = vec![id];
            Ok(id)
        })?;
        s.execute("sequence.open", json!({"item":id}))?;
        s.execute("playhead.set", json!({"time":0}))?;
        Ok(())
    })?;
    s.toast(format!("Jarvis Edit: готово, {count} фрагментов{}. Отменить — Undo", o.bpm.map(|b| format!(", {b:.0} BPM")).unwrap_or_default()));
    Ok(())
}
pub(crate) fn poll(s: &mut Session) {
    let mut i = 0;
    while i < s.voice_auto_edits.len() {
        let p = &s.voice_auto_edits[i];
        if s.jobs.iter().find(|j| j.id == p.job).is_some_and(|j| !j.progress.finished.load(Ordering::Acquire)) {
            i += 1;
            continue;
        }
        let p = s.voice_auto_edits.remove(i);
        if s.jobs.iter().find(|j| j.id == p.job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed)) {
            continue;
        }
        let output = lock(&p.output).take();
        let r = if let Some(o) = output {
            apply(s, &p, o)
        } else {
            Err(err(s
                .jobs
                .iter()
                .find(|j| j.id == p.job)
                .and_then(|j| lock(&j.result).clone())
                .and_then(|r| r.err())
                .unwrap_or_else(|| "Jarvis Edit: анализ завершился без результата".into())))
        };
        if let Err(e) = r {
            if let Some(j) = s.jobs.iter().find(|j| j.id == p.job) {
                *lock(&j.result) = Some(Err(e.to_string()));
            }
            s.error_toast("voice.autoEdit", e.to_string());
        }
    }
}
pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec { id: "voice.autoEdit", label: "Jarvis Auto Edit", menu: &[], shortcut: None, params: "{}", enabled: has_seq, run: start, journal: true }]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn drums() -> Vec<f32> {
        let mut x = vec![0.; SR as usize * 12];
        for beat in 0..24 {
            let start = beat * SR as usize / 2;
            let gain = if beat % 4 == 0 { 0.8 } else { 0.12 };
            for i in 0..1600 {
                let t = i as f32 / SR as f32;
                x[start + i] += gain * (-t * 45.).exp() * (std::f32::consts::TAU * (65. * t + 0.3 * (1. - (-t * 50.).exp()))).sin();
            }
        }
        x
    }
    #[test]
    fn attacks_include_quiet_beats_and_reject_silence() {
        let rate = filmcraft_time::FrameRate::FPS_24;
        let (cuts, _) = rhythm(&drums(), rate, Tick(12 * TICKS_PER_SECOND)).unwrap();
        assert!(cuts.len() > 12, "{cuts:?}");
        let near = |time: f64| cuts.iter().any(|(t, _)| (t.0 as f64 / TICKS_PER_SECOND as f64 - time).abs() < 0.09);
        for t in [0.5, 1.0, 2.5, 4.5, 6.5] {
            assert!(near(t), "quiet attack {t}: {cuts:?}");
        }
        assert!(rhythm(&vec![0.; SR as usize * 4], rate, Tick(4 * TICKS_PER_SECOND)).is_err());
        assert!(rhythm(&[f32::NAN], rate, Tick(TICKS_PER_SECOND)).is_err());
    }
    #[test]
    fn montage_is_bounded_alternates_preserves_sources_and_undo() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let q = s.active_sequence().unwrap();
        let ids = vec![q.video_tracks[0].items[0].id, q.video_tracks[0].items[1].id, q.audio_tracks[1].items[0].id];
        s.state.selection = ids.clone();
        let (videos, _, _, _) = selection(&s).unwrap();
        let before = s.project.clone();
        let original = s.state.active_sequence.unwrap();
        let undo = s.history.undo.len();
        let duration = s.sequence_rate().snap_nearest(Tick(TICKS_PER_SECOND * 12));
        let dir = std::env::temp_dir().join(format!("filmcraft-beat-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("beats.wav");
        std::fs::write(&wav, filmcraft_media::wav::write_wav16(&drums(), 1, SR)).unwrap();
        let p = Pending { job: 0, snapshot: before.clone(), sequence: original, selection: ids, videos: videos.clone(), duration, output: Default::default() };
        let rate = s.sequence_rate();
        let (cuts, bpm) = rhythm(&drums(), rate, duration).unwrap();
        let output = || Output { cuts: cuts.clone(), bpm, wav: wav.clone() };
        apply(&mut s, &p, output()).unwrap();
        let q = s.active_sequence().unwrap();
        q.check().unwrap();
        assert_ne!(s.state.active_sequence, Some(original));
        assert_eq!(s.project.sequence(original), before.sequence(original));
        assert_eq!(q.duration(), duration);
        let clips = &q.video_tracks[0].items;
        assert!(clips.len() > 12);
        for pair in clips.windows(2) {
            assert_ne!(pair[0].item, pair[1].item);
            assert_eq!(pair[0].end(), pair[1].start);
        }
        for c in clips {
            let source = videos.iter().find(|v| v.item == c.item).unwrap();
            assert!(c.source_in >= source.source_in);
            assert!(c.source_time_at(c.end()) <= source.source_time_at(source.end()));
        }
        assert!(s.render_program(0.1).unwrap().px.chunks(4).any(|p| p[0] > 0.02 || p[1] > 0.02 || p[2] > 0.02));
        assert_eq!(s.history.undo.len(), undo + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        s.execute("sequence.open", json!({"item":original})).unwrap();
        s.state.selection.clear();
        assert!(apply(&mut s, &p, output()).is_err());
        assert_eq!(s.project, before);
        for args in [json!(null), json!({"clips":[]})] {
            assert!(start(&mut s, &args).is_err());
        }
        assert!(selection(&s).is_err());
        s.state.selection = vec![videos[0].id];
        assert!(selection(&s).unwrap_err().to_string().contains("аудио"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
