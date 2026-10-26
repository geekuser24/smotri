//! VST3 ABI calls live only in the disposable worker process.
#![allow(unsafe_code, non_snake_case)]
use filmcraft_project::audio_plugin::*;
use libloading::Library;
use std::{
    collections::BTreeMap,
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
};
use vst3::{Class, ComPtr, ComWrapper, Interface, Steinberg::Vst::*, Steinberg::*};

fn status(result: tresult, operation: &str) -> Result<(), String> {
    if result == kResultOk { Ok(()) } else { Err(format!("VST3 {operation}: {result}")) }
}
fn text(bytes: &[i8]) -> String {
    String::from_utf8_lossy(&bytes.iter().take_while(|b| **b != 0).map(|b| *b as u8).collect::<Vec<_>>()).into_owned()
}
fn wide(bytes: &[u16]) -> String {
    String::from_utf16_lossy(&bytes.iter().copied().take_while(|b| *b != 0).collect::<Vec<_>>())
}
pub(crate) fn native_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Ok(path.to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        let info = plist::Value::from_file(path.join("Contents/Info.plist")).map_err(|e| e.to_string())?;
        let executable =
            info.as_dictionary().and_then(|p| p.get("CFBundleExecutable")).and_then(plist::Value::as_string).ok_or("VST3 bundle has no executable")?;
        if executable.contains('/') || executable.contains('\\') || executable == ".." {
            return Err("Invalid VST3 executable name".into());
        }
        Ok(path.join("Contents/MacOS").join(executable))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let arch = match (std::env::consts::ARCH, std::env::consts::OS) {
            ("x86_64", "windows") => "x86_64-win",
            ("aarch64", "windows") => "arm64-win",
            ("x86_64", "linux") => "x86_64-linux",
            ("aarch64", "linux") => "aarch64-linux",
            _ => return Err("Unsupported VST3 architecture".into()),
        };
        let directory = path.join("Contents").join(arch);
        let mut candidates =
            std::fs::read_dir(directory).map_err(|e| e.to_string())?.take(128).filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_file());
        let module = candidates.find(|p| p.extension().is_some_and(|e| e == "vst3" || e == "so")).ok_or("No VST3 module for this architecture")?;
        Ok(module)
    }
}

