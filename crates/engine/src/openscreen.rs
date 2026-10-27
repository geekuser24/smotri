//! Import OpenScreen recordings as native, independently editable nested sequences.
//! Only project data is imported; OpenScreen is not required at playback/export time.
mod model;
mod motion;
#[cfg(test)]
mod tests;
mod wallpaper;

use crate::{EngineError, Result, Session};
use base64::Engine as _;
use filmcraft_geom::Vec2;
use filmcraft_media::SharedSource;
use filmcraft_project::{
    BinId, Interpolation, ItemId, ItemKind, Keyframe, Label, Mask, MaskPath, MediaClip, MediaRef, Param, ParamValue, Project, SequenceSettings, TrackItem,
    TrackKind,
};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use model::{Cursor, Document, Rect};
use serde_json::{Value, json};
use std::path::Path;
fn error(msg: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("OpenScreen: {msg}"))
}
pub fn is_project(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("openscreen"))
}
fn ms(t: f64) -> Tick {
    Tick((t * (TICKS_PER_SECOND / 1000) as f64).round() as i64)
}
fn millis(t: Tick) -> f64 {
    t.0 as f64 / (TICKS_PER_SECOND / 1000) as f64
}
fn read(s: &Session, path: &str, limit: u64) -> Result<Vec<u8>> {
    let n = s.services.file_size(path).map_err(|e| error(format!("{path}: {e}")))?;
    if n > limit {
        return Err(error(format!("{path}: file exceeds import size limit")));
    }
    let b = s.services.read_file(path).map_err(|e| error(format!("{path}: {e}")))?;
    if b.len() as u64 > limit {
        return Err(error("file grew beyond import size limit"));
    }
    Ok(b)
}
fn resolve(project: &str, path: &str) -> String {
    filmcraft_interchange::resolve_path(path, Path::new(project).parent().map(|p| p.to_string_lossy()).as_deref())
}
struct Source {
    id: ItemId,
    path: String,
    src: SharedSource,
}
fn source(s: &Session, p: &mut Project, bin: BinId, path: String, name: &str) -> Result<Source> {
    s.media.add_stored_paths([path.clone()], None);
    let allowed = s.media.follow(&path).map_err(error)?;
    let src = s.media.open_file(&allowed, &*s.services).map_err(|e| error(format!("{path}: {e}")))?;
    let id = p.add_item(
        name,
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: path.clone() },
            info: src.info().clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        Some(bin),
    );
    Ok(Source { id, path, src })
}
/// Embedded project-owned images are extracted beside the document with exclusive creation.
/// Never overwrite the source project or existing files; never accept an embedded output path.
fn embedded(s: &Session, project: &str, label: &str, url: &str) -> Result<String> {
    let (prefix, data) = url.split_once(',').ok_or_else(|| error("invalid embedded image"))?;
    let ext = match prefix {
        "data:image/png;base64" => "png",
        "data:image/jpeg;base64" => "jpg",
        "data:image/webp;base64" => "webp",
        _ => return Err(error("embedded image must be PNG, JPEG or WebP")),
    };
    if data.len() > 96 * 1024 * 1024 {
        return Err(error("embedded image exceeds 96 MiB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| error(format!("invalid image base64: {e}")))?;
    let root = Path::new(project).with_extension("openscreen-media");
    for i in 0..1000 {
        let path = root.join(format!("{label}-{i}.{ext}")).to_string_lossy().to_string();
        match s.services.write_file_new(&path, &bytes) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if s.services.file_size(&path).ok() == Some(bytes.len() as u64) && s.services.read_file(&path).is_ok_and(|b| b == bytes) {
                    return Ok(path);
                }
            }
            Err(e) => return Err(error(format!("cannot extract image: {e}"))),
        }
    }
    Err(error("no unused asset filename"))
}
fn video_size(src: &Source) -> Result<(f64, f64)> {
    let v = src.src.info().video.as_ref().ok_or_else(|| error(format!("{} has no video", src.path)))?;
    filmcraft_project::validate_frame_size(v.width, v.height).map_err(error)?;
    Ok((f64::from(v.width), f64::from(v.height)))
}
fn clip(p: &mut Project, item: ItemId, kind: TrackKind, start: Tick, duration: Tick, source_in: Tick, settings: &SequenceSettings) -> Result<TrackItem> {
    let mut c =
        p.make_track_item(item, kind, start, TimeRange::new(source_in, duration), settings.frame_rate).ok_or_else(|| error("missing imported media"))?;
    c.duration = duration;
    Ok(c)
}
fn put(c: &mut TrackItem, effect: &str, param: &str, t: Tick, v: ParamValue) {
    if let Some(p) = c.effects.iter_mut().find(|e| e.effect == effect).and_then(|e| e.params.get_mut(param)) {
        append(p, t, v);
    }
}
fn append(p: &mut Param, t: Tick, v: ParamValue) {
    if p.keyframes.is_empty() {
        p.value = v.clone();
    }
    if let Some(last) = p.keyframes.last_mut()
        && last.time == t
    {
        last.value = v;
        return;
    }
    // A long stationary run needs only its endpoints, while retaining the start of motion.
    let n = p.keyframes.len();
    if n >= 2 && p.keyframes.get(n - 2).is_some_and(|k| k.value == v) && p.keyframes.last().is_some_and(|k| k.value == v) {
        if let Some(last) = p.keyframes.last_mut() {
            last.time = t;
        }
        return;
    }
    p.keyframes.push(Keyframe::new(t, v));
}
fn transform(c: &mut TrackItem, t: Tick, k: f64, x: f64, y: f64, anchor: Vec2) {
    put(c, "motion", "anchor", t, ParamValue::Vec2(anchor));
    put(c, "motion", "position", t, ParamValue::Vec2(Vec2::new(x, y)));
    put(c, "motion", "scale", t, ParamValue::Float(k * 100.));
}
fn opacity(c: &mut TrackItem, t: Tick, value: f64) {
    put(c, "opacity", "opacity", t, ParamValue::Float(value));
    if let Some(p) = c.effects.iter_mut().find(|e| e.effect == "opacity").and_then(|e| e.params.get_mut("opacity"))
        && let Some(k) = p.keyframes.last_mut()
    {
        k.interp = Interpolation::Hold;
    }
}
fn mask(c: &mut TrackItem, t: Tick, path: MaskPath) {
    if let Some(e) = c.effects.iter_mut().find(|e| e.effect == "opacity") {
        if e.masks.is_empty() {
            let mut m = Mask::new("OpenScreen frame", path.clone());
            m.feather = Param::new(ParamValue::Float(0.));
            e.masks.push(m);
        }
        if let Some(m) = e.masks.first_mut() {
            append(&mut m.path, t, ParamValue::Path(path));
        }
    }
}
fn fit(c: &mut TrackItem, t: Tick, r: Rect, size: (f64, f64), crop: Rect, cover: bool, z: motion::Transform, radius: f64, circle: bool) {
    let (sw, sh) = size;
    let cw = sw * crop.width;
    let ch = sh * crop.height;
    let scale = if cover { (r.width / cw).max(r.height / ch) } else { r.width / cw };
    let cx = (crop.x + crop.width / 2.) * sw;
    let cy = (crop.y + crop.height / 2.) * sh;
    let mw = r.width / scale;
    let mh = r.height / scale;
    transform(c, t, scale * z.k, (r.x + r.width / 2.) * z.k + z.x, (r.y + r.height / 2.) * z.k + z.y, Vec2::new(cx, cy));
    let path = if circle {
        MaskPath::ellipse(Vec2::new(cx, cy), Vec2::new(mw / 2., mh / 2.))
    } else {
        MaskPath::rounded_rect(cx - mw / 2., cy - mh / 2., cx + mw / 2., cy + mh / 2., radius / scale)
    };
    mask(c, t, path);
}
#[derive(Clone)]
struct Frame {
    at: Tick,
    layout: motion::Layout,
    zoom: motion::Transform,
    cursor: Option<model::Point>,
    asset: Option<usize>,
    bounce: f64,
}
fn sample_frames(
    d: &Document,
    c: &Cursor,
    settings: &SequenceSettings,
    duration: Tick,
    screen: (f64, f64),
    cam: Option<(f64, f64)>,
    camera_duration: Option<Tick>,
) -> Result<Vec<Frame>> {
    let rate = settings.frame_rate;
    let step = rate.frame_duration();
    if step.0 <= 0 {
        return Err(error("invalid recording frame rate"));
    }
    let count = duration.0 / step.0 + 2;
    if !(1..=120_000).contains(&count) {
        return Err(error("recording is too long for editable keyframe import (maximum 120000 frames)"));
    }
    let mut times: Vec<Tick> = (0..count).map(|i| Tick(i * step.0).min(duration)).collect();
    for pose in &d.editor.composition_keyframes {
        let t = ms(pose.time_ms);
        if t > Tick::ZERO && t < duration {
            times.push(t);
            times.push(t - Tick(1));
        }
    }
    for r in d.editor.trim_regions.iter().chain(&d.editor.speed_regions) {
        for t in [ms(r.start_ms), ms(r.end_ms)] {
            if t > Tick::ZERO && t < duration {
                times.push(t);
                times.push(t - Tick(1));
            }
        }
    }
    if let Some(dur) = camera_duration {
        for t in [ms(-d.editor.webcam_offset_ms), dur - ms(d.editor.webcam_offset_ms)] {
            if t > Tick::ZERO && t < duration {
                times.push(t);
                times.push(t - Tick(1));
            }
        }
    }
    times.sort();
    times.dedup();
    let mut camera = motion::Camera::default();
    let cursor = motion::CursorPath::new(c, d.editor.cursor_smoothing);
    let (w, h) = (f64::from(settings.width), f64::from(settings.height));
    Ok(times
        .into_iter()
        .map(|at| {
            let t = millis(at);
            let layout = motion::layout(&d.editor, t, w, h, screen.0 * d.editor.crop_region.width, screen.1 * d.editor.crop_region.height, cam);
            let zoom = camera.step(&d.editor, c, t, w, h);
            let point = cursor.at(c, t);
            let asset =
                motion::sample(c, t).filter(|(s, _)| s.visible && d.editor.show_cursor).and_then(|(s, _)| c.assets.iter().position(|a| a.id == s.asset_id));
            Frame { at, layout, zoom, cursor: point, asset, bounce: motion::bounce(c, t, d.editor.cursor_click_bounce) }
        })
        .collect())
}
/// All assets are validated before the single undoable edit. Failure never replaces an open project.
pub fn import(s: &mut Session, path: &str) -> Result<Value> {
    let bytes = read(s, path, 128 * 1024 * 1024)?;
    let d = model::parse(&bytes)?;
    let e = &d.editor;
    let name = Path::new(path).file_stem().and_then(|n| n.to_str()).unwrap_or("OpenScreen");
    let mut p = (*s.project).clone();
    let bin = p.add_bin(&format!("{name} · OpenScreen"), None);
    let media_bin = p.add_bin("Media", Some(bin));
    let comps = p.add_bin("Compositions", Some(bin));
    let screen = source(s, &mut p, media_bin, resolve(path, &d.media.screen_video_path), "Desktop")?;
    let screen_size = video_size(&screen)?;
    let duration = screen.src.info().duration;
    if duration <= Tick::ZERO || duration > ms(7_200_000.) {
        return Err(error("recording duration must be between zero and two hours"));
    }
    let webcam = d.media.webcam_video_path.as_ref().filter(|s| !s.is_empty()).map(|v| source(s, &mut p, media_bin, resolve(path, v), "Camera")).transpose()?;
    let cam_size = webcam.as_ref().map(video_size).transpose()?;
    for media in std::iter::once(&screen).chain(webcam.as_ref()) {
        media.src.video_frame(filmcraft_media::FrameRequest::full(Tick::ZERO)).map_err(|e| error(format!("cannot decode {}: {e}", media.path)))?;
    }
    let mut report = Vec::new();
    let cursor_path = format!("{}.cursor.json", screen.path);
    let cursor = if d.media.cursor_capture_mode == "editable-overlay" || s.services.file_size(&cursor_path).is_ok() {
        let b = read(s, &cursor_path, 32 * 1024 * 1024)
            .map_err(|_| error(format!("cursor recording is missing or unreadable: {cursor_path}; keep it beside the screen recording")))?;
        model::cursor(&b)?
    } else {
        Cursor::default()
    };
    let ratio = match e.aspect_ratio.as_str() {
        "16:9" => (1920, 1080),
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        "4:3" => (1440, 1080),
        "3:4" => (1080, 1440),
        "21:9" => (2520, 1080),
        "auto" | "original" => (screen_size.0 as u32, screen_size.1 as u32),
        _ => return Err(error(format!("unsupported aspect ratio {}", e.aspect_ratio))),
    };
    let frame_rate = screen.src.info().video.as_ref().ok_or_else(|| error("screen has no video"))?.frame_rate;
    let settings = SequenceSettings { width: ratio.0, height: ratio.1, frame_rate, preset: "OpenScreen".into(), ..Default::default() };
    settings.validate().map_err(error)?;
    let estimated_frames = (duration.0 / settings.frame_rate.frame_duration().0) as usize;
    // Desktop, camera and the single visible cursor can animate at each frame.
    // Inactive cursor assets only need visibility switches, not per-frame work.
    if estimated_frames.saturating_mul(3).saturating_add(cursor.assets.len()) > 1_000_000 {
        return Err(error("too many animated layers for editable import; split the recording into shorter projects"));
    }

    let mut assets = Vec::new();
    for (i, a) in cursor.assets.iter().enumerate() {
        let file = embedded(s, path, &format!("cursor-{i}"), &a.image_data_url)?;
        assets.push(source(s, &mut p, media_bin, file, &format!("Cursor {}", i + 1))?);
    }
    let wallpaper = if e.wallpaper.starts_with("data:") {
        let file = embedded(s, path, "background", &e.wallpaper)?;
        Some(source(s, &mut p, media_bin, file, "Background")?)
    } else if !e.wallpaper.is_empty() && !e.wallpaper.starts_with('#') {
        let file = wallpaper::resolve(s, path, &e.wallpaper)?;
        Some(source(s, &mut p, media_bin, file, "Background")?)
    } else {
        None
    };
    if e.shadow_intensity > 0. || e.motion_blur_amount > 0. {
        report.push("OpenScreen shadow and motion blur are not reproduced; motion and masks remain editable.");
    }
    if e.zoom_regions.iter().any(|z| !z.rotation3d.is_null()) {
        report.push("3D zoom rotation is not imported.");
    }
    if !cursor.assets.is_empty() {
        report.push("Cursor uses the captured cursor images; OpenScreen replacement themes and cursor motion blur are not imported.");
    }
    let mut variants = Vec::new();
    let mut all_sequences = Vec::new();
    let mut original = None;
    let mut output_duration = Tick::ZERO;
    let modes = [
        ("original", "Original edit"),
        ("picture-in-picture", "Picture in picture"),
        ("dual-frame", "Dual screen"),
        ("no-webcam", "Desktop"),
        ("webcam-only", "Webcam"),
    ];
    for (mode, title) in modes {
        if webcam.is_none() && matches!(mode, "dual-frame" | "webcam-only") {
            continue;
        }
        let mut variant = d.clone();
        if mode != "original" {
            variant.editor.webcam_layout_preset = mode.into();
            // Zoom belongs to the parent PiP effect; other angles stay neutral.
            variant.editor.zoom_regions.clear();
            // Each angle is available for the entire recording. Keep its first authored geometry.
            let first = variant.editor.composition_keyframes.iter().find(|k| k.layout_preset == mode).cloned();
            variant.editor.composition_keyframes = first
                .into_iter()
                .map(|mut k| {
                    k.time_ms = 0.;
                    k
                })
                .collect();
        }
        let e = &variant.editor;
        let frames = sample_frames(&variant, &cursor, &settings, duration, screen_size, cam_size, webcam.as_ref().map(|c| c.src.info().duration))?;
        let native = mode != "original";
        let main = p.new_sequence(
            title,
            settings.clone(),
            if native { 3 + assets.len() } else { 1 },
            if native { screen.src.info().audio_streams.len() } else { 1 },
            Some(comps),
        );
        if let Some(item) = p.items.get_mut(&main) {
            item.metadata.insert("smotri.openscreen.layout".into(), mode.into());
        }

        let mut bounds = vec![Tick::ZERO, duration];
        for k in &e.composition_keyframes {
            let t = ms(k.time_ms);
            if t > Tick::ZERO && t < duration {
                bounds.push(t);
            }
        }
        for r in e.trim_regions.iter().chain(&e.speed_regions) {
            for t in [ms(r.start_ms), ms(r.end_ms)] {
                if t > Tick::ZERO && t < duration {
                    bounds.push(t);
                }
            }
        }
        // Camera source clamping becomes explicit frame holds, so late/early camera sync is preserved.
        if let Some(cam) = &webcam {
            for t in [ms(-e.webcam_offset_ms), cam.src.info().duration - ms(e.webcam_offset_ms)] {
                if t > Tick::ZERO && t < duration {
                    bounds.push(t);
                }
            }
        }
        bounds.sort();
        bounds.dedup();
        let mut output = Tick::ZERO;
        let mut sequences = Vec::new();
        for pair in bounds.windows(2) {
            let [start, end] = *pair else { continue };
            let mid = millis(start + Tick((end - start).0 / 2));
            if e.trim_regions.iter().any(|r| mid >= r.start_ms && mid < r.end_ms) {
                continue;
            }
            let speed = e.speed_regions.iter().find(|r| mid >= r.start_ms && mid < r.end_ms).and_then(|r| r.speed).unwrap_or(1.);
            let len = end - start;
            let seq_name = format!(
                "{:02} · {}",
                sequences.len() + 1,
                e.composition_keyframes.iter().rev().find(|k| ms(k.time_ms) <= start).map_or(e.webcam_layout_preset.as_str(), |k| k.layout_preset.as_str())
            );
            let seq =
                if native { main } else { p.new_sequence(&seq_name, settings.clone(), 3 + assets.len(), screen.src.info().audio_streams.len(), Some(comps)) };
            let mut desktop = clip(&mut p, screen.id, TrackKind::Video, Tick::ZERO, len, start, &settings)?;
            let mut cam =
                webcam.as_ref().map(|v| clip(&mut p, v.id, TrackKind::Video, Tick::ZERO, len, start + ms(e.webcam_offset_ms), &settings)).transpose()?;
            if let (Some(c), Some(src)) = (&mut cam, &webcam) {
                let source_duration = src.src.info().duration;
                if c.source_in < Tick::ZERO {
                    c.frame_hold = Some(Tick::ZERO);
                } else if c.source_in >= source_duration {
                    c.frame_hold = Some((source_duration - settings.frame_rate.frame_duration()).max(Tick::ZERO));
                }
                if e.webcam_mirrored
                    && let Some(fx) = filmcraft_project::find_effect("horizontal_flip")
                {
                    c.effects.push(fx.instance());
                }
            }
            let mut cursors: Vec<TrackItem> =
                assets.iter().map(|a| clip(&mut p, a.id, TrackKind::Video, Tick::ZERO, len, Tick::ZERO, &settings)).collect::<Result<_>>()?;
            for c in &mut cursors {
                opacity(c, Tick::ZERO, 0.);
            }
            let mut visible_cursor = None;
            let from = frames.partition_point(|f| f.at < start);
            let to = frames.partition_point(|f| f.at < end);
            for f in frames.get(from..to).unwrap_or_default() {
                let local = f.at - start;
                // Smotri evaluates effects in source time (effect_time_at), including nested clips.
                // Using local time here would freeze/restart motion in every later composition.
                fit(
                    &mut desktop,
                    start + local,
                    f.layout.screen,
                    screen_size,
                    e.crop_region,
                    f.layout.cover,
                    f.zoom,
                    if f.layout.cover { f.layout.radius } else { e.border_radius },
                    false,
                );
                if mode == "webcam-only" {
                    opacity(&mut desktop, start + local, 0.);
                }
                if let (Some(c), Some(size)) = (&mut cam, cam_size) {
                    let ct = c.source_in + local;
                    if let Some(mut rect) = if mode == "webcam-only" {
                        Some(Rect { x: 0., y: 0., width: f64::from(settings.width), height: f64::from(settings.height) })
                    } else {
                        f.layout.webcam
                    } {
                        if e.webcam_reactive_zoom && e.composition_keyframes.is_empty() && e.webcam_layout_preset == "picture-in-picture" {
                            let k = (1. / f.zoom.k).clamp(0.35, 1.);
                            rect.x += rect.width * (1. - k) * if e.webcam_position.is_none_or(|p| p.cx >= 0.5) { 1. } else { 0. };
                            rect.y += rect.height * (1. - k) * if e.webcam_position.is_none_or(|p| p.cy >= 0.5) { 1. } else { 0. };
                            rect.width *= k;
                            rect.height *= k;
                        }
                        fit(
                            c,
                            ct,
                            rect,
                            size,
                            Rect::default(),
                            true,
                            motion::Transform::default(),
                            f.layout.camera_radius,
                            e.webcam_mask_shape == "circle" && !f.layout.cover,
                        );
                        opacity(c, ct, 100.);
                    } else {
                        opacity(c, ct, 0.);
                    }
                }
                let next_cursor = f.asset.filter(|_| mode != "webcam-only" && f.cursor.is_some());
                if visible_cursor != next_cursor {
                    if let Some(c) = visible_cursor.and_then(|i: usize| cursors.get_mut(i)) {
                        opacity(c, local, 0.);
                    }
                    if let Some(c) = next_cursor.and_then(|i| cursors.get_mut(i)) {
                        opacity(c, local, 100.);
                    }
                    visible_cursor = next_cursor;
                }
                if let Some(i) = next_cursor
                    && let (Some(c), Some(point), Some(asset)) = (cursors.get_mut(i), f.cursor, cursor.assets.get(i))
                {
                    let rect = f.layout.screen;
                    let x = rect.x + (point.cx - e.crop_region.x) / e.crop_region.width * rect.width;
                    let y = rect.y + (point.cy - e.crop_region.y) / e.crop_region.height * rect.height;
                    let k = e.cursor_size * f.bounce * f.zoom.k * rect.width / (screen_size.0 * e.crop_region.width * asset.scale_factor);
                    transform(c, local, k, x * f.zoom.k + f.zoom.x, y * f.zoom.k + f.zoom.y, Vec2::new(asset.hotspot_x, asset.hotspot_y));
                }
            }
            let mut bg = if let Some(bg) = &wallpaper {
                let mut c = clip(&mut p, bg.id, TrackKind::Video, Tick::ZERO, len, Tick::ZERO, &settings)?;
                let size = video_size(bg)?;
                let r = Rect { x: 0., y: 0., width: f64::from(settings.width), height: f64::from(settings.height) };
                fit(&mut c, Tick::ZERO, r, size, Rect::default(), true, motion::Transform::default(), 0., false);
                Some(c)
            } else {
                None
            };
            let mut audio = Vec::new();
            for stream in 0..screen.src.info().audio_streams.len() {
                let mut a = clip(&mut p, screen.id, TrackKind::Audio, Tick::ZERO, len, start, &settings)?;
                a.audio_stream = stream;
                a.name = if stream == 0 { "System audio".into() } else { format!("Microphone / audio {}", stream + 1) };
                audio.push(a);
            }
            let output_len = Tick((len.0 as f64 / speed).round() as i64);
            if native {
                for c in std::iter::once(&mut desktop).chain(cam.iter_mut()).chain(cursors.iter_mut()).chain(bg.iter_mut()).chain(audio.iter_mut()) {
                    c.start = output;
                    c.duration = output_len;
                    c.speed = speed;
                }
            }
            let q = p.sequence_mut(seq).ok_or_else(|| error("missing composition"))?;
            if let Some(t) = q.video_tracks.get_mut(0) {
                t.name = "Background".into();
            }
            if let Some(t) = q.video_tracks.get_mut(1) {
                t.name = "Desktop".into();
            }
            if let Some(t) = q.video_tracks.get_mut(0) {
                t.items.extend(bg);
            }
            if let Some(t) = q.video_tracks.get_mut(1) {
                t.items.push(desktop);
            }
            // A track displays one active item: separate cursor assets must never overlap on one track.
            for (i, c) in cursors.into_iter().enumerate() {
                if let Some(t) = q.video_tracks.get_mut(2 + i) {
                    t.name = format!("Cursor {}", i + 1);
                    t.items.push(c);
                }
            }
            if let Some(t) = q.video_tracks.get_mut(2 + assets.len()) {
                t.name = "Camera".into();
                t.items.extend(cam);
            }
            for (track, a) in q.audio_tracks.iter_mut().zip(audio) {
                track.name = a.name.clone();
                track.items.push(a);
            }
            if !native {
                let link = p.alloc_id();
                let mut v = clip(&mut p, seq, TrackKind::Video, output, output_len, Tick::ZERO, &settings)?;
                v.speed = speed;
                v.link = Some(link);
                let mut a = clip(&mut p, seq, TrackKind::Audio, output, output_len, Tick::ZERO, &settings)?;
                a.speed = speed;
                a.link = Some(link);
                let q = p.sequence_mut(main).ok_or_else(|| error("missing main sequence"))?;
                if let Some(t) = q.video_tracks.first_mut() {
                    t.items.push(v);
                }
                if !screen.src.info().audio_streams.is_empty()
                    && let Some(t) = q.audio_tracks.first_mut()
                {
                    t.items.push(a);
                }
            }
            output += output_len;
            sequences.push(seq);
        }
        if sequences.is_empty() {
            return Err(error("all recording content was trimmed"));
        }
        all_sequences.push(main);
        all_sequences.extend(sequences);
        output_duration = output;
        if native {
            variants.push((main, title));
        } else {
            original = Some(main);
        }
    }
    let main = p.new_sequence(name, settings.clone(), variants.len(), 1, Some(bin));
    for (i, (item, title)) in variants.iter().enumerate() {
        let mut v = clip(&mut p, *item, TrackKind::Video, Tick::ZERO, output_duration, Tick::ZERO, &settings)?;
        if i == 0 {
            v.effects.insert(0, imported_zoom(&d, duration)?);
        }
        let q = p.sequence_mut(main).ok_or_else(|| error("missing layout sequence"))?;
        if let Some(t) = q.video_tracks.get_mut(i) {
            t.name = (*title).into();
            t.enabled = i == 0;
            t.items.push(v);
        }
    }
    if !screen.src.info().audio_streams.is_empty()
        && let Some((item, _)) = variants.first()
    {
        let a = clip(&mut p, *item, TrackKind::Audio, Tick::ZERO, output_duration, Tick::ZERO, &settings)?;
        if let Some(t) = p.sequence_mut(main).and_then(|q| q.audio_tracks.first_mut()) {
            t.name = "Recording audio".into();
            t.items.push(a);
        }
    }
    p.resolve_placed_auto_points(|id| id == main || all_sequences.contains(&id), |_| true);

    s.edit("Import OpenScreen", |project, state| {
        *project = p;
        state.active_sequence = Some(main);
        state.open_sequences.push(main);
        state.playheads.insert(main, Tick::ZERO);
        Ok(())
    })?;
    if !report.is_empty() {
        s.toast(format!("OpenScreen: {}", report.join(" ")));
    }
    for src in std::iter::once(screen).chain(webcam).chain(wallpaper).chain(assets) {
        s.media.insert_file(src.id, &src.path, src.src);
    }
    Ok(
        json!({"format":"OpenScreen","sequences":[main.0],"compositions":variants.iter().map(|(id,_)|id.0).collect::<Vec<_>>(),"originalSequence":original.map(|id|id.0),"report":report,"duration":output_duration.0}),
    )
}

/// Translate a recording time to the trimmed/retimed composition timeline.
fn output_time(d: &Document, at: Tick, duration: Tick) -> Tick {
    let mut bounds = vec![Tick::ZERO, at.min(duration)];
    for r in d.editor.trim_regions.iter().chain(&d.editor.speed_regions) {
        for t in [ms(r.start_ms), ms(r.end_ms)] {
            if t > Tick::ZERO && t < at.min(duration) {
                bounds.push(t);
            }
        }
    }
    bounds.sort();
    bounds.dedup();
    let mut out = Tick::ZERO;
    for pair in bounds.windows(2) {
        let [a, b] = *pair else { continue };
        let mid = millis(a + Tick((b - a).0 / 2));
        if d.editor.trim_regions.iter().any(|r| mid >= r.start_ms && mid < r.end_ms) {
            continue;
        }
        let speed = d.editor.speed_regions.iter().find(|r| mid >= r.start_ms && mid < r.end_ms).and_then(|r| r.speed).unwrap_or(1.);
        out += Tick(((b - a).0 as f64 / speed).round() as i64);
    }
    out
}

fn imported_zoom(d: &Document, duration: Tick) -> Result<filmcraft_project::EffectInstance> {
    let mut fx = filmcraft_project::find_effect("openscreen_zoom").ok_or_else(|| error("zoom effect unavailable"))?.instance();
    for z in &d.editor.zoom_regions {
        let start = output_time(d, ms(z.start_ms), duration);
        let end = output_time(d, ms(z.end_ms), duration);
        if end <= start {
            continue;
        }
        let scale = z.custom_scale.unwrap_or(match z.depth {
            1 => 1.25,
            2 => 1.5,
            3 => 1.8,
            4 => 2.2,
            5 => 3.5,
            _ => 5.,
        }) * 100.;
        if let Some(p) = fx.params.get_mut("active") {
            p.put_keyframe(start, ParamValue::Bool(true));
            p.put_keyframe(end, ParamValue::Bool(false));
        }
        if let Some(p) = fx.params.get_mut("scale") {
            p.put_keyframe(start, ParamValue::Float(scale));
            for k in &mut p.keyframes {
                k.interp = Interpolation::Hold;
            }
        }
    }
    Ok(fx)
}

/// Legacy automation id now edits the visible effect; never bakes Motion keys.
/// The UI uses Effect Controls directly instead of an extra dialog.
pub fn add_zoom(s: &mut Session, params: &Value) -> Result<Value> {
    let q = s.active_sequence().ok_or_else(|| error("open the layout sequence"))?;
    let candidates: Vec<_> = q
        .video_tracks
        .iter()
        .flat_map(|t| &t.items)
        .filter(|c| {
            s.state.selection.contains(&c.id)
                && s.project.item(c.item).and_then(|i| i.metadata.get("smotri.openscreen.layout")).is_some_and(|v| v == "picture-in-picture")
        })
        .collect();
    let c = if candidates.len() == 1 { candidates.first().copied() } else { None }.ok_or_else(|| error("select one Picture in picture clip"))?;
    let clip_id = c.id;
    let number = |key: &str, default: f64, min: f64, max: f64| -> Result<f64> {
        let v = match params.get(key) {
            None => default,
            Some(v) => v.as_f64().ok_or_else(|| error(format!("invalid {key}")))?,
        };
        if !v.is_finite() || !(min..=max).contains(&v) {
            return Err(error(format!("{key} must be between {min} and {max}")));
        }
        Ok(v)
    };
    let start = ms(number("start", s.playhead().seconds(), 0., 7200.)? * 1000.);
    let end = start + ms(number("duration", 3., 0.1, 120.)? * 1000.);
    let scale = number("scale", 180., 100., 500.)?;
    let follow = match params.get("follow") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| error("follow must be boolean"))?,
    };
    if start < c.start || start >= c.start + c.duration {
        return Err(error("zoom starts outside the clip"));
    }
    let a = c.effect_time_at(start);
    let b = c.effect_time_at(end.min(c.start + c.duration));
    s.edit_sequence("OpenScreen zoom keys", |q, _, _| {
        let c = q.all_tracks_mut().flat_map(|t| &mut t.items).find(|c| c.id == clip_id).ok_or_else(|| error("missing clip"))?;
        if !c.effects.iter().any(|e| e.effect == "openscreen_zoom") {
            c.effects.insert(0, filmcraft_project::find_effect("openscreen_zoom").ok_or_else(|| error("zoom effect unavailable"))?.instance());
        }
        let fx = c.effects.iter_mut().find(|e| e.effect == "openscreen_zoom").ok_or_else(|| error("zoom effect unavailable"))?;
        if let Some(p) = fx.params.get_mut("active") {
            p.put_keyframe(a, ParamValue::Bool(true));
            p.put_keyframe(b, ParamValue::Bool(false));
        }
        if let Some(p) = fx.params.get_mut("scale") {
            p.put_keyframe(a, ParamValue::Float(scale));
            for k in &mut p.keyframes {
                k.interp = Interpolation::Hold;
            }
        }
        fx.params.insert("follow".into(), Param::new(ParamValue::Bool(follow)));
        Ok(())
    })?;
    Ok(json!({"clip":clip_id.0,"effect":"openscreen_zoom","start":a.0,"end":b.0}))
}
