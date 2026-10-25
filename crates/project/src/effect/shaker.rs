//! Original Shaker presets shared by manual controls, Jarvis and beat editing.
use super::*;
use filmcraft_time::Tick;
pub const PRESETS: &[&str] = &["Hit", "EDIT Flash", "Handheld"];
pub fn preset_values(preset: u32) -> Option<&'static [(&'static str, f64)]> {
    match preset {
        0 => Some(&[
            ("amount", 40.),
            ("rotation", 2.),
            ("zoom", 6.),
            ("frequency", 12.),
            ("complexity", 4.),
            ("motion_blur", 25.),
            ("flash", 0.),
            ("ripple", 0.),
            ("duration", 280.),
        ]),
        1 => Some(&[
            ("amount", 40.),
            ("rotation", 2.),
            ("zoom", 6.),
            ("frequency", 12.),
            ("complexity", 4.),
            ("motion_blur", 25.),
            ("flash", 55.),
            ("ripple", 10.),
            ("duration", 280.),
        ]),
        2 => Some(&[
            ("amount", 9.),
            ("rotation", 0.45),
            ("zoom", 4.),
            ("frequency", 1.3),
            ("complexity", 4.),
            ("motion_blur", 25.),
            ("flash", 0.),
            ("ripple", 0.),
            ("duration", 280.),
        ]),
        _ => None,
    }
}
/// Selecting a preset resets its tuning parameters; trigger keys and hit mode are preserved.
pub fn apply_preset(effect: &mut EffectInstance, preset: u32) -> bool {
    let Some(values) = preset_values(preset) else { return false };
    effect.params.insert("preset".into(), Param::new(ParamValue::Choice(preset)));
    for (id, value) in values {
        effect.params.insert((*id).into(), Param::new(ParamValue::Float(*value)));
    }
    true
}
pub fn instance(preset: u32, hit: bool, at: Tick, seed: u32) -> Option<EffectInstance> {
    let mut e = find_effect("shaker")?.instance();
    if !apply_preset(&mut e, preset) {
        return None;
    }
    e.params.insert("hit_mode".into(), Param::new(ParamValue::Bool(hit)));
    e.params.insert("seed".into(), Param::new(ParamValue::Float(f64::from(seed))));
    if hit {
        e.params.get_mut("trigger")?.put_keyframe(at, ParamValue::Float(100.));
    }
    Some(e)
}
pub(super) fn definition() -> EffectDef {
    let scalar = |id, label, value, min, max, unit| f(id, label, value, min, max, unit);
    let mut e = video(
        "shaker",
        "Shaker",
        &["Video Effects", "Transform"],
        vec![
            ch("preset", "Shake Preset", PRESETS, 2),
            b("hit_mode", "Hit Mode", false),
            scalar("trigger", "Hit Trigger", 100., 0., 200., "%"),
            scalar("duration", "Hit Duration", 280., 20., 2000., "ms"),
            scalar("strength", "Shake Strength", 100., 0., 300., "%"),
            scalar("amount", "Position Amount", 9., 0., 1000., "px"),
            scalar("rotation", "Rotation Amount", 0.45, 0., 45., "°"),
            scalar("zoom", "Zoom Amount", 4., 0., 100., "%"),
            scalar("frequency", "Frequency", 1.3, 0.1, 60., "Hz"),
            scalar("complexity", "Complexity", 4., 1., 6., ""),
            scalar("motion_blur", "Motion Blur", 25., 0., 100., "%"),
            scalar("flash", "Flash / Flicker", 0., 0., 100., "%"),
            scalar("ripple", "Ripple Amount", 0., 0., 100., "px"),
            scalar("seed", "Random Seed", 0., 0., 10000., ""),
            b("mirror_edges", "Mirror Edges", true),
        ],
    );
    e.accelerated = false;
    e
}