struct Code {
    library: Library,
    #[cfg(target_os = "macos")]
    _bundle: objc2_core_foundation::CFRetained<objc2_core_foundation::CFBundle>,
}
impl Drop for Code {
    fn drop(&mut self) {
        // SAFETY: all factory/component references drop before Code; exit pairs with successful entry.
        unsafe {
            #[cfg(target_os = "macos")]
            if let Ok(exit) = self.library.get::<unsafe extern "C" fn() -> bool>(b"bundleExit\0") {
                let _ = exit();
            }
            #[cfg(target_os = "windows")]
            if let Ok(exit) = self.library.get::<unsafe extern "C" fn() -> bool>(b"ExitDll\0") {
                let _ = exit();
            }
            #[cfg(target_os = "linux")]
            if let Ok(exit) = self.library.get::<unsafe extern "C" fn() -> bool>(b"ModuleExit\0") {
                let _ = exit();
            }
        }
    }
}
struct Module {
    factory: ComPtr<IPluginFactory>,
    _code: Code,
}
impl Module {
    fn load(path: &Path) -> Result<Self, String> {
        let native = native_path(path)?;
        // SAFETY: only the isolated worker loads user-selected native libraries. The library
        // outlives all COM objects; ABI symbols are checked before use.
        let library = unsafe { Library::new(&native) }.map_err(|e| format!("Cannot load VST3 (check architecture): {e}"))?;
        #[cfg(target_os = "macos")]
        let bundle = {
            use objc2_core_foundation::{CFBundle, CFURL};
            use std::os::unix::ffi::OsStrExt;
            let bytes = path.as_os_str().as_bytes();
            // SAFETY: bytes points to this live path buffer for exactly bytes.len() bytes.
            let url = unsafe { CFURL::from_file_system_representation(None, bytes.as_ptr(), bytes.len() as isize, true) }.ok_or("Invalid VST3 bundle URL")?;
            let bundle = CFBundle::new(None, Some(&url)).ok_or("Invalid VST3 bundle")?;
            // SAFETY: bundleEntry has the VST3 macOS ABI; retained bundle is valid until unload.
            unsafe {
                let entry = library.get::<unsafe extern "C" fn(*const c_void) -> bool>(b"bundleEntry\0").map_err(|e| e.to_string())?;
                if !entry((&*bundle as *const CFBundle).cast()) {
                    return Err("VST3 bundle initialization failed".into());
                }
            }
            bundle
        };
        #[cfg(target_os = "windows")]
        // SAFETY: standard optional VST3 Windows module initialization ABI.
        unsafe {
            if let Ok(entry) = library.get::<unsafe extern "C" fn() -> bool>(b"InitDll\0")
                && !entry()
            {
                return Err("VST3 initialization failed".into());
            }
        }
        #[cfg(target_os = "linux")]
        // SAFETY: standard VST3 Linux module entry accepts the dlopen handle.
        unsafe {
            let handle: libloading::os::unix::Library = library.into();
            let raw = handle.into_raw();
            let handle = libloading::os::unix::Library::from_raw(raw);
            let library2: Library = handle.into();
            if let Ok(entry) = library2.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0")
                && !entry(raw)
            {
                return Err("VST3 initialization failed".into());
            }
            Self::from_library(library2)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let code = Code {
                library,
                #[cfg(target_os = "macos")]
                _bundle: bundle,
            };
            // SAFETY: factory entry point returns an owned IPluginFactory reference by VST3 ABI.
            let factory = unsafe {
                let get = code.library.get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0").map_err(|e| e.to_string())?;
                ComPtr::from_raw(get()).ok_or("Null VST3 factory")?
            };
            Ok(Self { factory, _code: code })
        }
    }
    #[cfg(target_os = "linux")]
    fn from_library(library: Library) -> Result<Self, String> {
        let code = Code { library };
        // SAFETY: checked symbol with the standard owned factory result ABI.
        let factory = unsafe {
            let get = code.library.get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0").map_err(|e| e.to_string())?;
            ComPtr::from_raw(get()).ok_or("Null VST3 factory")?
        };
        Ok(Self { factory, _code: code })
    }
    fn create<I: Interface>(&self, id: &TUID) -> Result<ComPtr<I>, String> {
        let mut raw = ptr::null_mut();
        // SAFETY: factory remains alive, IDs are fixed-size ABI arrays and raw is writable.
        unsafe {
            status(self.factory.createInstance(id.as_ptr(), I::IID.as_ptr().cast(), &mut raw), "create instance")?;
            ComPtr::from_raw(raw.cast()).ok_or_else(|| "Null VST3 instance".into())
        }
    }
}

