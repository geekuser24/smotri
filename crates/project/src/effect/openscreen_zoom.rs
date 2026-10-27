//! Editable on/off zoom intervals on OpenScreen picture-in-picture nests.
use super::*;
use filmcraft_time::Tick;

pub const ID: &str = "openscreen_zoom";
pub(super) fn definition() -> EffectDef {
    let mut active = b("active", "Zoom Active", false);
    active.animatable = true;
    let mut d = video(
        ID,
        "OpenScreen Zoom",
        &["Video Effects", "Transform"],
        vec![
            active,
            f("scale", "Zoom Scale", 180., 100., 500., "%"),
            b("follow", "Follow recorded cursor", true),
            f("ease_in", "Zoom In Duration", 350., 0., 3000., "ms"),
            f("ease_out", "Zoom Out Duration", 350., 0., 3000., "ms"),
        ],
    );
    d.accelerated = false;
    d
}

fn bounded(e: &EffectInstance, key: &str, at: Tick, default: f64, max: f64) -> f64 {
    let v = e.f64_at(key, at);
    if v.is_finite() { v.clamp(0., max) } else { default }
}

/// Each boolean key starts a smooth transition. Before the first key zoom is off.
/// No history/cache is required, so seeking and export produce identical frames.
pub fn amount(e: &EffectInstance, t: Tick) -> f64 {
    if !e.enabled {
        return 0.;
    }
    let Some(p) = e.param("active") else { return 0. };
    if p.keyframes.is_empty() {
        return if p.value.as_bool() == Some(true) { 1. } else { 0. };
    }
    let mut from = 0.;
    let mut target = 0.;
    let mut start = Tick::ZERO;
    let mut duration = 0.;
    let sample = |from: f64, target: f64, start: Tick, duration: f64, at: Tick| {
        let u = if duration <= 0. { 1. } else { ((at.0.saturating_sub(start.0)) as f64 / filmcraft_time::TICKS_PER_SECOND as f64 / duration).clamp(0., 1.) };
        from + (target - from) * u * u * (3. - 2. * u)
    };
    for k in p.keyframes.iter().take(10_000).take_while(|k| k.time <= t) {
        let next = if k.value.as_bool() == Some(true) { 1. } else { 0. };
        if next == target {
            continue;
        }
        from = sample(from, target, start, duration, k.time);
        target = next;
        start = k.time;
        duration = bounded(e, if next > 0. { "ease_in" } else { "ease_out" }, k.time, 350., 3000.) / 1000.;
    }
    sample(from, target, start, duration, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boolean_keys_start_and_stop_zoom_with_seek_safe_ramps() {
        let mut e = definition().instance();
        let sec = |v: i64| Tick(v * filmcraft_time::TICKS_PER_SECOND);
        e.params.get_mut("active").unwrap().put_keyframe(sec(1), ParamValue::Bool(true));
        e.params.get_mut("active").unwrap().put_keyframe(sec(3), ParamValue::Bool(false));
        assert_eq!(amount(&e, sec(0)), 0.);
        assert_eq!(amount(&e, sec(1)), 0.);
        assert_eq!(amount(&e, sec(2)), 1.);
        assert_eq!(amount(&e, sec(3)), 1.);
        assert_eq!(amount(&e, sec(4)), 0.);
        let mid = sec(1) + Tick(filmcraft_time::TICKS_PER_SECOND * 175 / 1000);
        assert!((amount(&e, mid) - 0.5).abs() < 1e-6);
        e.enabled = false;
        assert_eq!(amount(&e, sec(2)), 0.);
        e.enabled = true;
        e.params.get_mut("ease_in").unwrap().value = ParamValue::Float(f64::NAN);
        assert!(amount(&e, Tick(i64::MAX)).is_finite());
    }
}
