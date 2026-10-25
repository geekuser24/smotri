//! Deterministic contextual editing for voice; shared by UI, CLI and control channel.
use crate::commands::{CommandSpec, has_seq};
use crate::{EngineError, Result, Session};
use filmcraft_project::{ClipId, EffectKind, TrackKind, effect::find_effect};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

fn err(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}
fn normalized(s: &str) -> String {
    s.to_lowercase()
        .replace('ё', "е")
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Explicit targets never silently fall back to the selection. Ordinals are chronological,
/// one-based within a track. With no track use the selected clip's track, else V1/A1.
pub fn targets(s: &Session, p: &Value, video_only: bool) -> Result<Vec<ClipId>> {
    if !p.is_object() {
        return Err(err("parameters must be an object"));
    }
    if p.get("target").is_some_and(|t| !t.is_object()) {
        return Err(err("target must be an object"));
    }
    let q = s.active_sequence().ok_or_else(|| err("no sequence is open"))?;
    let target = p.get("target").unwrap_or(&Value::Null);
    if let Some(t) = target.as_object() {
        if t.keys().any(|k| !matches!(k.as_str(), "kind" | "track" | "ordinal" | "name" | "atPlayhead")) {
            return Err(err("unknown target field"));
        }
        if t.get("kind").is_some_and(|v| !matches!(v.as_str(), Some("video" | "audio")))
            || t.get("name").is_some_and(|v| !v.is_string())
            || (t.contains_key("name") && t.contains_key("ordinal"))
        {
            return Err(err("invalid or conflicting clip target"));
        }
    }
    let explicit = target.as_object().is_some_and(|v| !v.is_empty());
    if !explicit && !s.state.selection.is_empty() {
        let ids: Vec<_> = s
            .state
            .selection
            .iter()
            .copied()
            .filter(|id| q.find_item(*id).is_some_and(|(tr, _)| !video_only || q.track(tr).is_some_and(|t| t.kind == TrackKind::Video)))
            .collect();
        if ids.is_empty() {
            return Err(err("the selection has no suitable clips"));
        }
        return Ok(ids);
    }
    let audio = target.get("kind").and_then(Value::as_str) == Some("audio");
    if audio && video_only {
        return Err(err("Select a video clip for this effect"));
    }
    let tracks = if audio || (!video_only && q.video_tracks.iter().all(|t| t.items.is_empty())) { &q.audio_tracks } else { &q.video_tracks };
    let requested = target.get("track").and_then(Value::as_u64);
    if target.get("track").is_some() && requested.is_none() {
        return Err(err("invalid track number"));
    }
    let track = if let Some(n) = requested {
        let index = usize::try_from(n).ok().and_then(|n| n.checked_sub(1)).ok_or_else(|| err("tracks start at 1"))?;
        tracks.get(index).ok_or_else(|| err("no such track"))?
    } else {
        s.state
            .selection
            .first()
            .and_then(|c| q.find_item(*c))
            .and_then(|(t, _)| tracks.iter().find(|x| x.id == t))
            .filter(|t| target.get("ordinal").is_some() || target.get("name").is_some() || t.items.iter().any(|c| c.range().contains(s.playhead())))
            .or_else(|| {
                if target.get("ordinal").is_none() && target.get("name").is_none() {
                    tracks.iter().rev().find(|t| t.items.iter().any(|c| c.range().contains(s.playhead())))
                } else {
                    None
                }
            })
            .or_else(|| tracks.first())
            .ok_or_else(|| err("no suitable track"))?
    };
    let mut clips: Vec<_> =
        if requested.is_none() && target.get("name").is_some() { tracks.iter().flat_map(|t| t.items.iter()).collect() } else { track.items.iter().collect() };
    clips.sort_by_key(|c| (c.start, c.id));
    let ids = if let Some(n) = target.get("ordinal") {
        let idx = n.as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)).ok_or_else(|| err("clip numbers start at 1"))?;
        vec![clips.get(idx).ok_or_else(|| err("no clip with that number on the track"))?.id]
    } else if let Some(name) = target.get("name").and_then(Value::as_str) {
        let name = normalized(name);
        if name.is_empty() || name.len() > 512 {
            return Err(err("invalid clip name"));
        }
        let exact: Vec<_> = clips.iter().filter(|c| normalized(&c.name) == name).map(|c| c.id).collect();
        let found = if exact.is_empty() { clips.iter().filter(|c| normalized(&c.name).contains(&name)).map(|c| c.id).collect() } else { exact };
        if found.len() != 1 {
            return Err(err("clip name is missing or ambiguous; specify track and clip number"));
        }
        found
    } else {
        vec![clips.iter().find(|c| c.range().contains(s.playhead())).ok_or_else(|| err("there is no clip under the playhead on that track"))?.id]
    };
    Ok(ids)
}
fn editable(s: &Session, ids: &[ClipId]) -> Result<()> {
    let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
    if ids.iter().any(|c| q.find_item(*c).is_none_or(|(t, _)| q.track(t).is_none_or(|t| t.locked))) {
        return Err(err("a target track is locked or a clip is missing"));
    }
    Ok(())
}
fn select(s: &mut Session, p: &Value) -> Result<Value> {
    // A plain "select clip" always uses the playhead, not the previous selection.
    if !p.is_object() {
        return Err(err("parameters must be an object"));
    }
    let mut p = p.clone();
    if p.get("target").is_none() {
        p["target"] = json!({"atPlayhead":true});
    }
    let ids = targets(s, &p, false)?;
    s.execute("timeline.select", json!({"clips":ids,"linked":false}))?;
    if p.get("reveal").and_then(Value::as_bool).unwrap_or(false) {
        let at = s.active_sequence().and_then(|q| ids.first().and_then(|c| q.find_item(*c))).map(|(_, c)| c.start);
        if let Some(at) = at {
            s.execute("playhead.set", json!({"time":at.0}))?;
        }
    }
    Ok(json!({"clips":ids}))
}
fn effect(s: &mut Session, p: &Value) -> Result<Value> {
    let id = p.get("effect").and_then(Value::as_str).ok_or_else(|| err("name an effect"))?;
    let def = find_effect(id).ok_or_else(|| err("unknown effect"))?;
    let ids = targets(s, p, def.kind != EffectKind::Audio)?;
    editable(s, &ids)?;
    s.execute("effects.apply", json!({"clips":ids,"effect":id}))
}
fn select_effect(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p, false)?;
    if ids.len() != 1 {
        return Err(err("Select one clip to select its effect"));
    }
    let effect = p.get("effect").and_then(Value::as_str).ok_or_else(|| err("Name an effect"))?;
    s.execute("effects.select", json!({"clip":ids[0],"effect":effect}))
}
fn parameter(s: &mut Session, p: &Value) -> Result<Value> {
    let effect = p.get("effect").and_then(Value::as_str);
    let param = p.get("param").and_then(Value::as_str).ok_or_else(|| err("name a parameter"))?;
    let value = p.get("value").ok_or_else(|| err("give a parameter value"))?;
    let op = p.get("op").and_then(Value::as_str).unwrap_or("set");
    if !matches!(op, "set" | "add" | "percent") {
        return Err(err("invalid parameter operation"));
    }
    let ids = targets(s, p, effect.and_then(find_effect).is_none_or(|e| e.kind != EffectKind::Audio))?;
    editable(s, &ids)?;
    s.edit_group("Voice: change effect parameter", |s| {
        for clip in &ids {
            let selected = s.state.selected_effect.as_ref().filter(|e| e.clip == *clip);
            let it = s.active_sequence().and_then(|q| q.find_item(*clip)).map(|(_, c)| c).ok_or_else(|| err("clip missing"))?;
            let existing = if let Some(id) = effect {
                let indices: Vec<_> = it.effects.iter().enumerate().filter(|(_, e)| e.effect == id).map(|(i, _)| i).collect();
                if indices.len() > 1 {
                    selected
                        .and_then(|e| e.index(&it.effects))
                        .filter(|i| indices.contains(i))
                        .ok_or_else(|| err("multiple instances: select the effect first"))?
                        .into()
                } else {
                    indices.first().copied()
                }
            } else {
                selected.and_then(|e| e.index(&it.effects))
            };
            let index = if let Some(index) = existing {
                index
            } else {
                let id = effect.ok_or_else(|| err("select an effect or name it"))?;
                if p.get("ensure").and_then(Value::as_bool) != Some(true) {
                    return Err(err("effect is not on this clip"));
                }
                s.execute("effects.apply", json!({"clips":[clip],"effect":id}))?;
                s.active_sequence()
                    .and_then(|q| q.find_item(*clip))
                    .and_then(|(_, c)| c.effects.iter().position(|e| e.effect == id))
                    .ok_or_else(|| err("could not add effect"))?
            };
            let it = s.active_sequence().and_then(|q| q.find_item(*clip)).map(|(_, c)| c).ok_or_else(|| err("clip missing"))?;
            let e = it.effects.get(index).ok_or_else(|| err("effect missing"))?;
            let def = e
                .def()
                .and_then(|d| d.params.iter().find(|d| d.id == param || normalized(d.label) == normalized(param)))
                .ok_or_else(|| err("unknown effect parameter"))?;
            let val = if op == "set" {
                value.clone()
            } else {
                let n = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| err("finite number required"))?;
                let old = e.f64_at(def.id, it.source_time_at(s.playhead().clamp(it.start, (it.end() - Tick(1)).max(it.start))));
                json!(if op == "percent" { old * (1.0 + n / 100.0) } else { old + n })
            };
            let pid = def.id;
            s.execute(
                "effects.setParam",
                json!({"clip":clip,"effect":index,"param":pid,"value":val,"keyframe":p.get("keyframe").and_then(Value::as_bool).unwrap_or(false)}),
            )?;
        }
        Ok(json!({"clips":ids,"param":param}))
    })
}
/// Create a real adjustment source and place it above every overlapping video layer.
/// Grouping makes source creation, a new track, placement and effects one Undo operation.
fn place_adjustment(s: &mut Session, start: Tick, duration: Tick) -> Result<Value> {
    use filmcraft_project::{ItemKind, Label};
    use filmcraft_time::TimeRange;
    if start < Tick::ZERO || duration <= Tick::ZERO || start.0.checked_add(duration.0).is_none() {
        return Err(err("invalid adjustment layer range"));
    }
    let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
    let settings = q.settings.clone();
    let range = TimeRange::new(start, duration);
    let bottom = q.video_tracks.iter().rposition(|t| t.items.iter().any(|c| c.range().overlaps(&range))).map_or(0, |i| i + 1);
    let track = q
        .video_tracks
        .iter()
        .skip(bottom)
        .find(|t| !t.locked && t.enabled && !t.items.iter().any(|c| c.range().overlaps(&range)))
        .map(|t| json!(t.id.0))
        .unwrap_or(json!("new"));
    let item = s.edit("Adjustment Layer", |pr, _| {
        Ok(pr.add_item(
            "Adjustment Layer",
            Label::Lavender,
            ItemKind::AdjustmentLayer { width: settings.width, height: settings.height, rate: settings.frame_rate, duration },
            None,
        ))
    })?;
    s.execute("timeline.place", json!({"item":item.0,"track":track,"time":start.0,"duration":duration.0,"sourceIn":0,"video":true,"audio":false}))
}
fn adjustment(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.as_object().is_some_and(|p| p.keys().all(|k| k == "seconds")) {
        return Err(err("adjustment layer accepts only seconds"));
    }
    let rate = s.sequence_rate();
    let start = rate.snap_nearest(s.playhead());
    let duration = if let Some(value) = p.get("seconds") {
        let seconds = value.as_f64().filter(|v| v.is_finite() && *v > 0.0 && *v <= 3600.0).ok_or_else(|| err("duration must be between 0 and 3600 seconds"))?;
        rate.snap_nearest(Tick::from_seconds_f64(seconds)).max(rate.frame_duration())
    } else {
        let end = targets(s, &json!({}), true)
            .ok()
            .and_then(|ids| {
                let q = s.active_sequence()?;
                ids.iter().filter_map(|id| q.find_item(*id)).map(|(_, c)| c.end()).max()
            })
            .filter(|end| *end > start);
        end.map(|end| end - start).unwrap_or(rate.snap_nearest(Tick(5 * TICKS_PER_SECOND)))
    };
    s.edit_group("Voice: adjustment layer", |s| place_adjustment(s, start, duration))
}