#[derive(Default)]
struct MemoryStream {
    inner: std::sync::Mutex<(Vec<u8>, usize)>,
}
impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}
impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, count: i32, actual: *mut i32) -> tresult {
        if count < 0 || (buffer.is_null() && count > 0) {
            return kInvalidArgument;
        }
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = (count as usize).min(g.0.len().saturating_sub(g.1));
        if n > 0
            && let Some(bytes) = g.0.get(g.1..g.1 + n)
        {
            // SAFETY: the ABI caller supplies count writable bytes; n is bounded by count.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast(), n) };
        }
        g.1 += n;
        // SAFETY: optional ABI output pointer is checked before writing.
        if !actual.is_null() {
            // SAFETY: actual is a non-null optional ABI output.
            unsafe { actual.write(n as i32) };
        }
        // A short read is successful; numBytesRead reports the actual size.
        // Some plugins read state in fixed-size chunks until EOF.
        if n > 0 || count == 0 { kResultOk } else { kResultFalse }
    }
    unsafe fn write(&self, buffer: *mut c_void, count: i32, actual: *mut i32) -> tresult {
        if count < 0 || (buffer.is_null() && count > 0) {
            return kInvalidArgument;
        }
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(end) = g.1.checked_add(count as usize).filter(|n| *n <= MAX_PLUGIN_STATE) else {
            return kOutOfMemory;
        };
        let new_len = end.max(g.0.len());
        g.0.resize(new_len, 0);
        let start = g.1;
        if count > 0
            && let Some(dest) = g.0.get_mut(start..end)
        {
            // SAFETY: caller supplies count readable bytes; destination is a bounded live slice.
            unsafe { ptr::copy_nonoverlapping(buffer.cast(), dest.as_mut_ptr(), count as usize) };
        }
        g.1 = end;
        // SAFETY: optional ABI output pointer is checked before writing.
        if !actual.is_null() {
            // SAFETY: actual is a non-null optional ABI output.
            unsafe { actual.write(count) };
        }
        kResultOk
    }
    unsafe fn seek(&self, pos: i64, mode: i32, result: *mut i64) -> tresult {
        let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = match mode {
            0 => 0,
            1 => g.1 as i64,
            2 => g.0.len() as i64,
            _ => return kInvalidArgument,
        };
        let Some(next) = base.checked_add(pos).filter(|x| *x >= 0 && *x <= MAX_PLUGIN_STATE as i64) else {
            return kInvalidArgument;
        };
        g.1 = next as usize;
        // SAFETY: optional ABI output pointer is checked before writing.
        if !result.is_null() {
            // SAFETY: result is a non-null optional ABI output.
            unsafe { result.write(next) };
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        let g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: caller provides writable position pointer; null rejected.
        unsafe { pos.write(g.1 as i64) };
        kResultOk
    }
}
fn memory(bytes: Vec<u8>) -> Result<ComPtr<IBStream>, String> {
    ComWrapper::new(MemoryStream { inner: std::sync::Mutex::new((bytes, 0)) }).to_com_ptr::<IBStream>().ok_or("Cannot create state stream".into())
}
fn stream_bytes(s: &ComPtr<IBStream>) -> Result<Vec<u8>, String> {
    let mut len = 0;
    // SAFETY: owned stream with valid output pointers and bounded length checked below.
    unsafe {
        status(s.seek(0, 2, &mut len), "state size")?;
        status(s.seek(0, 0, ptr::null_mut()), "rewind")?;
    }
    if !(0..=MAX_PLUGIN_STATE as i64).contains(&len) {
        return Err("VST3 state too large".into());
    }
    let mut bytes = vec![0; len as usize];
    // SAFETY: bytes has exactly len writable bytes and len fits i32.
    unsafe {
        status(s.read(bytes.as_mut_ptr().cast(), len as i32, ptr::null_mut()), "read state")?;
    }
    Ok(bytes)
}
struct Host;
impl Class for Host {
    type Interfaces = (IHostApplication, IComponentHandler);
}
impl IHostApplicationTrait for Host {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        let mut value = [0; 128];
        for (d, s) in value.iter_mut().zip("Smotri".encode_utf16()) {
            *d = s;
        }
        // SAFETY: ABI caller provides a writable String128; null was rejected.
        unsafe { name.write(value) };
        kResultOk
    }
    unsafe fn createInstance(&self, _cid: *mut TUID, _iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: writable pointer from the ABI; no host object interfaces are exposed here.
        unsafe { obj.write(ptr::null_mut()) };
        kNoInterface
    }
}
impl IComponentHandlerTrait for Host {
    unsafe fn beginEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, _id: u32, value: f64) -> tresult {
        if value.is_finite() { kResultOk } else { kInvalidArgument }
    }
    unsafe fn endEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        kNotImplemented
    }
}
struct Point {
    id: u32,
    value: f64,
}
impl Class for Point {
    type Interfaces = (IParamValueQueue,);
}
impl IParamValueQueueTrait for Point {
    unsafe fn getParameterId(&self) -> u32 {
        self.id
    }
    unsafe fn getPointCount(&self) -> i32 {
        1
    }
    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut f64) -> tresult {
        if index != 0 || offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: output pointers supplied by the processor ABI, validated non-null.
        unsafe {
            offset.write(0);
            value.write(self.value);
        }
        kResultOk
    }
    unsafe fn addPoint(&self, _offset: i32, _value: f64, _index: *mut i32) -> tresult {
        kNotImplemented
    }
}
struct Changes {
    points: Vec<ComPtr<IParamValueQueue>>,
}
impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}
impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> i32 {
        self.points.len() as i32
    }
    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if index < 0 {
            return ptr::null_mut();
        }
        self.points.get(index as usize).map_or(ptr::null_mut(), ComPtr::as_ptr)
    }
    unsafe fn addParameterData(&self, _id: *const u32, _index: *mut i32) -> *mut IParamValueQueue {
        ptr::null_mut()
    }
}

