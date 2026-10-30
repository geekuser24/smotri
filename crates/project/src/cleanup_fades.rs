//! Short anti-click fades created by silence/take cleanup. These belong to the clip edges,
//! not to fixed source times: revealing a trimmed handle must reveal its original sound.
use std::borrow::Cow;

use filmcraft_time::Tick;

use crate::{EffectInstance, Keyframe, Param, ParamValue, TrackItem, find_effect};

const FLAGS: &str = "__smotri_cleanup_edges";

/// Add an independent envelope without replacing the user's volume automation.
pub fn add(item: &mut TrackItem, fade_in: bool, fade_out: bool) {
    if !(fade_in || fade_out) || item.duration <= Tick::ZERO || item.frame_hold.is_some() {
        return;
    }
    let Some(mut effect) = find_effect("volume_a").map(|d| d.instance()) else { return };
    let flags = u32::from(fade_in) | (u32::from(fade_out) << 1);
    effect.params.insert(FLAGS.into(), Param::new(ParamValue::Choice(flags)));
    envelope(item, &mut effect, flags);
    item.effects.push(effect);
}

fn flags(item: &TrackItem, effect: &EffectInstance) -> Option<u32> {
    if effect.effect != "volume_a" || item.frame_hold.is_some() || !item.speed.is_finite() || item.speed <= 0.0 {
        return None;
    }
    if let Some(p) = effect.params.get(FLAGS) {
        return match p.value {
            ParamValue::Choice(n @ 1..=3) if p.keyframes.is_empty() => Some(n),
            _ => None,
        };
    }
    // Compatibility with cleanup in older projects: only its exact default Volume rack,
    // 3/4 linear keys at 0/-96 dB, with precisely 5 ms ramps. Other volume automation stays put.
    if item.reverse || !effect.masks.is_empty() || effect.post_fader || effect.essential || effect.layer.is_some() {
        return None;
    }
    let defaults = find_effect("volume_a")?.instance();
    if effect.params.len() != defaults.params.len() || effect.params.iter().any(|(name, p)| name != "level" && defaults.params.get(name) != Some(p)) {
        return None;
    }
    let level = effect.params.get("level")?;
    if level.value != ParamValue::Float(0.0) || !(3..=4).contains(&level.keyframes.len()) {
        return None;
    }
    let keys = &level.keyframes;
    if keys.iter().any(|k| *k != Keyframe::new(k.time, k.value.clone())) || keys.windows(2).any(|w| w[0].time >= w[1].time) {
        return None;
    }
    let first = keys.first()?;
    let last = keys.last()?;
    let fi = first.value == ParamValue::Float(-96.0);
    let fo = last.value == ParamValue::Float(-96.0);
    if !(fi || fo) || keys.len() != 2 + usize::from(fi) + usize::from(fo) {
        return None;
    }
    if (!fi && first.value != ParamValue::Float(0.0))
        || (!fo && last.value != ParamValue::Float(0.0))
        || keys.iter().skip(1).take(keys.len().saturating_sub(2)).any(|k| k.value != ParamValue::Float(0.0))
    {
        return None;
    }
    let ramp = Tick::from_seconds_f64(0.005 * item.speed);
    let near = |a: Tick, b: Tick| a.0.abs_diff(b.0) <= 1;
    if (fi && !near(keys.get(1)?.time - first.time, ramp)) || (fo && !near(last.time - keys.get(keys.len().checked_sub(2)?)?.time, ramp)) {
        return None;
    }
    Some(u32::from(fi) | (u32::from(fo) << 1))
}

fn envelope(item: &TrackItem, effect: &mut EffectInstance, flags: u32) {
    let fade = Tick::from_units(5, 1000).min(Tick(item.duration.0.max(0) / 2));
    if fade <= Tick::ZERO {
        return;
    }
    let Some(level) = effect.params.get_mut("level") else { return };
    level.keyframes.clear();
    let fi = flags & 1 != 0;
    let fo = flags & 2 != 0;
    level.put_keyframe(item.source_time_at(item.start), ParamValue::Float(if fi { -96.0 } else { 0.0 }));
    if fi {
        level.put_keyframe(item.source_time_at(item.start + fade), ParamValue::Float(0.0));
    }
    if fo {
        level.put_keyframe(item.source_time_at(item.end() - fade), ParamValue::Float(0.0));
    }
    level.put_keyframe(item.source_time_at(item.end()), ParamValue::Float(if fo { -96.0 } else { 0.0 }));
}

