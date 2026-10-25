//! Deterministic continuous camera motion and independently triggered, decaying hits.
use super::*;
fn value(e: &EffectInstance, id: &str, cx: &FxCtx, lo: f64, hi: f64, fallback: f64) -> f64 {
    let v = e.f64_at(id, cx.t);
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}
fn envelope(e: &EffectInstance, cx: &FxCtx) -> (f64, f64) {
    if !bv(e, "hit_mode") {
        return (1., cx.seconds);
    }
    let duration = value(e, "duration", cx, 20., 2000., 280.) / 1000.;
    let Some(trigger) = e.param("trigger") else { return (0., 0.) };
    let mut result = (0., 0.);
    // Keys are impulses, not an interpolated amplitude curve. Bounded for corrupt projects.
    for key in trigger.keyframes.iter().take(10000) {
        let dt = (i128::from(cx.t.0) - i128::from(key.time.0)) as f64 / filmcraft_time::TICKS_PER_SECOND as f64;
        if dt < 0. || dt >= duration {
            continue;
        }
        let gain = key.value.as_f64().filter(|v| v.is_finite()).unwrap_or(0.).clamp(0., 200.) / 100.;
        let x = dt / duration;
        let strength = gain * (1. - x).powi(3);
        if strength > result.0 {
            result = (strength, dt);
        }
    }
    result
}
fn mirror(x: f64, size: usize) -> f64 {
    if size <= 1 {
        return 0.5;
    }
    let span = size.saturating_sub(1).max(1) as f64;
    let phase = (x - 0.5).rem_euclid(span * 2.);
    0.5 + if phase > span { span * 2. - phase } else { phase }
}
pub(super) fn apply(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w == 0 || img.h == 0 {
        return;
    }
    let (env, phase) = envelope(e, cx);
    let strength = env * value(e, "strength", cx, 0., 300., 100.) / 100.;
    if strength <= 0. || !phase.is_finite() {
        return;
    }
    let phase = phase.rem_euclid(86400.);
    let px = if cx.px_scale.is_finite() { cx.px_scale.clamp(0.0001, 100.) as f64 } else { 1. };
    let amount = value(e, "amount", cx, 0., 1000., 0.) * strength * px;
    let rotation = value(e, "rotation", cx, 0., 45., 0.) * strength;
    let zoom = value(e, "zoom", cx, 0., 100., 0.) * strength / 100.;
    let freq = value(e, "frequency", cx, 0.1, 60., 1.3);
    let complexity = value(e, "complexity", cx, 1., 6., 4.) as usize;
    let seed = value(e, "seed", cx, 0., 10000., 0.) as u64;
    let ripple = value(e, "ripple", cx, 0., 100., 0.) * strength * px;
    let blur = value(e, "motion_blur", cx, 0., 100., 0.) / 100.;
    let fps = if cx.frame_rate.is_finite() { cx.frame_rate.clamp(1., 240.) } else { 30. };
    let (w, h) = (img.w, img.h);
    let (cx0, cy0) = (w as f64 / 2., h as f64 / 2.);
    let noise = |t: f64, ch: u64| {
        let (mut sum, mut amplitude, mut frequency, mut norm) = (0., 1., 1., 0.);
        for o in 0..complexity {
            sum += f64::from(noise1(t * freq * frequency, ch * 10 + o as u64, seed)) * amplitude;
            norm += amplitude;
            amplitude *= 0.5;
            frequency *= 2.;
        }
        sum / norm * 1.6
    };
    let mirrored = bv(e, "mirror_edges");
    super::distort::motion_blurred(img, if blur > 0. { 3 } else { 1 }, |image, k| {
        let t = phase + k * blur / fps;
        let (dx, dy) = (noise(t, 0) * amount, noise(t, 1) * amount);
        let angle = (noise(t, 2) * rotation).to_radians();
        let (sin, cos) = angle.sin_cos();
        let scale = (1. + zoom * (0.5 + 0.5 * noise(t, 3))).max(0.1);
        crate::effects::warp(image, |x, y| {
            let x = x - cx0 - dx;
            let y = y - cy0 - dy;
            let u = (cos * x + sin * y) / scale + cx0 + (y * 0.09 + t * 35.).sin() * ripple;
            let v = (-sin * x + cos * y) / scale + cy0;
            if mirrored { (mirror(u, w), mirror(v, h)) } else { (u, v) }
        });
    });
    let flash = value(e, "flash", cx, 0., 100., 0.) * strength / 100.;
    if flash > 0. {
        let flicker = 0.2 + 0.8 * (phase * 90.).cos().powi(2);
        let mix = (flash * flicker).min(1.) as f32;
        img.px.par_chunks_mut(4).for_each(|p| {
            let a = p[3];
            for v in &mut p[..3] {
                *v = (*v + (a - *v) * mix).max(0.);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(ms: i64) -> FxCtx<'static> {
        FxCtx {
            t: Tick(ms * filmcraft_time::TICKS_PER_SECOND / 1000),
            seconds: ms as f64 / 1000.,
            px_scale: 1.,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 30.,
            working: filmcraft_color::WorkingSpace::Rec709,
        }
    }
    fn picture() -> Image {
        let mut i = Image::filled(64, 40, [0., 0., 0., 1.]);
        for y in 0..40 {
            for x in 0..64 {
                let n = (y * 64 + x) * 4;
                i.px[n] = x as f32 / 64.;
                i.px[n + 1] = y as f32 / 40.;
                i.px[n + 2] = ((x / 5 + y / 5) % 2) as f32;
            }
        }
        i
    }
    fn render(e: &EffectInstance, ms: i64) -> Image {
        let mut i = picture();
        apply(&mut i, e, &context(ms));
        i
    }
    #[test]
    fn hits_are_impulses_without_motion_between_keys() {
        let mut e = filmcraft_project::effect::shaker::instance(1, true, Tick(filmcraft_time::TICKS_PER_SECOND), 27).unwrap();
        e.param_mut("trigger").unwrap().put_keyframe(Tick(3 * filmcraft_time::TICKS_PER_SECOND), ParamValue::Float(100.));
        for ms in [0, 999, 1280, 2000, 2999, 3280, 5000] {
            assert_eq!(render(&e, ms), picture(), "{ms}");
        }
        for ms in [1000, 1040, 3000, 3040] {
            let out = render(&e, ms);
            assert_ne!(out, picture(), "{ms}");
            assert_eq!(out, render(&e, ms));
            assert!(out.px.iter().all(|v| v.is_finite()));
            assert!(out.px.as_chunks::<4>().0.iter().all(|p| (p[3] - 1.).abs() < 0.0001));
        }
        e.param_mut("trigger").unwrap().keyframes.clear();
        assert_eq!(render(&e, 1000), picture());
    }
    #[test]
    fn continuous_presets_move_without_keys_and_custom_strength_can_disable() {
        for preset in 0..3 {
            let mut e = filmcraft_project::effect::shaker::instance(preset, false, Tick::ZERO, 31).unwrap();
            assert_ne!(render(&e, 100), render(&e, 600));
            e.param_mut("strength").unwrap().value = ParamValue::Float(0.);
            assert_eq!(render(&e, 100), picture());
        }
    }
    #[test]
    fn corrupt_values_and_extreme_key_times_do_not_panic() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300] {
            let mut e = filmcraft_project::effect::shaker::instance(1, true, Tick(i64::MIN), 0).unwrap();
            for p in e.params.values_mut() {
                if matches!(p.value, ParamValue::Float(_)) {
                    p.value = ParamValue::Float(bad);
                }
            }
            let mut i = picture();
            let mut c = context(0);
            c.t = Tick(i64::MAX);
            c.frame_rate = bad;
            c.px_scale = bad as f32;
            apply(&mut i, &e, &c);
            assert!(i.px.iter().all(|x| x.is_finite()));
            e.param_mut("hit_mode").unwrap().value = ParamValue::Bool(false);
            c.seconds = bad;
            apply(&mut i, &e, &c);
            assert!(i.px.iter().all(|x| x.is_finite()));
        }
    }
}