pub struct Processor {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    controller_separate: bool,
    _host: ComPtr<IHostApplication>,
    _module: std::sync::Arc<Module>,
    channels: usize,
    input_buses: usize,
    output_buses: usize,
    params: Vec<PluginParameter>,
    previous: BTreeMap<String, f64>,
    pub latency: usize,
    pub tail: usize,
    sample_rate: u32,
    position: i64,
}
impl Processor {
    fn new(module: std::sync::Arc<Module>, id: TUID, sr: u32, channels: usize) -> Result<Self, String> {
        let host = ComWrapper::new(Host).to_com_ptr::<IHostApplication>().ok_or("Cannot create VST3 host context")?;
        let component = module.create::<IComponent>(&id)?;
        // SAFETY: all interfaces own references and the host context outlives the plugin.
        unsafe {
            status(component.initialize(host.as_ptr().cast()), "initialize")?;
        }
        let processor = component.cast::<IAudioProcessor>().ok_or("VST3 is not an audio processor")?;
        let mut controller = component.cast::<IEditController>();
        let mut separate = false;
        if controller.is_none() {
            let mut cid = [0; 16];
            // SAFETY: component is initialized; cid is a writable 16-byte class identifier.
            if unsafe { component.getControllerClassId(&mut cid) } == kResultOk {
                let c = module.create::<IEditController>(&cid)?;
                // SAFETY: live host context and owned controller reference.
                unsafe {
                    status(c.initialize(host.as_ptr().cast()), "initialize controller")?;
                }
                controller = Some(c);
                separate = true;
            }
        }
        if let Some(c) = &controller {
            let handler = host.cast::<IComponentHandler>().ok_or("Missing host component handler")?;
            // SAFETY: interfaces and host remain owned until after component termination.
            unsafe {
                status(c.setComponentHandler(handler.as_ptr()), "component handler")?;
                if let (Some(a), Some(b)) = (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                    status(a.connect(b.as_ptr()), "connect processor")?;
                    status(b.connect(a.as_ptr()), "connect controller")?;
                }
                let stream = memory(vec![])?;
                if component.getState(stream.as_ptr()) == kResultOk {
                    status(stream.seek(0, 0, ptr::null_mut()), "rewind state")?;
                    let _ = c.setComponentState(stream.as_ptr());
                }
            }
        }
        let mut params = vec![];
        if let Some(c) = &controller {
            // SAFETY: initialized controller; count is bounded before use.
            let count = unsafe { c.getParameterCount() };
            if !(0..=MAX_PLUGIN_PARAMETERS as i32).contains(&count) {
                return Err("VST3 parameter limit exceeded".into());
            }
            for i in 0..count {
                // SAFETY: ParameterInfo is a C record of integer/floating values and fixed arrays.
                let mut p: ParameterInfo = unsafe { std::mem::zeroed() };
                // SAFETY: initialized controller and writable parameter record.
                unsafe {
                    status(c.getParameterInfo(i, &mut p), "parameter metadata")?;
                }
                if p.flags & 2 != 0 {
                    continue;
                } // read-only meters are not editable parameters
                if !p.defaultNormalizedValue.is_finite() {
                    return Err("Invalid VST3 parameter default".into());
                }
                params.push(PluginParameter {
                    id: p.id.to_string(),
                    name: wide(&p.title),
                    unit: String::new(), // VST values are normalized, not the vendor display unit.
                    min: 0.0,
                    max: 1.0,
                    default: p.defaultNormalizedValue.clamp(0.0, 1.0),
                    steps: p.stepCount.max(0) as u32,
                });
            }
        }
        let mut this = Self {
            component,
            processor,
            controller,
            controller_separate: separate,
            _host: host,
            _module: module,
            channels,
            input_buses: 1,
            output_buses: 1,
            params,
            previous: BTreeMap::new(),
            latency: 0,
            tail: 0,
            sample_rate: sr,
            position: 0,
        };
        // SAFETY: owned live interfaces, supported format, valid bus arrangement arrays and setup.
        unsafe {
            let ni = this.component.getBusCount(0, 0);
            let no = this.component.getBusCount(0, 1);
            if !(1..=16).contains(&ni) || !(1..=16).contains(&no) {
                return Err("VST3 requires a main audio input/output (maximum 16 buses)".into());
            }
            this.input_buses = ni as usize;
            this.output_buses = no as usize;
            let arrangement = if channels == 1 { 1u64 << 19 } else { 3 };
            let mut input = vec![0u64; ni as usize];
            let mut output = vec![0u64; no as usize];
            for (i, bus) in input.iter_mut().enumerate() {
                let _ = this.processor.getBusArrangement(0, i as i32, bus);
            }
            for (i, bus) in output.iter_mut().enumerate() {
                let _ = this.processor.getBusArrangement(1, i as i32, bus);
            }
            if let Some(bus) = input.first_mut() {
                *bus = arrangement;
            }
            if let Some(bus) = output.first_mut() {
                *bus = arrangement;
            }
            status(this.processor.setBusArrangements(input.as_mut_ptr(), ni, output.as_mut_ptr(), no), "channel layout")?;
            for i in 1..ni {
                status(this.component.activateBus(0, 0, i, 0), "disable auxiliary input")?;
            }
            for i in 1..no {
                status(this.component.activateBus(0, 1, i, 0), "disable auxiliary output")?;
            }
            status(this.processor.canProcessSampleSize(0), "32-bit float audio")?;
            let mut setup = ProcessSetup { processMode: 0, symbolicSampleSize: 0, maxSamplesPerBlock: MAX_AUDIO_BLOCK as i32, sampleRate: sr as f64 };
            status(this.processor.setupProcessing(&mut setup), "setup processing")?;
            status(this.component.activateBus(0, 0, 0, 1), "activate input")?;
            status(this.component.activateBus(0, 1, 0, 1), "activate output")?;
            status(this.component.setActive(1), "activate")?;
            status(this.processor.setProcessing(1), "start processing")?;
            this.latency = this.processor.getLatencySamples() as usize;
            this.tail = this.processor.getTailSamples().min(sr.saturating_mul(30)) as usize;
        }
        if this.latency > sr as usize * 30 {
            return Err("VST3 latency exceeds 30 seconds".into());
        }
        Ok(this)
    }
    pub fn open(state: &AudioPluginState, sr: u32, channels: usize) -> Result<Self, String> {
        state.validate()?;
        if channels != 1 && channels != 2 {
            return Err("VST3 requires mono or stereo audio".into());
        }

        let raw = state.plugin.native_id.as_bytes();
        if raw.len() != 32 {
            return Err("Invalid VST3 class ID".into());
        }
        let mut id = [0i8; 16];
        for (out, pair) in id.iter_mut().zip(raw.as_chunks::<2>().0) {
            let pair = std::str::from_utf8(pair).map_err(|e| e.to_string())?;
            *out = u8::from_str_radix(pair, 16).map_err(|e| e.to_string())? as i8;
        }
        let mut p = Self::new(std::sync::Arc::new(Module::load(Path::new(&state.plugin.path))?), id, sr, channels)?;
        if !state.state.is_empty() {
            p.restore(&state.state)?;
        }
        Ok(p)
    }
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let component = memory(vec![])?;
        let controller = memory(vec![])?;
        // SAFETY: owned initialized interfaces receive bounded live IBStream objects.
        unsafe {
            status(self.component.getState(component.as_ptr()), "save component")?;
            if let Some(c) = &self.controller {
                let _ = c.getState(controller.as_ptr());
            }
        }
        serde_json::to_vec(&(stream_bytes(&component)?, stream_bytes(&controller)?)).map_err(|e| e.to_string())
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (a, b): (Vec<u8>, Vec<u8>) = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if a.len() > MAX_PLUGIN_STATE || b.len() > MAX_PLUGIN_STATE {
            return Err("State too large".into());
        }
        let a = memory(a)?;
        let b = memory(b)?;
        // SAFETY: initialized owned interfaces, valid bounded streams. Stop DSP before restore.
        unsafe {
            status(self.processor.setProcessing(0), "stop")?;
            status(self.component.setActive(0), "deactivate")?;
            status(self.component.setState(a.as_ptr()), "restore component")?;
            if let Some(c) = &self.controller {
                status(a.seek(0, 0, ptr::null_mut()), "rewind")?;
                let _ = c.setComponentState(a.as_ptr());
                let _ = c.setState(b.as_ptr());
            }
            status(self.component.setActive(1), "activate")?;
            status(self.processor.setProcessing(1), "start")?;
        }
        self.previous.clear();
        Ok(())
    }
    pub fn process(&mut self, audio: Vec<Vec<f32>>, values: &BTreeMap<String, f64>) -> Result<Vec<Vec<f32>>, String> {
        let frames = validate_audio(&audio)?;
        if frames == 0 {
            return Ok(audio);
        }
        if audio.len() != self.channels {
            return Err("VST3 channel count changed".into());
        }
        let mut points = vec![];
        for (id, value) in values {
            if !value.is_finite() || !(0.0..=1.0).contains(value) || !self.params.iter().any(|p| p.id == *id) {
                return Err("Invalid VST3 parameter".into());
            }
            if self.previous.get(id) == Some(value) {
                continue;
            }
            let numeric = id.parse::<u32>().map_err(|e| e.to_string())?;
            if let Some(c) = &self.controller {
                // SAFETY: validated parameter ID and normalized value; controller is initialized.
                unsafe {
                    status(c.setParamNormalized(numeric, *value), "set parameter")?;
                }
            }
            points.push(ComWrapper::new(Point { id: numeric, value: *value }).to_com_ptr::<IParamValueQueue>().ok_or("Cannot create parameter queue")?);
        }
        let changes = ComWrapper::new(Changes { points }).to_com_ptr::<IParameterChanges>().ok_or("Cannot create parameter changes")?;
        let mut input = audio;
        let mut output = vec![vec![0f32; frames]; self.channels];
        let mut ip: Vec<_> = input.iter_mut().map(Vec::as_mut_ptr).collect();
        let mut op: Vec<_> = output.iter_mut().map(Vec::as_mut_ptr).collect();
        let ib = AudioBusBuffers { numChannels: self.channels as i32, silenceFlags: 0, __field0: AudioBusBuffers__type0 { channelBuffers32: ip.as_mut_ptr() } };
        let ob = AudioBusBuffers { numChannels: self.channels as i32, silenceFlags: 0, __field0: AudioBusBuffers__type0 { channelBuffers32: op.as_mut_ptr() } };
        let empty = AudioBusBuffers { numChannels: 0, silenceFlags: u64::MAX, __field0: AudioBusBuffers__type0 { channelBuffers32: ptr::null_mut() } };
        let mut inputs = vec![empty; self.input_buses];
        let mut outputs = vec![empty; self.output_buses];
        if let Some(bus) = inputs.first_mut() {
            *bus = ib;
        }
        if let Some(bus) = outputs.first_mut() {
            *bus = ob;
        }
        // SAFETY: ProcessContext is a C POD; zero marks optional fields invalid.
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        context.sampleRate = self.sample_rate as f64;
        context.projectTimeSamples = self.position;
        context.continousTimeSamples = self.position;
        context.state = 2 | 131072;
        let mut data = ProcessData {
            processMode: 0,
            symbolicSampleSize: 0,
            numSamples: frames as i32,
            numInputs: self.input_buses as i32,
            numOutputs: self.output_buses as i32,
            inputs: inputs.as_mut_ptr(),
            outputs: outputs.as_mut_ptr(),
            inputParameterChanges: changes.as_ptr(),
            outputParameterChanges: ptr::null_mut(),
            inputEvents: ptr::null_mut(),
            outputEvents: ptr::null_mut(),
            processContext: &mut context,
        };
        // SAFETY: all pointers reference live bounded planar buffers and COM queues for this call.
        unsafe {
            status(self.processor.process(&mut data), "process")?;
        }
        // SAFETY: processor remains initialized. Changed look-ahead needs a new compensated graph.
        if unsafe { self.processor.getLatencySamples() } as usize != self.latency {
            return Err("Plugin changed latency; restart playback to rebuild delay compensation".into());
        }
        self.position = self.position.saturating_add(frames as i64);
        validate_audio(&output)?;
        self.previous = values.clone();
        Ok(output)
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        // SAFETY: teardown occurs while all objects and module code remain alive, on this worker.
        unsafe {
            let _ = self.processor.setProcessing(0);
            let _ = self.component.setActive(0);
            if let Some(c) = &self.controller {
                if let (Some(a), Some(b)) = (self.component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                    let _ = a.disconnect(b.as_ptr());
                    let _ = b.disconnect(a.as_ptr());
                }
                let _ = c.setComponentHandler(ptr::null_mut());
                if self.controller_separate {
                    let _ = c.terminate();
                }
            }
            let _ = self.component.terminate();
        }
    }
}
pub fn scan(path: &str) -> Result<Vec<AudioPlugin>, String> {
    let module = std::sync::Arc::new(Module::load(Path::new(path))?);
    // SAFETY: factory is live and info is a C record containing only arrays/integers.
    let mut vendor: PFactoryInfo = unsafe { std::mem::zeroed() };
    // SAFETY: owned factory and writable descriptor.
    unsafe {
        status(module.factory.getFactoryInfo(&mut vendor), "factory info")?;
    }
    // SAFETY: owned factory; count is bounded below.
    let count = unsafe { module.factory.countClasses() };
    if !(0..=1024).contains(&count) {
        return Err("Invalid VST3 class count".into());
    }
    let mut result = vec![];
    for i in 0..count {
        // SAFETY: fixed C record, all-zero is a valid initial representation.
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        // SAFETY: class index is validated and info is writable.
        unsafe {
            status(module.factory.getClassInfo(i, &mut info), "class info")?;
        }
        if text(&info.category) != "Audio Module Class" {
            continue;
        }
        let id = info.cid.iter().map(|b| format!("{:02x}", *b as u8)).collect::<String>();
        // Each candidate is instantiated in this disposable scan worker to verify it is an effect.
        let processor = Processor::new(module.clone(), info.cid, 48000, 2).or_else(|_| Processor::new(module.clone(), info.cid, 48000, 1))?;
        let mut category = "Other".to_string();
        if let Some(factory) = module.factory.cast::<IPluginFactory2>() {
            // SAFETY: PClassInfo2 is a POD, populated by a live factory for this bounded class index.
            unsafe {
                let mut detail: PClassInfo2 = std::mem::zeroed();
                if factory.getClassInfo2(i, &mut detail) == kResultOk {
                    category = text(&detail.subCategories);
                }
            }
        }
        result.push(AudioPlugin {
            format: PluginFormat::Vst3,
            path: path.into(),
            native_id: id,
            name: text(&info.name),
            vendor: text(&vendor.vendor),
            category,
            parameters: processor.params.clone(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_state_stream_rejects_hostile_offsets_and_round_trips() {
        let s = memory(vec![1, 2, 3]).unwrap();
        assert_eq!(stream_bytes(&s).unwrap(), vec![1, 2, 3]);
        // SAFETY: test calls owned bounded stream with valid pointers or deliberate rejected nulls.
        unsafe {
            assert_eq!(s.seek(0, 0, ptr::null_mut()), kResultOk);
            let mut block = [0u8; 16];
            let mut actual = 0;
            assert_eq!(s.read(block.as_mut_ptr().cast(), 16, &mut actual), kResultOk);
            assert_eq!(actual, 3);
            assert_eq!(&block[..3], &[1, 2, 3]);
            assert_eq!(s.read(block.as_mut_ptr().cast(), 16, &mut actual), kResultFalse);
            assert_eq!(actual, 0);
            assert_eq!(s.seek(-1, 0, ptr::null_mut()), kInvalidArgument);
            assert_eq!(s.seek(i64::MAX, 1, ptr::null_mut()), kInvalidArgument);
            assert_eq!(s.write(ptr::null_mut(), 1, ptr::null_mut()), kInvalidArgument);
            assert_eq!(s.read(ptr::null_mut(), -1, ptr::null_mut()), kInvalidArgument);
            assert_eq!(s.seek(MAX_PLUGIN_STATE as i64, 0, ptr::null_mut()), kResultOk);
            let mut b = 0u8;
            assert_eq!(s.write((&mut b as *mut u8).cast(), 1, ptr::null_mut()), kOutOfMemory);
        }
    }
}