/// Resolve at playback too, covering older saved projects whose handles were already extended.
pub fn resolved<'a>(item: &TrackItem, effect: &'a EffectInstance) -> Cow<'a, EffectInstance> {
    let Some(flags) = flags(item, effect) else { return Cow::Borrowed(effect) };
    let mut e = effect.clone();
    e.params.insert(FLAGS.into(), Param::new(ParamValue::Choice(flags)));
    envelope(item, &mut e, flags);
    Cow::Owned(e)
}

/// Keep the visible keys in Effect Controls at the new edges after a trim.
pub fn refresh(item: &mut TrackItem) {
    let effects = std::mem::take(&mut item.effects);
    item.effects = effects
        .into_iter()
        .map(|e| match resolved(item, &e) {
            Cow::Owned(updated) => updated,
            Cow::Borrowed(_) => e,
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Project, SequenceSettings, TrackKind};
    use filmcraft_time::{FrameRate, TimeRange};

    fn clip() -> TrackItem {
        let mut p = Project::new("fade handles");
        let seq = p.new_sequence("source", SequenceSettings::default(), 1, 1, None);
        p.make_track_item(seq, TrackKind::Audio, Tick::from_units(2, 1), TimeRange::new(Tick::from_units(2, 1), Tick::from_units(2, 1)), FrameRate::FPS_25)
            .unwrap()
    }

    #[test]
    fn cleanup_fades_reveal_both_handles_in_old_and_new_projects() {
        for legacy in [false, true] {
            for (fi, fo) in [(true, false), (false, true), (true, true)] {
                let mut c = clip();
                add(&mut c, fi, fo);
                let mut e = c.effects.last().unwrap().clone();
                if legacy {
                    e.params.remove(FLAGS);
                }
                c.start -= Tick::from_units(1, 1);
                c.source_in -= Tick::from_units(1, 1);
                c.duration += Tick::from_units(2, 1);
                let fixed = resolved(&c, &e);
                for t in [1.5, 2.0, 3.0, 4.0, 4.5] {
                    assert_eq!(fixed.f64_at("level", Tick::from_seconds_f64(t)), 0.0, "legacy={legacy} fi={fi} fo={fo} t={t}");
                }
                assert_eq!(fixed.f64_at("level", c.source_time_at(c.start)), if fi { -96.0 } else { 0.0 });
                assert_eq!(fixed.f64_at("level", c.source_time_at(c.end())), if fo { -96.0 } else { 0.0 });
                let saved = serde_json::to_string(fixed.as_ref()).unwrap();
                let loaded: EffectInstance = serde_json::from_str(&saved).unwrap();
                assert_eq!(resolved(&c, &loaded).as_ref(), fixed.as_ref());
            }
        }
    }

    #[test]
    fn manual_volume_automation_and_hostile_markers_are_preserved() {
        let mut c = clip();
        add(&mut c, true, true);
        let mut e = c.effects.last().unwrap().clone();
        e.params.remove(FLAGS);
        let level = e.params.get_mut("level").unwrap();
        level.keyframes[1].value = ParamValue::Float(-6.0);
        assert!(matches!(resolved(&c, &e), Cow::Borrowed(_)));
        e.params.insert(FLAGS.into(), Param::new(ParamValue::Choice(u32::MAX)));
        assert!(matches!(resolved(&c, &e), Cow::Borrowed(_)));
        for speed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            c.speed = speed;
            assert!(matches!(resolved(&c, &e), Cow::Borrowed(_)));
        }
    }

    #[test]
    fn short_reverse_and_speed_changed_cleanup_fades_remain_bounded() {
        for speed in [0.5, 1.0, 2.0] {
            let mut c = clip();
            c.speed = speed;
            c.reverse = true;
            c.duration = Tick::from_units(8, 1000);
            add(&mut c, true, true);
            c.duration = Tick::from_units(1, 1);
            refresh(&mut c);
            let e = c.effects.last().unwrap();
            assert_eq!(e.f64_at("level", c.source_time_at(c.start + Tick::from_units(1, 2))), 0.0);
            assert_eq!(e.f64_at("level", c.source_time_at(c.end())), -96.0);
        }
    }
}
