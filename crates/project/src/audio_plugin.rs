//! Serializable identity and state for installed audio effects. No native code or plugin
//! binaries belong to a project. Missing plugins retain their identity and settings.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PLUGIN_PARAMETERS: usize = 4096;
pub const MAX_PLUGIN_STATE: usize = 8 * 1024 * 1024;
pub const MAX_AUDIO_BLOCK: usize = 4096;
pub const MAX_AUDIO_CHANNELS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Vst3,
    Vst2,
    AudioUnit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParameter {
    pub id: String,
    pub name: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub steps: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioPlugin {
    pub format: PluginFormat,
    pub path: String,
    pub native_id: String,
    pub name: String,
    pub vendor: String,
    pub category: String,
    pub parameters: Vec<PluginParameter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioPluginState {
    pub plugin: AudioPlugin,
    #[serde(default)]
    pub values: BTreeMap<String, f64>,
    #[serde(default)]
    pub state: Vec<u8>,
}

impl AudioPluginState {
    pub fn validate(&self) -> Result<(), String> {
        if self.plugin.parameters.len() > MAX_PLUGIN_PARAMETERS || self.values.len() > MAX_PLUGIN_PARAMETERS || self.state.len() > MAX_PLUGIN_STATE {
            return Err("Audio plugin state exceeds limits".into());
        }
        for s in [&self.plugin.path, &self.plugin.native_id, &self.plugin.name, &self.plugin.vendor, &self.plugin.category] {
            if s.len() > 32768 || s.contains('\0') {
                return Err("Invalid audio plugin metadata".into());
            }
        }
        if self.plugin.native_id.is_empty() {
            return Err("Missing audio plugin identifier".into());
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.plugin.parameters {
            if p.id == STATE_PARAM || p.id.is_empty() || !seen.insert(&p.id) {
                return Err("Duplicate or reserved plugin parameter ID".into());
            }
            if p.id.len() > 256
                || p.name.len() > 1024
                || p.unit.len() > 256
                || !p.min.is_finite()
                || !p.max.is_finite()
                || p.min > p.max
                || !p.default.is_finite()
                || !(p.min..=p.max).contains(&p.default)
            {
                return Err("Invalid audio plugin parameter".into());
            }
        }
        for (id, value) in &self.values {
            let p = self.plugin.parameters.iter().find(|p| &p.id == id).ok_or("Unknown audio plugin parameter")?;
            if !value.is_finite() || !(p.min..=p.max).contains(value) {
                return Err("Invalid audio plugin parameter value".into());
            }
        }
        Ok(())
    }
}

/// Private, versioned helper protocol. A fresh connection belongs to exactly one worker.
#[derive(Debug, Serialize, Deserialize)]
pub enum PluginRequest {
    Scan { path: String, format: PluginFormat },
    Open { state: AudioPluginState, sample_rate: u32, channels: usize },
    Process { audio: Vec<Vec<f32>>, values: BTreeMap<String, f64> },
    Reset,
    SaveState,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum PluginResponse {
    Catalog(Vec<AudioPlugin>),
    Opened { latency: usize, tail: usize },
    Audio(Vec<Vec<f32>>),
    State(Vec<u8>),
    Done,
    Error(String),
}

pub fn validate_audio(audio: &[Vec<f32>]) -> Result<usize, String> {
    if audio.is_empty() || audio.len() > MAX_AUDIO_CHANNELS {
        return Err("Unsupported plugin channel count".into());
    }
    let frames = audio.first().map_or(0, Vec::len);
    if frames > MAX_AUDIO_BLOCK || audio.iter().any(|a| a.len() != frames || a.iter().any(|s| !s.is_finite())) {
        return Err("Invalid plugin audio block".into());
    }
    Ok(frames)
}

/// The descriptor and initial opaque state travel inside every instance, so missing plugins
/// survive project round trips and can be relinked after moving a project to another computer.
pub const STATE_PARAM: &str = "__installed_audio_plugin";
pub fn instance_state(effect: &crate::EffectInstance) -> Result<AudioPluginState, String> {
    let Some(crate::ParamValue::Text(text)) = effect.param(STATE_PARAM).map(|p| &p.value) else {
        return Err("Missing installed plugin descriptor".into());
    };
    if text.len() > MAX_PLUGIN_STATE * 5 {
        return Err("Plugin descriptor too large".into());
    }
    let mut state: AudioPluginState = serde_json::from_str(text).map_err(|e| e.to_string())?;
    state.values = state.plugin.parameters.iter().filter_map(|p| effect.param(&p.id).and_then(|p| p.value.as_f64()).map(|v| (p.id.clone(), v))).collect();
    state.validate()?;
    if effect_id(&state.plugin) != effect.effect {
        return Err("Plugin identity does not match its effect".into());
    }
    Ok(state)
}
pub fn effect_id(plugin: &AudioPlugin) -> String {
    // Stable FNV-1a identifier; path is intentionally excluded for project portability.
    let mut h = 0xcbf29ce484222325u64;
    for b in format!("{:?}:{}", plugin.format, plugin.native_id).bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("installed_audio_{h:016x}")
}
fn registry() -> &'static std::sync::Mutex<BTreeMap<String, &'static crate::EffectDef>> {
    static R: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, &'static crate::EffectDef>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}
pub fn definitions() -> Vec<&'static crate::EffectDef> {
    registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner).values().copied().collect()
}
pub fn definition(id: &str) -> Option<&'static crate::EffectDef> {
    registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(id).copied()
}
pub fn register(state: &AudioPluginState) -> Result<&'static crate::EffectDef, String> {
    state.validate()?;
    let mut r = registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = effect_id(&state.plugin);
    if let Some(d) = r.get(&id) {
        return Ok(d);
    }
    // EffectDef has static references. Allocate each identity only once and bound the lifetime
    // registry, including total parameter bytes. Rescan never leaks another copy of an identity.
    if r.len() >= 512 || state.plugin.parameters.len() > 512 {
        return Err("Installed plugin catalog limit reached (512 effects/parameters)".into());
    }
    fn intern(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }
    let metadata = serde_json::to_string(state).map_err(|e| e.to_string())?;
    let used: usize = r
        .values()
        .map(|d| {
            d.params
                .iter()
                .map(|p| {
                    p.id.len()
                        + p.label.len()
                        + match &p.default {
                            crate::ParamValue::Text(s) => s.len(),
                            _ => 0,
                        }
                })
                .sum::<usize>()
        })
        .sum();
    if used.saturating_add(metadata.len()).saturating_add(state.plugin.parameters.len() * 2048) > 32 * 1024 * 1024 {
        return Err("Audio plugin catalog memory limit reached".into());
    }
    let mut params = state
        .plugin
        .parameters
        .iter()
        .map(|p| crate::ParamDef {
            id: intern(p.id.clone()),
            label: intern(p.name.clone()),
            kind: crate::ParamKind::Float { min: p.min, max: p.max, soft_min: p.min, soft_max: p.max, unit: intern(p.unit.clone()), decimals: 3 },
            default: crate::ParamValue::Float(p.default),
            animatable: true,
            group: None,
        })
        .collect::<Vec<_>>();
    params.push(crate::ParamDef {
        id: STATE_PARAM,
        label: "Plugin state",
        kind: crate::ParamKind::Text,
        default: crate::ParamValue::Text(metadata),
        animatable: false,
        group: None,
    });
    let folder = intern(format!("Installed Plugins / {} / {} ({:?})", state.plugin.vendor, state.plugin.category, state.plugin.format));
    let d = Box::leak(Box::new(crate::EffectDef {
        id: intern(id.clone()),
        name: intern(state.plugin.name.clone()),
        kind: crate::EffectKind::Audio,
        category: Box::leak(Box::new(["Audio Effects", folder])),
        params,
        intrinsic: false,
        accelerated: false,
        float32: true,
        yuv: false,
    }));
    r.insert(id, d);
    Ok(d)
}

#[cfg(test)]
mod hostile_tests {
    use super::*;
    fn state() -> AudioPluginState {
        AudioPluginState {
            plugin: AudioPlugin {
                format: PluginFormat::Vst3,
                path: "test.vst3".into(),
                native_id: "hostile-test".into(),
                name: "Test".into(),
                vendor: "Test".into(),
                category: "Other".into(),
                parameters: vec![PluginParameter { id: "1".into(), name: "Gain".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, steps: 0 }],
            },
            values: Default::default(),
            state: vec![],
        }
    }
    #[test]
    fn hostile_parameters_and_state_are_errors() {
        let mut s = state();
        s.values.insert("1".into(), f64::NAN);
        assert!(s.validate().is_err());
        s.values.clear();
        s.plugin.parameters[0].max = -1.0;
        assert!(s.validate().is_err());
        let mut s = state();
        s.state = vec![0; MAX_PLUGIN_STATE + 1];
        assert!(s.validate().is_err());
        let s = state();
        let a = register(&s).unwrap();
        let b = register(&s).unwrap();
        assert!(std::ptr::eq(a, b));
        let instance = a.instance();
        assert_eq!(instance_state(&instance).unwrap().plugin, s.plugin);
        let bytes = serde_json::to_vec(&instance).unwrap();
        for end in (0..bytes.len()).step_by(13) {
            assert!(std::panic::catch_unwind(|| serde_json::from_slice::<crate::EffectInstance>(&bytes[..end])).is_ok());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_audio_is_rejected() {
        for audio in [vec![], vec![vec![f32::NAN]], vec![vec![0.0]; 9], vec![vec![0.0], vec![]], vec![vec![0.0; MAX_AUDIO_BLOCK + 1]]] {
            assert!(validate_audio(&audio).is_err());
        }
        assert_eq!(validate_audio(&[vec![], vec![]]).unwrap(), 0);
    }
}
