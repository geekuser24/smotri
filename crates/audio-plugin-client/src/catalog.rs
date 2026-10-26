//! Explicit, bounded scans. Each module has its own disposable worker and timeout.
use filmcraft_project::audio_plugin::*;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub running: bool,
    pub current: String,
    pub plugins: Vec<AudioPluginState>,
    pub failures: Vec<String>,
}
fn shared() -> &'static Mutex<Catalog> {
    static C: OnceLock<Mutex<Catalog>> = OnceLock::new();
    C.get_or_init(Default::default)
}
pub fn snapshot() -> Catalog {
    shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}
pub fn status() -> Catalog {
    let c = shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Catalog {
        running: c.running,
        current: c.current.clone(),
        failures: c.failures.clone(),
        plugins: c.plugins.iter().map(|s| AudioPluginState { plugin: s.plugin.clone(), values: Default::default(), state: vec![] }).collect(),
    }
}
pub fn local(id: &str) -> Option<AudioPlugin> {
    shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).plugins.iter().find(|p| effect_id(&p.plugin) == id).map(|s| s.plugin.clone())
}
pub fn load(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.metadata().map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
        return Err("Plugin catalog too large".into());
    }
    let states: Vec<AudioPluginState> = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if states.len() > 512 {
        return Err("Too many installed plugins".into());
    }
    for state in &states {
        state.validate()?;
        register(state)?;
    }
    let mut c = shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !c.running {
        c.plugins = states;
    }
    Ok(())
}
pub fn defaults() -> Vec<PathBuf> {
    let mut p = vec![];
    if cfg!(target_os = "macos") {
        p.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Some(h) = std::env::var_os("HOME") {
            p.push(PathBuf::from(h).join("Library/Audio/Plug-Ins/VST3"));
        }
    } else if cfg!(windows) {
        for base in ["CommonProgramFiles", "CommonProgramFiles(x86)"] {
            if let Some(h) = std::env::var_os(base) {
                p.push(PathBuf::from(h).join("VST3"));
            }
        }
    } else {
        p.extend(["/usr/lib/vst3", "/usr/local/lib/vst3"].map(PathBuf::from));
        if let Some(h) = std::env::var_os("HOME") {
            p.push(PathBuf::from(h).join(".vst3"));
        }
    }
    if cfg!(target_os = "macos") {
        p.push(PathBuf::from("/Library/Audio/Plug-Ins/VST"));
        if let Some(h) = std::env::var_os("HOME") {
            p.push(PathBuf::from(h).join("Library/Audio/Plug-Ins/VST"));
        }
    }
    if cfg!(windows)
        && let Some(h) = std::env::var_os("ProgramFiles")
    {
        let h = PathBuf::from(h);
        p.push(h.join("VstPlugins"));
        p.push(h.join("Steinberg/VstPlugins"));
    }
    if cfg!(target_os = "linux") {
        p.push(PathBuf::from("/usr/lib/vst"));
        p.push(PathBuf::from("/usr/local/lib/vst"));
        if let Some(h) = std::env::var_os("HOME") {
            p.push(PathBuf::from(h).join(".vst"));
        }
    }
    p
}
pub fn start(extra: Vec<PathBuf>, destination: Option<PathBuf>) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (extra, destination);
        Err("Installed audio plugins require the desktop editor".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if extra.len() > 32 || extra.iter().any(|p| p.as_os_str().len() > 32768) {
            return Err("Too many plugin folders".into());
        }
        {
            let mut c = shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if c.running {
                return Err("Plugin scan is already running".into());
            }
            c.running = true;
            c.failures.clear();
        }
        let spawn = std::thread::Builder::new().name("audio-plugin-scan".into()).spawn(move || {
            let result = std::panic::catch_unwind(|| scan(extra, destination)).unwrap_or_else(|_| Err("Plugin scan failed".into()));
            let mut c = shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(e) = result {
                c.failures.push(e);
            }
            c.running = false;
            c.current.clear();
        });
        if let Err(e) = spawn {
            shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).running = false;
            return Err(e.to_string());
        }
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn scan(extra: Vec<PathBuf>, destination: Option<PathBuf>) -> Result<(), String> {
    use super::Worker;
    let mut paths = defaults();
    paths.extend(extra);
    let mut modules = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut visited = 0;
    while let Some(path) = paths.pop() {
        visited += 1;
        if visited > 20_000 || modules.len() > 512 {
            return Err("Plugin folder scan limit reached".into());
        }
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vst3")) {
            modules.push((path.to_string_lossy().into_owned(), PluginFormat::Vst3));
            continue;
        }
        if path.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case("vst") || cfg!(windows) && e.eq_ignore_ascii_case("dll") || cfg!(target_os = "linux") && e.eq_ignore_ascii_case("so")
        }) {
            modules.push((path.to_string_lossy().into_owned(), PluginFormat::Vst2));
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.take(4096).flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir() || t.is_file()) {
                    paths.push(entry.path());
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut worker = Worker::start()?;
        if let PluginResponse::Catalog(units) = worker.request(&PluginRequest::Scan { path: String::new(), format: PluginFormat::AudioUnit })? {
            for p in units {
                modules.push((p.native_id, PluginFormat::AudioUnit));
            }
        }
    }
    modules.sort_by(|a, b| a.0.cmp(&b.0));
    if modules.len() > 512 {
        shared()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .failures
            .push("Scan limited to 512 modules; narrow the plugin folders if needed".into());
    }
    let mut found = vec![];
    for (path, format) in modules.into_iter().take(512) {
        shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).current = path.clone();
        let result = (|| -> Result<Vec<AudioPluginState>, String> {
            let mut worker = Worker::start()?;
            let PluginResponse::Catalog(plugins) = worker.request(&PluginRequest::Scan { path: path.clone(), format })? else {
                return Err("Invalid scan response".into());
            };
            let mut states = vec![];
            for plugin in plugins {
                let mut state = AudioPluginState { values: plugin.parameters.iter().map(|p| (p.id.clone(), p.default)).collect(), plugin, state: vec![] };
                worker
                    .request(&PluginRequest::Open { state: state.clone(), sample_rate: 48000, channels: 2 })
                    .or_else(|_| worker.request(&PluginRequest::Open { state: state.clone(), sample_rate: 48000, channels: 1 }))?;
                if let PluginResponse::State(bytes) = worker.request(&PluginRequest::SaveState)? {
                    state.state = bytes;
                } else {
                    return Err("Plugin did not provide its state".into());
                }
                state.validate()?;
                worker.request(&PluginRequest::Reset)?;
                worker
                    .request(&PluginRequest::Open { state: state.clone(), sample_rate: 48000, channels: 2 })
                    .or_else(|_| worker.request(&PluginRequest::Open { state: state.clone(), sample_rate: 48000, channels: 1 }))?;
                register(&state)?;
                states.push(state);
            }
            Ok(states)
        })();
        match result {
            Ok(states) => found.extend(states),
            Err(e) => shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).failures.push(format!("{path}: {e}")),
        }
    }
    found.sort_by(|a, b| a.plugin.vendor.cmp(&b.plugin.vendor).then(a.plugin.name.cmp(&b.plugin.name)));
    found.dedup_by(|a, b| effect_id(&a.plugin) == effect_id(&b.plugin));
    if let Some(path) = destination {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&found).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
    }
    shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).plugins = found;
    Ok(())
}
