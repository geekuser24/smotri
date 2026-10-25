//! Atomic application of normalized curves between selected consecutive keyframes.
use crate::{Result, Session, commands::bad};
use filmcraft_project::{
    ClipId, Interpolation, Param, Sequence,
    easing::{EasingCurve, EasingPreset},
};
use filmcraft_time::Tick;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
const ID: &str = "effects.applyEasing";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    clip: u64,
    effect: usize,
    param: String,
    #[serde(default)]
    mask: Option<usize>,
    time: Tick,
}
type Lane = (u64, usize, Option<usize>, String);
fn param_mut<'a>(q: &'a mut Sequence, l: &Lane) -> Result<&'a mut Param> {
    let track = q.find_item(ClipId(l.0)).map(|(t, _)| t).ok_or_else(|| bad(ID, "clip does not exist"))?;
    if q.track(track).is_some_and(|t| t.locked) {
        return Err(bad(ID, "track is locked"));
    }
    let (_, it) = q.find_item_mut(ClipId(l.0)).ok_or_else(|| bad(ID, "clip does not exist"))?;
    let e = it.effects.get_mut(l.1).ok_or_else(|| bad(ID, "effect does not exist"))?;
    match l.2 {
        Some(m) => e.masks.get_mut(m).and_then(|m| m.param_mut(&l.3)),
        None => e.params.get_mut(&l.3),
    }
    .ok_or_else(|| bad(ID, "parameter does not exist"))
}
fn curve(p: &Value) -> Result<EasingCurve> {
    let c: EasingCurve = serde_json::from_value(p.get("curve").cloned().unwrap_or(Value::Null)).map_err(|_| bad(ID, "need curve [x1,y1,x2,y2]"))?;
    if !c.valid() {
        return Err(bad(ID, "curve requires finite x in 0..1 and y in -2..3"));
    }
    Ok(c)
}
pub(crate) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let c = curve(p)?;
    let raw = p.get("keyframes").and_then(Value::as_array).ok_or_else(|| bad(ID, "select at least two consecutive keys"))?;
    if !(2..=4096).contains(&raw.len()) {
        return Err(bad(ID, "select 2..4096 keys"));
    }
    let keys: Vec<Key> = serde_json::from_value(Value::Array(raw.clone())).map_err(|_| bad(ID, "invalid key reference"))?;
    let mut lanes: BTreeMap<Lane, Vec<Tick>> = BTreeMap::new();
    for k in keys {
        if k.param.len() > 256 {
            return Err(bad(ID, "invalid parameter name"));
        }
        lanes.entry((k.clip, k.effect, k.mask, k.param)).or_default().push(k.time);
    }
    // Validate a private copy first: malformed selections cannot partially edit the session.
    let mut q = s.active_sequence().ok_or(crate::EngineError::NoSequence)?.clone();
    let mut segments = 0;
    for (l, times) in &mut lanes {
        times.sort();
        if times.len() < 2 || times.windows(2).any(|w| w[0] == w[1]) {
            return Err(bad(ID, "select distinct consecutive keys in each parameter"));
        }
        let p = param_mut(&mut q, l)?;
        if !p.value.interpolates() {
            return Err(bad(ID, "this parameter cannot interpolate"));
        }
        let mut indices = Vec::new();
        for t in times.iter() {
            indices.push(p.keyframes.iter().position(|k| k.time == *t).ok_or_else(|| bad(ID, "key no longer exists"))?);
        }
        if indices.windows(2).any(|w| w[1] != w[0] + 1) {
            return Err(bad(ID, "select consecutive keys; an unselected key lies between them"));
        }
        for t in times.iter().take(times.len() - 1) {
            if let Some(k) = p.keyframes.iter_mut().find(|k| k.time == *t) {
                k.easing = Some(c);
                k.interp = Interpolation::Bezier;
                segments += 1;
            }
        }
    }
    s.edit_sequence("Apply Easing", move |sequence, _, _| {
        *sequence = q;
        Ok(())
    })?;
    Ok(json!({"segments":segments}))
}
pub(crate) fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"builtIn":filmcraft_project::easing::presets(),"custom":s.prefs.easing_presets}))
}
pub(crate) fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let c = curve(p)?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err(bad(ID, "preset name must be 1..120 bytes"));
    }
    if filmcraft_project::easing::presets().iter().any(|v| v.name.eq_ignore_ascii_case(name)) {
        return Err(bad(ID, "choose a custom name"));
    }
    let mut prefs = s.prefs.clone();
    if let Some(v) = prefs.easing_presets.iter_mut().find(|v| v.name == name) {
        v.curve = c;
    } else {
        if prefs.easing_presets.len() >= 128 {
            return Err(bad(ID, "maximum 128 custom presets"));
        }
        prefs.easing_presets.push(EasingPreset { name: name.into(), curve: c });
    }
    s.set_prefs(prefs).map_err(|e| bad(ID, format!("could not save presets: {e}")))?;
    list(s, &Value::Null)
}
pub(crate) fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(ID, "need name"))?;
    let mut prefs = s.prefs.clone();
    if !prefs.easing_presets.iter().any(|v| v.name == name) {
        return Err(bad(ID, "custom preset not found"));
    }
    prefs.easing_presets.retain(|v| v.name != name);
    s.set_prefs(prefs).map_err(|e| bad(ID, format!("could not save presets: {e}")))?;
    list(s, &Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Session, Value) {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let id = s.active_sequence().unwrap().video_tracks[0].items[0].id;
        for (seconds, value) in [(0.5, 10.0), (1.0, 80.0), (1.5, 100.0)] {
            s.execute("playhead.set", json!({"seconds":seconds})).unwrap();
            s.execute("effects.addKeyframe", json!({"clip":id.0,"effect":"motion","param":"scale"})).unwrap();
            s.execute("effects.setParam", json!({"clip":id.0,"effect":"motion","param":"scale","value":value})).unwrap();
        }
        let it = s.active_sequence().unwrap().find_item(id).unwrap().1;
        let effect = it.effects.iter().position(|e| e.effect == "motion").unwrap();
        let keys =
            it.effects[effect].params["scale"].keyframes.iter().map(|k| json!({"clip":id.0,"effect":effect,"param":"scale","time":k.time})).collect::<Vec<_>>();
        (s, json!(keys))
    }
    #[test]
    fn application_preserves_keys_and_is_one_undo() {
        let (mut s, keys) = fixture();
        let before = serde_json::to_value(&s.project).unwrap();
        let undo = s.history.undo.len();
        let result = s.execute(ID, json!({"keyframes":keys,"curve":[0.6,0.0,0.4,1.0]})).unwrap();
        assert_eq!(result["segments"], 2);
        assert_eq!(s.history.undo.len(), undo + 1);
        let q = s.active_sequence().unwrap();
        let it = q.find_item(ClipId(keys[0]["clip"].as_u64().unwrap())).unwrap().1;
        let p = &it.effects[keys[0]["effect"].as_u64().unwrap() as usize].params["scale"];
        assert_eq!(p.keyframes.len(), 3);
        assert_eq!(p.keyframes[0].value, filmcraft_project::ParamValue::Float(10.0));
        assert!(p.keyframes[0].easing.is_some());
        assert!(p.keyframes[2].easing.is_none());
        s.undo();
        assert_eq!(serde_json::to_value(&s.project).unwrap(), before);
        s.redo();
        s.execute(
            "effects.setInterpolation",
            json!({"clip":keys[0]["clip"],"effect":keys[0]["effect"],"param":"scale","mediaTime":keys[0]["time"],"interpolation":"linear"}),
        )
        .unwrap();
        let it = s.active_sequence().unwrap().find_item(ClipId(keys[0]["clip"].as_u64().unwrap())).unwrap().1;
        assert!(it.effects[keys[0]["effect"].as_u64().unwrap() as usize].params["scale"].keyframes[0].easing.is_none());
    }
    #[test]
    fn hostile_requests_leave_project_and_history_unchanged() {
        let (mut s, keys) = fixture();
        let before = serde_json::to_value(&s.project).unwrap();
        let undo = s.history.undo.len();
        for p in [
            json!({}),
            json!({"keyframes":keys,"curve":[-1,0,1,1]}),
            json!({"keyframes":[keys[0],keys[0]],"curve":[0,0,1,1]}),
            json!({"keyframes":[keys[0],keys[2]],"curve":[0,0,1,1]}),
            json!({"keyframes":[keys[0],{"clip":u64::MAX,"effect":0,"param":"scale","time":0}],"curve":[0,0,1,1]}),
        ] {
            assert!(s.execute(ID, p).is_err());
            assert_eq!(serde_json::to_value(&s.project).unwrap(), before);
            assert_eq!(s.history.undo.len(), undo);
        }
    }
    #[test]
    fn named_presets_roundtrip_without_changing_project() {
        let mut s = Session::default();
        let before = serde_json::to_value(&s.project).unwrap();
        s.execute("easing.save", json!({"name":"My smooth","curve":[0.2,-0.2,0.7,1.3]})).unwrap();
        let prefs: crate::autosave::Preferences = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
        assert_eq!(prefs.easing_presets.len(), 1);
        assert_eq!(prefs.easing_presets[0].name, "My smooth");
        s.execute("easing.save", json!({"name":"My smooth","curve":[0.1,0.0,0.8,1.0]})).unwrap();
        assert_eq!(s.prefs.easing_presets.len(), 1);
        assert!(s.execute("easing.save", json!({"name":"Linear","curve":[0,0,1,1]})).is_err());
        assert!(s.execute("easing.save", json!({"name":"bad","curve":[0,99,1,1]})).is_err());
        s.execute("easing.delete", json!({"name":"My smooth"})).unwrap();
        assert!(s.prefs.easing_presets.is_empty());
        assert_eq!(serde_json::to_value(&s.project).unwrap(), before);
    }
}
