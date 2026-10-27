//! OpenScreen layout / camera semantics translated to editable sampled keys.
//! See THIRD-PARTY-OPENSCREEN.txt for the upstream MIT notice.
use super::model::*;
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub screen: Rect,
    pub webcam: Option<Rect>,
    pub cover: bool,
    pub radius: f64,
    pub camera_radius: f64,
}
fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn blend(a: Rect, b: Rect, t: f64) -> Rect {
    Rect { x: mix(a.x, b.x, t), y: mix(a.y, b.y, t), width: mix(a.width, b.width, t), height: mix(a.height, b.height, t) }
}
fn center(w: f64, h: f64, sw: f64, sh: f64, p: f64) -> Rect {
    let k = (w * p / sw).min(h * p / sh);
    let width = (sw * k).round();
    let height = (sh * k).round();
    Rect { x: ((w - width) / 2.).floor().max(0.), y: ((h - height) / 2.).floor().max(0.), width, height }
}
pub fn layout(e: &Editor, t: f64, w: f64, h: f64, sw: f64, sh: f64, cam: Option<(f64, f64)>) -> Layout {
    let eval = |pose: Option<&Pose>| {
        let preset = pose.map_or(e.webcam_layout_preset.as_str(), |p| p.layout_preset.as_str());
        let p = if preset == "vertical-stack" { 1. } else { 1. - e.padding * 0.004 };
        let mut l = Layout { screen: center(w, h, sw, sh, p), webcam: None, cover: false, radius: 0., camera_radius: 0. };
        if let Some((cw, ch)) = cam.filter(|_| preset != "no-webcam") {
            match preset {
                "vertical-stack" => {
                    let height = (w * ch / cw).round();
                    l.screen = Rect { x: 0., y: 0., width: w, height: (h - height).max(1.) };
                    l.webcam = Some(Rect { x: 0., y: (h - height).max(0.), width: w, height });
                    l.cover = true;
                }
                "dual-frame" => {
                    let width = (w * p).round();
                    let height = (h * p).round();
                    let x = ((w - width) / 2.).floor();
                    let y = ((h - height) / 2.).floor();
                    let gap = (width * 0.02).round().max(12.);
                    let left = ((width - gap) * 2. / 3.).round();
                    let right = (width - gap - left).max(1.);
                    l.screen = Rect { x, y, width: left, height };
                    l.webcam = Some(Rect { x: x + left + gap, y, width: right, height });
                    l.cover = true;
                    l.radius = (right.min(height) * 0.06).round().clamp(12., 24.);
                    l.camera_radius = l.radius;
                }
                _ => {
                    let k = (w * h).sqrt() * e.webcam_size_preset / 100. / cw.max(ch);
                    let mut width = (cw * k).round();
                    let mut height = (ch * k).round();
                    if matches!(e.webcam_mask_shape.as_str(), "circle" | "square") {
                        width = width.min(height);
                        height = width;
                    }
                    let margin = (w.min(h) * 0.02).round();
                    let (x, y) = e.webcam_position.map_or(((w - margin - width).max(0.), (h - margin - height).max(0.)), |p| {
                        ((p.cx * w - width / 2.).round().clamp(0., (w - width).max(0.)), (p.cy * h - height / 2.).round().clamp(0., (h - height).max(0.)))
                    });
                    l.webcam = Some(Rect { x, y, width, height });
                    l.camera_radius = match e.webcam_mask_shape.as_str() {
                        "rounded" => (width.min(height) * 0.3).round(),
                        "circle" => (width.min(height) / 2.).round(),
                        _ => (width.min(height) * 0.12).round().clamp(12., 24.),
                    };
                }
            }
        }
        if let Some(p) = pose {
            if let Some(r) = p.screen {
                l.screen = Rect { x: r.x * w, y: r.y * h, width: r.width * w, height: r.height * h };
            }
            if l.webcam.is_some()
                && let Some(r) = p.webcam
            {
                l.webcam = Some(Rect { x: r.x * w, y: r.y * h, width: r.width * w, height: r.height * h });
            }
        }
        l
    };
    let i = e.composition_keyframes.partition_point(|p| p.time_ms <= t);
    let left = i.checked_sub(1).and_then(|j| e.composition_keyframes.get(j));
    let mut a = eval(left);
    if let (Some(left), Some(right)) = (left, e.composition_keyframes.get(i))
        && left.easing != "hold"
        && left.layout_preset == right.layout_preset
        && right.time_ms > left.time_ms
    {
        let b = eval(Some(right));
        let mut f = ((t - left.time_ms) / (right.time_ms - left.time_ms)).clamp(0., 1.);
        if left.easing == "smooth" {
            f = f * f * (3. - 2. * f);
        }
        a.screen = blend(a.screen, b.screen, f);
        if let (Some(x), Some(y)) = (a.webcam, b.webcam) {
            a.webcam = Some(blend(x, y, f));
        }
    }
    a
}
pub fn sample(c: &Cursor, t: f64) -> Option<(&Sample, Point)> {
    let i = c.samples.partition_point(|s| s.time_ms <= t).saturating_sub(1);
    let a = c.samples.get(i)?;
    let b = c.samples.get(i + 1).unwrap_or(a);
    let f = if b.time_ms > a.time_ms { ((t - a.time_ms) / (b.time_ms - a.time_ms)).clamp(0., 1.) } else { 0. };
    Some((a, Point { cx: mix(a.cx, b.cx, f), cy: mix(a.cy, b.cy, f) }))
}
fn clamp_focus(p: Point, k: f64) -> Point {
    let m = 0.5 / k;
    Point { cx: p.cx.clamp(m, 1. - m), cy: p.cy.clamp(m, 1. - m) }
}
fn zoom_scale(z: &Zoom) -> f64 {
    z.custom_scale.unwrap_or(match z.depth {
        1 => 1.25,
        2 => 1.5,
        3 => 1.8,
        4 => 2.2,
        5 => 3.5,
        _ => 5.,
    })
}
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let cubic = |a: f64, b: f64, s: f64| 3. * a * (1. - s).powi(2) * s + 3. * b * (1. - s) * s * s + s * s * s;
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..24 {
        let m = (lo + hi) * 0.5;
        if cubic(x1, x2, m) < t.clamp(0., 1.) {
            lo = m;
        } else {
            hi = m;
        }
    }
    cubic(y1, y2, (lo + hi) * 0.5)
}
fn strength(z: &Zoom, t: f64) -> f64 {
    let end = z.start_ms + 500.;
    let begin = end - 1522.575;
    if t < begin || t > z.end_ms + 1015.05 {
        0.
    } else if t < end {
        bezier(0.16, 1., 0.3, 1., (t - begin) / 1522.575)
    } else if t <= z.end_ms {
        1.
    } else {
        1. - bezier(0.16, 1., 0.3, 1., (t - z.end_ms) / 1015.05)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub k: f64,
    pub x: f64,
    pub y: f64,
}
impl Default for Transform {
    fn default() -> Self {
        Self { k: 1., x: 0., y: 0. }
    }
}
fn transform(k: f64, p: Point, v: f64, w: f64, h: f64) -> Transform {
    Transform { k: 1. + (k - 1.) * v, x: (w * 0.5 - p.cx * w * k) * v, y: (h * 0.5 - p.cy * h * k) * v }
}
#[derive(Default)]
pub struct Camera {
    pub value: Transform,
    velocity: [f64; 3],
    focus: Option<Point>,
    prev_progress: f64,
    prev: Option<f64>,
}
impl Camera {
    pub fn step(&mut self, e: &Editor, c: &Cursor, t: f64, w: f64, h: f64) -> Transform {
        let cursor = sample(c, t).map(|(_, p)| p);
        let focus = |z: &Zoom| clamp_focus(if z.focus_mode == "auto" { cursor.unwrap_or(z.focus) } else { z.focus }, zoom_scale(z));
        let mut target = None;
        for pair in e.zoom_regions.windows(2) {
            if let [a, b] = pair
                && b.start_ms - a.end_ms <= 1500.
                && t >= a.end_ms
                && t <= a.end_ms + 1000.
            {
                let f = bezier(0.1, 0., 0.2, 1., (t - a.end_ms) / 1000.);
                let aa = transform(zoom_scale(a), focus(a), 1., w, h);
                let bb = transform(zoom_scale(b), focus(b), 1., w, h);
                target = Some(Transform { k: mix(aa.k, bb.k, f), x: mix(aa.x, bb.x, f), y: mix(aa.y, bb.y, f) });
                break;
            }
        }
        let dt = self.prev.map_or(0., |last| t - last);
        if target.is_none() {
            let mut best: Option<(&Zoom, f64)> = None;
            for (i, z) in e.zoom_regions.iter().enumerate() {
                let incoming = i.checked_sub(1).and_then(|j| e.zoom_regions.get(j)).filter(|prev| z.start_ms - prev.end_ms <= 1500.);
                let outgoing = e.zoom_regions.get(i + 1).filter(|next| next.start_ms - z.end_ms <= 1500.);
                if outgoing.is_some() && t > z.end_ms {
                    continue;
                }
                if incoming.is_some_and(|prev| t < prev.end_ms + 1000.) {
                    continue;
                }
                let v = if incoming.is_some_and(|prev| t > prev.end_ms + 1000. && t < z.start_ms) { 1. } else { strength(z, t) };
                if v > 0. && best.is_none_or(|(other, bv)| v > bv || (v == bv && z.start_ms > other.start_ms)) {
                    best = Some((z, v));
                }
            }
            if let Some((z, v)) = best {
                let mut p = focus(z);
                if z.focus_mode == "auto" {
                    let prev = self.focus.unwrap_or(p);
                    if v >= 0.999 || v < self.prev_progress {
                        let dist = ((p.cx - prev.cx).powi(2) + (p.cy - prev.cy).powi(2)).sqrt();
                        let base = 0.1 + 0.15 * (dist / 0.15).min(1.);
                        let f = 1. - (1. - base).powf(dt / 25.);
                        p = Point { cx: mix(prev.cx, p.cx, f), cy: mix(prev.cy, p.cy, f) };
                    }
                    self.focus = Some(p);
                } else {
                    self.focus = None;
                }
                self.prev_progress = v;
                target = Some(transform(zoom_scale(z), p, v, w, h));
            }
        }
        let target = target.unwrap_or_default();
        if self.prev.is_none() || dt <= 0. || dt > 80. {
            self.value = target;
            self.velocity = [0.; 3];
        } else {
            self.value.k = spring(self.value.k, target.k, &mut self.velocity[0], dt);
            self.value.x = spring(self.value.x, target.x, &mut self.velocity[1], dt);
            self.value.y = spring(self.value.y, target.y, &mut self.velocity[2], dt);
        }
        self.prev = Some(t);
        self.value
    }
}
fn spring(value: f64, target: f64, velocity: &mut f64, ms: f64) -> f64 {
    if (target - value).abs() <= 0.0005 && velocity.abs() <= 0.015 {
        *velocity = 0.;
        return target;
    }
    // Analytic overdamped oscillator, matching the upstream spring's 320/40/0.92 configuration.
    let root = (40_f64 * 40. - 4. * 0.92 * 320.).sqrt();
    let r1 = (-40. + root) / (2. * 0.92);
    let r2 = (-40. - root) / (2. * 0.92);
    let y = value - target;
    let a = (*velocity - r2 * y) / (r1 - r2);
    let b = y - a;
    let out = target + a * (r1 * ms / 1000.).exp() + b * (r2 * ms / 1000.).exp();
    if (value <= target && out > target) || (value >= target && out < target) {
        *velocity = 0.;
        target
    } else {
        *velocity = (out - value) * 1000. / ms;
        out
    }
}
/// Offline 240 Hz spring, restarted for each visible run, independent of output frame rate.
/// Coordinates use the same Float32 resampling as OpenScreen's native cursor path.
pub struct CursorPath {
    runs: Vec<Vec<(f32, f32, f32)>>,
}
impl CursorPath {
    pub fn new(c: &Cursor, smoothing: f64) -> Self {
        let u = smoothing.clamp(0., 1.);
        let (k, d, m) = if u <= 0.5 {
            (760. - u * 840., 34. + u * 48., 0.55 + u * 0.9)
        } else {
            let f = (u - 0.5) / 1.5;
            (340. - f * 180., 58. + f * 22., 1. + f * 0.35)
        };
        let mut runs = Vec::new();
        if u > 0. {
            for visible in c.samples.split(|s| !s.visible).filter(|r| !r.is_empty()) {
                let (Some(first), Some(last)) = (visible.first(), visible.last()) else { continue };
                // parse() bounds all times to two hours: at most 1,728,002 points in total.
                let count = ((last.time_ms - first.time_ms) * 240. / 1000.).round().max(1.) as usize;
                let mut run = Vec::with_capacity(count.saturating_add(1));
                let mut x = f64::from(first.cx as f32);
                let mut y = f64::from(first.cy as f32);
                let (mut vx, mut vy) = (0., 0.);
                for i in 0..=count {
                    let t = if i == count { last.time_ms } else { first.time_ms + i as f64 * 1000. / 240. };
                    let j = visible.partition_point(|s| s.time_ms <= t).saturating_sub(1);
                    let Some(a) = visible.get(j) else { continue };
                    let b = visible.get(j + 1).unwrap_or(a);
                    let f = if b.time_ms > a.time_ms { ((t - a.time_ms) / (b.time_ms - a.time_ms)).clamp(0., 1.) } else { 0. };
                    let px = f64::from(mix(a.cx, b.cx, f) as f32);
                    let py = f64::from(mix(a.cy, b.cy, f) as f32);
                    if i > 0 {
                        vx += (-k * (x - px) - d * vx) / m / 240.;
                        vy += (-k * (y - py) - d * vy) / m / 240.;
                        x += vx / 240.;
                        y += vy / 240.;
                    }
                    run.push((t as f32, x as f32, y as f32));
                }
                runs.push(run);
            }
        }
        Self { runs }
    }
    pub fn at(&self, c: &Cursor, t: f64) -> Option<Point> {
        let (s, raw) = sample(c, t)?;
        if !s.visible {
            return None;
        }
        let i = self.runs.partition_point(|r| r.last().is_some_and(|v| f64::from(v.0) < t));
        let Some(run) = self.runs.get(i).filter(|r| r.first().is_some_and(|v| t >= f64::from(v.0))) else { return Some(raw) };
        let j = run.partition_point(|v| f64::from(v.0) <= t).saturating_sub(1);
        let &(at, ax, ay) = run.get(j)?;
        let &(bt, bx, by) = run.get(j + 1).unwrap_or(&(at, ax, ay));
        let f = if bt > at { ((t - f64::from(at)) / f64::from(bt - at)).clamp(0., 1.) } else { 0. };
        Some(Point { cx: mix(f64::from(ax), f64::from(bx), f), cy: mix(f64::from(ay), f64::from(by), f) })
    }
}
pub fn bounce(c: &Cursor, t: f64, intensity: f64) -> f64 {
    let i = c.samples.partition_point(|s| s.time_ms <= t);
    for s in c.samples.get(..i).unwrap_or_default().iter().rev() {
        let age = t - s.time_ms;
        if age > 260. {
            break;
        }
        if s.interaction_type == "click" {
            let f = age / 260.;
            return if f < 0.38 {
                1. - (f / 0.38 * std::f64::consts::PI).sin() * intensity.clamp(0., 5.) / 5. * 0.24
            } else {
                1. + ((f - 0.38) / 0.62 * std::f64::consts::PI).sin() * intensity.clamp(0., 5.) / 5. * 0.16
            };
        }
    }
    1.
}
