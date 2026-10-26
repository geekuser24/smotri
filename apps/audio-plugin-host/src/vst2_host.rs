//! Legacy VST 2.4 support through the MIT vst-rs ABI adapter, in this process only.
use filmcraft_project::audio_plugin::*;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
use vst::{
    host::{Host, HostBuffer, PluginInstance, PluginLoader},
    plugin::{Plugin, PluginParameters},
};
struct Context;
impl Host for Context {
    fn get_info(&self) -> (isize, String, String) {
        (1, "Smotri".into(), "Smotri".into())
    }
    fn get_block_size(&self) -> isize {
        MAX_AUDIO_BLOCK as isize
    }
}
pub struct Processor {
    parameters: Arc<dyn PluginParameters>,
    instance: PluginInstance,
    channels: usize,
    count: usize,
    chunks: bool,
    pub latency: usize,
    pub tail: usize,
}
impl Drop for Processor {
    fn drop(&mut self) {
        self.instance.suspend();
    }
}
impl Processor {
    fn load(path: &str, sr: u32, channels: usize) -> Result<Self, String> {
        if !(1..=2).contains(&channels) {
            return Err("VST requires mono or stereo audio".into());
        }
        let path = crate::vst3_host::native_path(Path::new(path))?;
        let mut loader = PluginLoader::load(&path, Arc::new(Mutex::new(Context))).map_err(|e| format!("Cannot load VST (check architecture): {e}"))?;
        let mut instance = loader.instance().map_err(|e| e.to_string())?;
        instance.init();
        let info = instance.get_info();
        if info.inputs != channels as i32
            || info.outputs != channels as i32
            || info.parameters < 0
            || info.parameters as usize > MAX_PLUGIN_PARAMETERS
            || info.initial_delay < 0
            || info.initial_delay as u32 > sr.saturating_mul(30)
        {
            return Err("Unsupported VST channel layout/parameter count/latency".into());
        }
        instance.set_sample_rate(sr as f32);
        instance.set_block_size(MAX_AUDIO_BLOCK as i64);
        let parameters = instance.get_parameter_object();
        let p =
            Self { parameters, instance, channels, count: info.parameters as usize, chunks: info.preset_chunks, latency: info.initial_delay as usize, tail: 0 };
        Ok(p)
    }
    pub fn open(state: &AudioPluginState, sr: u32, channels: usize) -> Result<Self, String> {
        state.validate()?;
        let mut p = Self::load(&state.plugin.path, sr, channels)?;
        if p.instance.get_info().unique_id.to_string() != state.plugin.native_id {
            return Err("VST identity changed".into());
        }
        if !state.state.is_empty() {
            let (chunk, values): (Vec<u8>, Vec<f32>) = serde_json::from_slice(&state.state).map_err(|e| e.to_string())?;
            if chunk.len() > MAX_PLUGIN_STATE || values.len() > MAX_PLUGIN_PARAMETERS || values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err("Invalid VST saved state".into());
            }
            if p.chunks {
                p.parameters.load_preset_data(&chunk);
            }
            for (i, v) in values.into_iter().enumerate().take(p.count) {
                p.parameters.set_parameter(i as i32, v);
            }
        }
        for (id, v) in &state.values {
            p.set(id, *v)?;
        }
        p.instance.resume();
        Ok(p)
    }
    fn set(&self, id: &str, value: f64) -> Result<(), String> {
        let index = id.parse::<usize>().map_err(|e| e.to_string())?;
        if index >= self.count || !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Invalid VST parameter".into());
        }
        self.parameters.set_parameter(index as i32, value as f32);
        Ok(())
    }
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let chunk = if self.chunks { self.parameters.get_preset_data() } else { vec![] };
        if chunk.len() > MAX_PLUGIN_STATE {
            return Err("VST state too large".into());
        }
        let values = (0..self.count).map(|i| self.parameters.get_parameter(i as i32)).collect::<Vec<_>>();
        if values.iter().any(|v| !v.is_finite()) {
            return Err("Invalid VST state values".into());
        }
        serde_json::to_vec(&(chunk, values)).map_err(|e| e.to_string())
    }
    pub fn process(&mut self, audio: Vec<Vec<f32>>, values: &BTreeMap<String, f64>) -> Result<Vec<Vec<f32>>, String> {
        let frames = validate_audio(&audio)?;
        if audio.len() != self.channels {
            return Err("VST channel count changed".into());
        }
        if frames == 0 {
            return Ok(audio);
        }
        for (id, value) in values {
            self.set(id, *value)?;
        }
        let mut output = vec![vec![0.0; frames]; self.channels];
        let mut host = HostBuffer::new(self.channels, self.channels);
        let mut buffer = host.bind(&audio, &mut output);
        self.instance.process(&mut buffer);
        validate_audio(&output)?;
        Ok(output)
    }
}
pub fn scan(path: &str) -> Result<Vec<AudioPlugin>, String> {
    let p = Processor::load(path, 48000, 2).or_else(|_| Processor::load(path, 48000, 1))?;
    let info = p.instance.get_info();
    let mut parameters = vec![];
    for i in 0..p.count {
        let value = p.parameters.get_parameter(i as i32);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Invalid VST parameter default".into());
        }
        parameters.push(PluginParameter {
            id: i.to_string(),
            name: p.parameters.get_parameter_name(i as i32),
            unit: String::new(), // VST 2.4 exposes normalized values; display text is vendor-specific.
            min: 0.0,
            max: 1.0,
            default: value as f64,
            steps: 0,
        });
    }
    Ok(vec![AudioPlugin {
        format: PluginFormat::Vst2,
        path: path.into(),
        native_id: info.unique_id.to_string(),
        name: info.name,
        vendor: info.vendor,
        category: format!("{:?}", info.category),
        parameters,
    }])
}