fn shaker_hit(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.as_object().is_some_and(|o| o.keys().all(|k| k == "target")) {
        return Err(err("Shaker hit accepts only a clip target"));
    }
    s.edit_group("Shaker Hit", |s| {
        let target = p.get("target").cloned().unwrap_or(json!({}));
        parameter(s, &json!({"target":target,"effect":"shaker","param":"hit_mode","value":true}))?;
        parameter(s, &json!({"target":target,"effect":"shaker","param":"trigger","value":100.,"keyframe":true}))
    })
}
fn shake(s: &mut Session, p: &Value) -> Result<Value> {
    let preset = p.get("preset").and_then(Value::as_str).unwrap_or("hit");
    let preset = if preset == "edt" { "edit" } else { preset };
    if !matches!(preset, "hit" | "edit" | "handheld") {
        return Err(err("unknown shake preset"));
    }
    let placement = p.get("placement").and_then(Value::as_str).unwrap_or(if preset == "handheld" { "clip" } else { "layer" });
    if !matches!(placement, "clip" | "layer") {
        return Err(err("placement must be clip or layer"));
    }
    let ids = targets(s, p, true)?;
    if placement == "layer" {
        let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
        let clips: Vec<_> = ids.iter().filter_map(|id| q.find_item(*id).map(|(_, c)| c)).collect();
        let first = clips.iter().map(|c| c.start).min().ok_or_else(|| err("no video clip"))?;
        let end = clips.iter().map(|c| c.end()).max().ok_or_else(|| err("no video clip"))?;
        let start = if preset == "handheld" || !clips.iter().any(|c| c.range().contains(s.playhead())) { first } else { s.playhead() };
        let duration = if preset == "handheld" {
            end - start
        } else {
            (end - start).min(q.settings.frame_rate.snap_nearest(Tick(TICKS_PER_SECOND * 280 / 1000)).max(q.settings.frame_rate.frame_duration()))
        };
        return s.edit_group("Voice: shake adjustment layer", |s| {
            let placed = place_adjustment(s, start, duration)?;
            let clips = placed.get("clips").cloned().ok_or_else(|| err("layer placement returned no clips"))?;
            s.execute("timeline.select", json!({"clips":clips,"linked":false}))?;
            shake(s, &json!({"preset":preset,"placement":"clip"}))?;
            Ok(placed)
        });
    }
    editable(s, &ids)?;
    let ph = s.playhead();
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0);
    s.edit_sequence("Voice: camera shake", |q, _, _| {
        let fd = q.settings.frame_rate.frame_duration();
        for id in &ids {
            let (_, c) = q.find_item_mut(*id).ok_or_else(|| err("clip missing"))?;
            if c.reverse || c.frame_hold.is_some() {
                return Err(err("shake preset requires a forward clip without a frame hold"));
            }
            let start = if c.range().contains(ph) { ph.min((c.end() - fd).max(c.start)) } else { c.start };
            let choice = match preset {
                "hit" => 0,
                "edit" => 1,
                _ => 2,
            };
            let e = filmcraft_project::effect::shaker::instance(
                choice,
                preset != "handheld",
                c.effect_time_at(start),
                ((salt ^ id.0.wrapping_mul(7919)) % 10000) as u32,
            )
            .ok_or_else(|| err("Shaker unavailable"))?;
            let pos = c.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(c.effects.len());
            c.effects.insert(pos, e);
        }
        Ok(json!({"clips":ids,"preset":preset}))
    })
}
/// Keep the normal ripple behavior when possible. If another synced layer blocks it,
/// close only the selected tracks, without changing their persistent sync-lock settings.
fn ripple_delete(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.as_object().is_some_and(|p| p.is_empty()) {
        return Err(err("voice ripple delete takes the current selection and no parameters"));
    }
    match s.execute("edit.rippleDelete", json!({})) {
        Err(EngineError::Edit(filmcraft_edit::EditError::SyncLockConflict(_))) => {}
        result => return result,
    }
    let ids = s.state.selection.clone();
    let gap = s.state.gap_selection.filter(|_| ids.is_empty());
    s.edit_sequence("Ripple Delete", |q, _, st| {
        let tracks: Vec<_> = if let Some(g) = gap { vec![g.track] } else { ids.iter().filter_map(|id| q.find_item(*id).map(|(t, _)| t)).collect() };
        if tracks.is_empty() || tracks.iter().any(|t| q.track(*t).is_none_or(|t| t.locked)) {
            return Err(err("Select unlocked clips or a gap before ripple deleting"));
        }
        let locks: Vec<_> = q.all_tracks().map(|t| (t.id, t.sync_lock)).collect();
        for tr in q.all_tracks_mut() {
            if !tracks.contains(&tr.id) {
                tr.sync_lock = false;
            }
        }
        let result = if let Some(g) = gap {
            filmcraft_edit::close_gap(q, g.track, g.range.start).map(|_| vec![g.range])
        } else {
            filmcraft_edit::ripple_delete_items(q, &ids)
        };
        for (id, sync) in locks {
            if let Some(t) = q.track_mut(id) {
                t.sync_lock = sync;
            }
        }
        let spans = result?;
        if st.ripple_sequence_markers {
            for span in spans.iter().rev() {
                crate::sequence_tools::ripple_markers(&mut q.markers, span.end(), -span.duration);
            }
        }
        st.selection.clear();
        st.gap_selection = None;
        Ok(json!({"scope":"selectedTracks"}))
    })
}
/// Only synchronous project/state operations may enter an atomic spoken chain.
pub fn plan_command_allowed(id: &str) -> bool {
    matches!(
        id,
        "voice.selectClip"
            | "voice.selectEffect"
            | "voice.effect"
            | "voice.parameter"
            | "voice.shake"
            | "voice.adjustmentLayer"
            | "voice.splitHalf"
            | "voice.duration"
            | "voice.reorder"
            | "markers.add"
            | "markers.goNext"
            | "markers.goPrev"
            | "markers.clearCurrent"
            | "voice.rippleDelete"
            | "edit.clear"
            | "playhead.step"
            | "timeline.selectNextClip"
            | "timeline.selectPrevClip"
    )
}
fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let obj = p.as_object().ok_or_else(|| err("plan must be an object"))?;
    if obj.keys().any(|k| k != "steps") {
        return Err(err("unknown plan field"));
    }
    let steps = p.get("steps").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 16).ok_or_else(|| err("a voice chain needs 1–16 steps"))?;
    if p.to_string().len() > 32768 {
        return Err(err("voice chain is too large"));
    }
    for step in steps {
        let obj = step.as_object().ok_or_else(|| err("invalid voice step"))?;
        if obj.keys().any(|k| !matches!(k.as_str(), "id" | "params"))
            || !step.get("id").and_then(Value::as_str).is_some_and(plan_command_allowed)
            || !step.get("params").is_some_and(Value::is_object)
        {
            return Err(err("unsupported command in voice chain"));
        }
    }
    let journal = s.journal.clone();
    let result = s.edit_group("Jarvis: command chain", |s| {
        let mut results = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let id = step.get("id").and_then(Value::as_str).ok_or_else(|| err("missing command"))?;
            let params = step.get("params").cloned().ok_or_else(|| err("missing parameters"))?;
            results.push(s.execute(id, params).map_err(|e| err(format!("Step {}: {e}. Chain rolled back.", i + 1)))?);
        }
        Ok(json!({"results":results}))
    });
    // The outer command is the replay unit; never retain nested partial commands.
    s.journal = journal;
    result
}
fn primary_target(s: &Session, p: &Value) -> Result<ClipId> {
    let audio = p.get("target").and_then(|t| t.get("kind")).and_then(Value::as_str) == Some("audio");
    let audio = audio
        || (p.get("target").and_then(Value::as_object).is_none_or(|t| t.is_empty())
            && !s.state.selection.is_empty()
            && s.active_sequence().is_some_and(|q| {
                s.state.selection.iter().all(|id| q.find_item(*id).is_some_and(|(t, _)| q.track(t).is_some_and(|t| t.kind == TrackKind::Audio)))
            }));
    let ids = targets(s, p, !audio)?;
    if ids.len() != 1 {
        return Err(err("Select one clip or specify its number"));
    }
    ids.first().copied().ok_or_else(|| err("clip missing"))
}
fn linked_targets(s: &Session, ids: &[ClipId]) -> Result<Vec<ClipId>> {
    let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
    let links: Vec<_> = ids.iter().filter_map(|id| q.find_item(*id).and_then(|(_, c)| c.link)).collect();
    let all: Vec<_> =
        q.all_tracks().flat_map(|t| &t.items).filter(|c| ids.contains(&c.id) || c.link.is_some_and(|l| links.contains(&l))).map(|c| c.id).collect();
    editable(s, &all)?;
    Ok(all)
}
fn duration(s: &mut Session, p: &Value) -> Result<Value> {
    let seconds = p
        .get("seconds")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 86400.0)
        .ok_or_else(|| err("duration must be between 0 and 86400 seconds"))?;
    let id = primary_target(s, p)?;
    linked_targets(s, &[id])?;
    let requested = s.sequence_rate().snap_nearest(Tick::from_seconds_f64(seconds)).max(s.sequence_rate().frame_duration());
    let old = s.active_sequence().and_then(|q| q.find_item(id)).map(|(_, c)| c.duration).ok_or_else(|| err("clip missing"))?;
    s.edit_group("Voice: clip duration", |s| {
        let follow = s.state.linked_selection;
        s.state.linked_selection = true;
        let result = s.execute("timeline.trim", json!({"clip":id,"edge":"out","mode":"regular","delta":(requested-old).0}));
        s.state.linked_selection = follow;
        result?;
        if s.active_sequence().and_then(|q| q.find_item(id)).is_none_or(|(_, c)| c.duration != requested) {
            return Err(err("Requested duration does not fit available media or the next clip"));
        }
        Ok(json!({"clip":id,"duration":requested.0}))
    })
}
fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let id = primary_target(s, p)?;
    let position = p
        .get("position")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_sub(1))
        .ok_or_else(|| err("positions start at 1"))?;
    let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
    let (track, _) = q.find_item(id).ok_or_else(|| err("clip missing"))?;
    let tr = q.track(track).ok_or_else(|| err("track missing"))?;
    if tr.items.len() > 10000 {
        return Err(err("too many clips to reorder"));
    }
    let mut clips: Vec<_> = tr.items.iter().collect();
    clips.sort_by_key(|c| (c.start, c.id));
    if position >= clips.len() {
        return Err(err("no such position on this track"));
    }
    let from = clips.iter().position(|c| c.id == id).ok_or_else(|| err("clip missing"))?;
    if from == position {
        return Ok(json!({"moved":[]}));
    }
    let lo = from.min(position);
    let hi = from.max(position);
    let block = clips.get(lo..=hi).ok_or_else(|| err("invalid reorder range"))?;
    let mut order = block.to_vec();
    let removed = order.remove(from - lo);
    order.insert(position - lo, removed);
    let mut at = block.first().map(|c| c.start).ok_or_else(|| err("empty reorder range"))?;
    let mut moves = Vec::new();
    let ids: Vec<_> = order.iter().map(|c| c.id).collect();
    linked_targets(s, &ids)?;
    for (i, c) in order.iter().enumerate() {
        moves.push(json!({"clip":c.id,"track":track,"time":at.0}));
        let gap = block.get(i).zip(block.get(i + 1)).map(|(a, b)| b.start - a.end()).unwrap_or(Tick::ZERO);
        if gap < Tick::ZERO {
            return Err(err("overlapping clips cannot be reordered"));
        }
        at = Tick(at.0.checked_add(c.duration.0).and_then(|v| v.checked_add(gap.0)).ok_or_else(|| err("reorder time overflow"))?);
    }
    s.edit_group("Voice: reorder clips", |s| {
        let result = s.execute("timeline.move", json!({"moves":moves,"linked":true,"insert":false}))?;
        if result.get("overwritten").and_then(Value::as_array).is_some_and(|v| !v.is_empty()) {
            return Err(err("Cannot reorder: linked audio would overwrite another clip"));
        }
        Ok(result)
    })
}
fn split_half(s: &mut Session, p: &Value) -> Result<Value> {
    let primary = primary_target(s, p)?;
    let ids = linked_targets(s, &[primary])?;
    let q = s.active_sequence().ok_or_else(|| err("no sequence"))?;
    let (_, clip) = q.find_item(primary).ok_or_else(|| err("clip missing"))?;
    let fd = q.settings.frame_rate.frame_duration();
    if fd.0 <= 0 {
        return Err(err("invalid frame duration"));
    }
    let half = clip.duration.0 / fd.0 / 2;
    if half < 1 {
        return Err(err("clip is too short to split in half"));
    }
    let offset = half.checked_mul(fd.0).ok_or_else(|| err("split time overflow"))?;
    let at = Tick(clip.start.0.checked_add(offset).ok_or_else(|| err("split time overflow"))?);
    s.edit_sequence("Jarvis: split clip in half", |q, ctx, _| {
        let cuts = filmcraft_edit::razor_items(q, &ids, at, ctx);
        if cuts.is_empty() {
            return Err(err("clip could not be split"));
        }
        Ok(json!({"clips":cuts,"time":at.0}))
    })
}
pub fn commands() -> Vec<CommandSpec> {
    type Run = fn(&mut Session, &Value) -> Result<Value>;
    [("voice.shakerHit","Shaker Hit",shaker_hit as Run),("voice.duration","Voice Clip Duration",duration as Run),("voice.reorder","Voice Reorder Clips",reorder as Run),("voice.adjustmentLayer","Voice Adjustment Layer",adjustment as Run),("voice.plan","Voice Command Chain",plan as Run),("voice.splitHalf","Voice Split Clip in Half",split_half as Run),("voice.rippleDelete","Voice Ripple Delete",ripple_delete as Run),("voice.selectClip","Voice Select Clip",select as Run),("voice.selectEffect","Voice Select Effect",select_effect as Run),("voice.effect","Voice Apply Effect",effect as Run),("voice.parameter","Voice Effect Parameter",parameter as Run),("voice.shake","Voice Shake Preset",shake as Run)].into_iter().map(|(id,label,run)|CommandSpec{id,label,menu:&[],shortcut:None,params:if id == "voice.plan" {r#"{"steps":[{"id":str,"params":object}]}"#} else {r#"{"target":{"kind":"video|audio","track":1,"ordinal":1,"name":str}?,"effect":str?,"param":str?,"value":any?,"op":"set|add|percent"?,"preset":"hit|edit|handheld"?,"placement":"clip|layer"?,"seconds":number?,"position":integer?}"#},enabled:has_seq,run,journal:true}).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demo() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s
    }
    #[test]
    fn jarvis_plan_atomic_rollback_and_single_undo() {
        let mut s = demo();
        let before = s.project.clone();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[1].id;
        let steps = json!([
            {"id":"voice.selectClip","params":{"target":{"ordinal":2,"track":1}}},
            {"id":"voice.parameter","params":{"effect":"motion","param":"scale","op":"percent","value":-20,"ensure":true}},
            {"id":"voice.splitHalf","params":{}}
        ]);
        let undo = s.history.undo.len();
        let items = s.active_sequence().unwrap().video_tracks[0].items.len();
        s.execute("voice.plan", json!({"steps":steps})).unwrap();
        assert_eq!(s.history.undo.len(), undo + 1);
        assert_eq!(s.active_sequence().unwrap().video_tracks[0].items.len(), items + 1);
        assert!(s.active_sequence().unwrap().find_item(clip).is_some());
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        let state = s.state.clone();
        let mut invalid = steps.as_array().unwrap().clone();
        invalid.push(json!({"id":"voice.parameter","params":{"effect":"unknown","param":"oops","value":1}}));
        let undo = s.history.undo.len();
        s.execute("voice.plan", json!({"steps":invalid})).unwrap_err();
        assert_eq!(s.project, before);
        assert_eq!(s.state.selection, state.selection);
        assert_eq!(s.history.undo.len(), undo);
        for params in [
            json!(null),
            json!({"steps":[]}),
            json!({"steps":[{"id":"file.save","params":{}}]}),
            json!({"steps":[{"id":"voice.plan","params":{}}]}),
            json!({"steps":[{"id":"voice.splitHalf","params":null}]}),
        ] {
            assert!(s.execute("voice.plan", params).is_err());
            assert_eq!(s.project, before);
        }
    }
    #[test]
    fn ripple_selected_tracks_preserves_other_layers_and_undo() {
        for gap in [false, true] {
            let mut s = demo();
            let q = s.active_sequence().unwrap();
            let c = q.video_tracks[0].items[1].clone();
            let next = q.video_tracks[0].items[2].clone();
            s.execute("timeline.select", json!({"clips":[c.id.0]})).unwrap();
            if gap {
                s.execute("edit.clear", json!({})).unwrap();
                s.execute("timeline.selectGap", json!({"track":"V1","time":c.start.0})).unwrap();
            }
            let before = s.project.clone();
            let q = s.active_sequence().unwrap().clone();
            let undo = s.history.undo.len();
            assert!(s.execute("edit.rippleDelete", json!({})).is_err());
            s.execute("voice.rippleDelete", json!({})).unwrap();
            let after = s.active_sequence().unwrap();
            assert_eq!(after.find_item(next.id).unwrap().1.start, c.start);
            assert_eq!(after.video_tracks[1], q.video_tracks[1]);
            assert_eq!(after.audio_tracks[1], q.audio_tracks[1]);
            assert_eq!(after.all_tracks().map(|t| t.sync_lock).collect::<Vec<_>>(), q.all_tracks().map(|t| t.sync_lock).collect::<Vec<_>>());
            assert_eq!(s.history.undo.len(), undo + 1);
            after.check().unwrap();
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.project, before);
            for p in [json!(null), json!({"clips":[c.id.0]}), json!({"scope":"all"})] {
                assert!(s.execute("voice.rippleDelete", p).is_err());
                assert_eq!(s.project, before);
            }
        }
    }
    #[test]
    fn select_parameter_shake_and_undo() {
        let mut s = demo();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        s.execute("playhead.set", json!({"time":clip.start.0})).unwrap();
        let r = s.execute("voice.selectClip", json!({})).unwrap();
        assert_eq!(r["clips"], json!([clip.id]));
        let before = s.project.clone();
        let undo = s.history.undo.len();
        s.execute("voice.parameter", json!({"effect":"motion","param":"scale","op":"percent","value":-20,"ensure":true})).unwrap();
        let c = &s.active_sequence().unwrap().find_item(clip.id).unwrap().1;
        assert_eq!(c.effects.iter().find(|e| e.effect == "motion").unwrap().f64_at("scale", clip.source_in), 80.);
        assert_eq!(s.history.undo.len(), undo + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        for preset in ["hit", "edit", "handheld"] {
            let before = s.project.clone();
            s.execute("voice.shake", json!({"preset":preset,"placement":"clip"})).unwrap();
            assert!(s.active_sequence().unwrap().find_item(clip.id).unwrap().1.effects.iter().any(|e| e.effect == "shaker"));
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.project, before);
        }
    }
    #[test]
    fn adjustment_and_hit_use_upper_tracks_and_one_undo() {
        let mut s = demo();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        s.execute("playhead.set", json!({"time":clip.start.0})).unwrap();
        s.execute("timeline.select", json!({"clips":[clip.id],"linked":false})).unwrap();
        for command in ["voice.adjustmentLayer", "voice.shake"] {
            let before = s.project.clone();
            let undo = s.history.undo.len();
            let r = s.execute(command, json!({})).unwrap();
            let id = ClipId(r["clips"][0].as_u64().unwrap());
            let q = s.active_sequence().unwrap();
            let (track, layer) = q.find_item(id).unwrap();
            assert!(matches!(s.project.item(layer.item).unwrap().kind, filmcraft_project::ItemKind::AdjustmentLayer { .. }));
            let index = q.video_tracks.iter().position(|t| t.id == track).unwrap();
            assert!(index > 0);
            assert!(q.video_tracks.iter().skip(index + 1).all(|t| t.items.iter().all(|c| !c.range().overlaps(&layer.range()))));
            assert_eq!(layer.start, clip.start);
            assert_eq!(q.find_item(clip.id).unwrap().1, &clip);
            if command == "voice.shake" {
                assert!(layer.duration <= Tick(TICKS_PER_SECOND / 3));
                assert!(layer.effects.iter().any(|e| e.effect == "shaker"));
            }
            assert_eq!(s.history.undo.len(), undo + 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.project, before);
        }
        let before = s.project.clone();
        for seconds in [json!(-1), json!(0), json!(1e100), json!("five")] {
            assert!(s.execute("voice.adjustmentLayer", json!({"seconds":seconds})).is_err());
            assert_eq!(s.project, before);
        }
    }

    #[test]
    fn linked_duration_split_reorder_and_percentage_points() {
        let mut s = demo();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[1].clone();
        s.execute("timeline.select", json!({"clips":[clip.id],"linked":true})).unwrap();
        let before = s.project.clone();
        let q = s.active_sequence().unwrap().clone();
        s.execute("voice.splitHalf", json!({})).unwrap();
        assert_eq!(s.active_sequence().unwrap().video_tracks[0].items.len(), q.video_tracks[0].items.len() + 1);
        assert_eq!(s.active_sequence().unwrap().audio_tracks[0].items.len(), q.audio_tracks[0].items.len() + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        s.execute("voice.duration", json!({"target":{"ordinal":2,"track":1},"seconds":2})).unwrap();
        assert_eq!(s.active_sequence().unwrap().find_item(clip.id).unwrap().1.duration, s.sequence_rate().snap_nearest(Tick(2 * TICKS_PER_SECOND)));
        let linked = s.active_sequence().unwrap().audio_tracks[0].items.iter().find(|c| c.link == clip.link).unwrap();
        assert_eq!(linked.duration, s.sequence_rate().snap_nearest(Tick(2 * TICKS_PER_SECOND)));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        s.execute("voice.reorder", json!({"target":{"ordinal":2,"track":1},"position":1})).unwrap();
        let mut video = s.active_sequence().unwrap().video_tracks[0].items.clone();
        video.sort_by_key(|c| c.start);
        assert_eq!(video[0].id, clip.id);
        assert_eq!(video[1].id, q.video_tracks[0].items[0].id);
        for c in &video {
            if let Some(link) = c.link {
                let a = s.active_sequence().unwrap().audio_tracks[0].items.iter().find(|a| a.link == Some(link)).unwrap();
                assert_eq!(a.start, c.start);
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        for p in [json!({"position":0}), json!({"position":u64::MAX}), json!({"seconds":-2})] {
            assert!(s.execute(if p.get("seconds").is_some() { "voice.duration" } else { "voice.reorder" }, p).is_err());
            assert_eq!(s.project, before);
        }
        s.execute("voice.parameter", json!({"effect":"motion","param":"scale","value":120,"ensure":true})).unwrap();
        s.execute("voice.parameter", json!({"effect":"motion","param":"scale","op":"add","value":-20,"ensure":true})).unwrap();
        let c = s.active_sequence().unwrap().find_item(clip.id).unwrap().1;
        assert_eq!(c.effects.iter().find(|e| e.effect == "motion").unwrap().f64_at("scale", c.source_in), 100.0);
    }

    #[test]
    fn invalid_targets_and_parameters_never_edit() {
        let mut s = demo();
        let before = s.project.clone();
        for p in [json!({"target":{"track":0}}), json!({"target":{"track":u64::MAX}}), json!({"target":{"ordinal":0}}), json!({"target":{"name":""}})] {
            assert!(s.execute("voice.selectClip", p).is_err());
            assert_eq!(s.project, before);
        }
        s.execute("voice.selectClip", json!({"target":{"ordinal":1,"track":1}})).unwrap();
        for p in [
            json!({"effect":"motion","param":"scale","value":null}),
            json!({"effect":"unknown","param":"x","value":2,"ensure":true}),
            json!({"effect":"motion","param":"scale","value":2,"op":"bad"}),
        ] {
            assert!(s.execute("voice.parameter", p).is_err());
            assert_eq!(s.project, before);
        }
    }
}
