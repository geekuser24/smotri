//! Parent-clip zoom transforms only the desktop and cursor layers of a PiP nest.
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{EffectInstance, Sequence};
use filmcraft_time::Tick;

pub(crate) fn matrix(e: &EffectInstance, seq: &Sequence, source_time: Tick, effect_time: Tick, output_scale: f32) -> Option<Affine> {
    let amount = filmcraft_project::effect::openscreen_zoom::amount(e, effect_time);
    let scale = e.f64_at("scale", effect_time);
    if amount <= 0. || !scale.is_finite() || scale <= 100. {
        return None;
    }
    let full = scale.clamp(100., 500.) / 100.;
    let zoom = 1. + (full - 1.) * amount;
    let center = Vec2::new(f64::from(seq.settings.width) / 2., f64::from(seq.settings.height) / 2.);
    let follow = e.param("follow").is_none_or(|p| p.value_at(effect_time).as_bool() == Some(true));
    let focus = if follow {
        seq.video_tracks
            .iter()
            .filter(|tr| tr.enabled && tr.name.starts_with("Cursor "))
            .filter_map(|tr| tr.item_at(source_time))
            .filter(|c| c.enabled)
            .find(|c| c.effect("opacity").is_some_and(|e| e.f64_at("opacity", c.effect_time_at(source_time)) > 0.))
            .and_then(|c| c.effect("motion").map(|e| e.vec2_at("position", c.effect_time_at(source_time))))
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .unwrap_or(center)
    } else {
        center
    };
    // Keep the output canvas covered at the edges; 100% is always neutral.
    let fx = focus.x.clamp(center.x / full, center.x * 2. - center.x / full);
    let fy = focus.y.clamp(center.y / full, center.y * 2. - center.y / full);
    let px = f64::from(output_scale);
    Some(Affine::translate((center.x - fx * full) * amount * px, (center.y - fy * full) * amount * px).then_apply(&Affine::scale(zoom, zoom)))
}

pub(crate) fn applies(track_name: &str) -> bool {
    track_name == "Desktop" || track_name.starts_with("Cursor ")
}
