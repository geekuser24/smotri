//! Audio Unit v2 effect host. Only this disposable process calls AudioToolbox.
#![allow(unsafe_code, non_snake_case)]
use filmcraft_project::audio_plugin::*;
use std::{collections::BTreeMap, ffi::c_void, ptr};
type Unit = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Description {
    kind: u32,
    sub: u32,
    vendor: u32,
    flags: u32,
    mask: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Format {
    rate: f64,
    id: u32,
    flags: u32,
    packet: u32,
    frames: u32,
    frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Smpte {
    subframes: i16,
    divisor: i16,
    counter: u32,
    kind: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Time {
    sample: f64,
    host: u64,
    rate: f64,
    word: u64,
    smpte: Smpte,
    flags: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Buffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}
#[repr(C)]
struct Buffers {
    count: u32,
    buffers: [Buffer; 2],
}
#[repr(C)]
struct Callback {
    proc: unsafe extern "C" fn(*mut c_void, *mut u32, *const Time, u32, u32, *mut Buffers) -> i32,
    context: *mut c_void,
}
#[repr(C)]
struct Parameter {
    name: [i8; 52],
    unit_name: *const c_void,
    clump: u32,
    cf_name: *const c_void,
    unit: u32,
    min: f32,
    max: f32,
    default: f32,
    flags: u32,
}
#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(component: Unit, desc: *const Description) -> Unit;
    fn AudioComponentGetDescription(component: Unit, desc: *mut Description) -> i32;
    fn AudioComponentCopyName(component: Unit, name: *mut *const c_void) -> i32;
    fn AudioComponentInstanceNew(component: Unit, unit: *mut Unit) -> i32;
    fn AudioComponentInstanceDispose(unit: Unit) -> i32;
    fn AudioUnitInitialize(unit: Unit) -> i32;
    fn AudioUnitUninitialize(unit: Unit) -> i32;
    fn AudioUnitGetPropertyInfo(unit: Unit, id: u32, scope: u32, element: u32, size: *mut u32, writable: *mut u8) -> i32;
    fn AudioUnitGetProperty(unit: Unit, id: u32, scope: u32, element: u32, data: *mut c_void, size: *mut u32) -> i32;
    fn AudioUnitSetProperty(unit: Unit, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> i32;
    fn AudioUnitSetParameter(unit: Unit, id: u32, scope: u32, element: u32, value: f32, offset: u32) -> i32;
    fn AudioUnitRender(unit: Unit, flags: *mut u32, time: *const Time, bus: u32, frames: u32, data: *mut Buffers) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFStringGetCString(value: *const c_void, buffer: *mut i8, size: isize, encoding: u32) -> u8;
    fn CFPropertyListCreateData(alloc: *const c_void, value: *const c_void, format: isize, options: usize, error: *mut *const c_void) -> *const c_void;
    fn CFPropertyListCreateWithData(alloc: *const c_void, data: *const c_void, options: usize, format: *mut isize, error: *mut *const c_void) -> *const c_void;
    fn CFDataCreate(alloc: *const c_void, bytes: *const u8, len: isize) -> *const c_void;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
}
fn status(code: i32, op: &str) -> Result<(), String> {
    if code == 0 { Ok(()) } else { Err(format!("Audio Unit {op}: {code}")) }
}
fn cf_text(value: *const c_void) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut text = [0i8; 1024];
    // SAFETY: caller provides a live CFString; output buffer is valid and bounded.
    if unsafe { CFStringGetCString(value, text.as_mut_ptr(), text.len() as isize, 0x08000100) } == 0 {
        return String::new();
    }
    String::from_utf8_lossy(&text.iter().take_while(|b| **b != 0).map(|b| *b as u8).collect::<Vec<_>>()).into_owned()
}
struct Cf(*const c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this wrapper owns one Create/Copy reference.
            unsafe { CFRelease(self.0) };
        }
    }
}
fn description(id: &str) -> Result<Description, String> {
    let values = id.split(':').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    match values.as_slice() {
        [kind, sub, vendor] if *kind == u32::from_be_bytes(*b"aufx") || *kind == u32::from_be_bytes(*b"aumf") => {
            Ok(Description { kind: *kind, sub: *sub, vendor: *vendor, ..Default::default() })
        }
        _ => Err("Invalid Audio Unit ID".into()),
    }
}
fn set<T>(unit: Unit, id: u32, scope: u32, value: &T) -> Result<(), String> {
    // SAFETY: initialized component instance and property-specific POD/reference with exact size.
    status(unsafe { AudioUnitSetProperty(unit, id, scope, 0, (value as *const T).cast(), std::mem::size_of::<T>() as u32) }, "set property")
}
fn get<T: Default>(unit: Unit, id: u32, element: u32) -> Result<T, String> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: property-specific output record with exact writable size; API reports result length.
    status(unsafe { AudioUnitGetProperty(unit, id, 0, element, (&mut value as *mut T).cast(), &mut size) }, "get property")?;
    if size != std::mem::size_of::<T>() as u32 {
        return Err("Audio Unit property size mismatch".into());
    }
    Ok(value)
}
struct Input {
    audio: Vec<Vec<f32>>,
}
unsafe extern "C" fn input(context: *mut c_void, _flags: *mut u32, _time: *const Time, _bus: u32, frames: u32, data: *mut Buffers) -> i32 {
    if context.is_null() || data.is_null() || frames as usize > MAX_AUDIO_BLOCK {
        return -50;
    }
    // SAFETY: context is the stable boxed Input registered for this unit's lifetime.
    let input = unsafe { &*(context as *const Input) };
    // SAFETY: AudioUnit supplies an AudioBufferList with count valid descriptors. We only use up to two.
    let count = unsafe { (*data).count as usize };
    if count != input.audio.len() || count > 2 {
        return -50;
    }
    for (index, channel) in input.audio.iter().enumerate() {
        if channel.len() < frames as usize {
            return -50;
        }
        // SAFETY: index is below the AudioUnit-supplied count (at most two). Source lives throughout render.
        unsafe {
            let buffer = ptr::addr_of_mut!((*data).buffers).cast::<Buffer>().add(index);
            if (*buffer).data.is_null() {
                (*buffer).data = channel.as_ptr().cast_mut().cast();
            } else {
                if (*buffer).size < (frames * 4) {
                    return -50;
                }
                ptr::copy_nonoverlapping(channel.as_ptr(), (*buffer).data.cast(), frames as usize);
            }
            (*buffer).size = frames * 4;
            (*buffer).channels = 1;
        }
    }
    0
}
pub struct Processor {
    unit: Unit,
    input: Box<Input>,
    channels: usize,
    params: Vec<PluginParameter>,
    position: f64,
    pub latency: usize,
    pub tail: usize,
}
impl Drop for Processor {
    fn drop(&mut self) {
        // SAFETY: unit remains live through uninitialization/disposal, before input callback context drops.
        unsafe {
            let _ = AudioUnitUninitialize(self.unit);
            let _ = AudioComponentInstanceDispose(self.unit);
        }
    }
}
impl Processor {
    pub fn open(state: &AudioPluginState, sr: u32, channels: usize) -> Result<Self, String> {
        state.validate()?;
        if !(1..=2).contains(&channels) {
            return Err("Audio Units currently require mono or stereo".into());
        }
        let desc = description(&state.plugin.native_id)?;
        // SAFETY: valid component description; returned registry reference belongs to AudioToolbox.
        let component = unsafe { AudioComponentFindNext(ptr::null_mut(), &desc) };
        if component.is_null() {
            return Err(format!("Audio Unit is not installed: {}", state.plugin.name));
        }
        let mut unit = ptr::null_mut();
        // SAFETY: writable output receives the owned instance of a registered component.
        status(unsafe { AudioComponentInstanceNew(component, &mut unit) }, "instantiate")?;
        if unit.is_null() {
            return Err("Null Audio Unit instance".into());
        }
        let mut p = Self { unit, input: Box::new(Input { audio: vec![vec![]; channels] }), channels, params: vec![], position: 0.0, latency: 0, tail: 0 };
        let format = Format {
            rate: sr as f64,
            id: u32::from_be_bytes(*b"lpcm"),
            flags: 1 | 8 | 32,
            packet: 4,
            frames: 1,
            frame: 4,
            channels: channels as u32,
            bits: 32,
            reserved: 0,
        };
        set(unit, 8, 1, &format)?;
        set(unit, 8, 2, &format)?;
        set(unit, 14, 0, &(MAX_AUDIO_BLOCK as u32))?;
        let cb = Callback { proc: input, context: (&mut *p.input as *mut Input).cast() };
        set(unit, 23, 1, &cb)?;
        if !state.state.is_empty() {
            // SAFETY: bounded state bytes copied into an owned CFData.
            let data = Cf(unsafe { CFDataCreate(ptr::null(), state.state.as_ptr(), state.state.len() as isize) });
            if data.0.is_null() {
                return Err("Cannot allocate Audio Unit state".into());
            }
            // SAFETY: valid owned CFData; immutable property list returned with Create ownership.
            let plist = Cf(unsafe { CFPropertyListCreateWithData(ptr::null(), data.0, 0, ptr::null_mut(), ptr::null_mut()) });
            if plist.0.is_null() {
                return Err("Invalid Audio Unit state".into());
            }
            set(unit, 0, 0, &plist.0)?;
        }
        // SAFETY: formats, callback and maximum frame count set before initialization.
        status(unsafe { AudioUnitInitialize(unit) }, "initialize")?;
        p.params = p.parameters()?;
        let latency = get::<f64>(unit, 12, 0).unwrap_or(0.0);
        let tail = get::<f64>(unit, 20, 0).unwrap_or(0.0);
        if !latency.is_finite() || !(0.0..=30.0).contains(&latency) {
            return Err("Invalid Audio Unit latency".into());
        }
        p.latency = (latency * sr as f64).ceil() as usize;
        p.tail = if tail.is_finite() { (tail.clamp(0.0, 30.0) * sr as f64).ceil() as usize } else { 0 };
        Ok(p)
    }
    fn parameters(&self) -> Result<Vec<PluginParameter>, String> {
        let mut size = 0;
        let mut writable = 0;
        // SAFETY: valid unit and writable scalar outputs for global parameter-list metadata.
        status(unsafe { AudioUnitGetPropertyInfo(self.unit, 3, 0, 0, &mut size, &mut writable) }, "parameters")?;
        if size as usize > MAX_PLUGIN_PARAMETERS * 4 || size % 4 != 0 {
            return Err("Invalid Audio Unit parameter count".into());
        }
        let mut ids = vec![0u32; size as usize / 4];
        if size == 0 {
            return Ok(vec![]);
        }
        // SAFETY: ids allocation is exactly the property size reported and bounded above.
        status(unsafe { AudioUnitGetProperty(self.unit, 3, 0, 0, ids.as_mut_ptr().cast(), &mut size) }, "parameter IDs")?;
        let mut result = vec![];
        for id in ids {
            // SAFETY: Parameter is a C POD containing raw nullable pointers and numbers.
            let mut info: Parameter = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<Parameter>() as u32;
            // SAFETY: live unit and exact writable Parameter record.
            status(unsafe { AudioUnitGetProperty(self.unit, 4, 0, id, (&mut info as *mut Parameter).cast(), &mut size) }, "parameter info")?;
            let name = if info.flags & (1 << 27) != 0 {
                cf_text(info.cf_name)
            } else {
                String::from_utf8_lossy(&info.name.iter().take_while(|b| **b != 0).map(|b| *b as u8).collect::<Vec<_>>()).into_owned()
            };
            if info.flags & (1 << 4) != 0 && !info.cf_name.is_null() {
                // SAFETY: CFNameRelease flag transfers ownership of the copied CFString.
                unsafe { CFRelease(info.cf_name) };
            }
            if info.flags & (1 << 31) == 0 {
                continue;
            }
            if !info.min.is_finite() || !info.max.is_finite() || info.min > info.max || !info.default.is_finite() {
                return Err("Invalid Audio Unit parameter bounds".into());
            }
            result.push(PluginParameter {
                id: id.to_string(),
                name,
                unit: String::new(),
                min: info.min as f64,
                max: info.max as f64,
                default: info.default.clamp(info.min, info.max) as f64,
                steps: 0,
            });
        }
        Ok(result)
    }
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let value = Cf(get::<*const c_void>(self.unit, 0, 0)?);
        if value.0.is_null() {
            return Err("Empty Audio Unit state".into());
        }
        // SAFETY: owned property-list object; CreateData serializes to immutable binary format.
        let data = Cf(unsafe { CFPropertyListCreateData(ptr::null(), value.0, 200, 0, ptr::null_mut()) });
        if data.0.is_null() {
            return Err("Cannot serialize Audio Unit state".into());
        }
        // SAFETY: valid CFData; length checked before forming a slice of its owned storage.
        let len = unsafe { CFDataGetLength(data.0) };
        if !(0..=MAX_PLUGIN_STATE as isize).contains(&len) {
            return Err("Audio Unit state too large".into());
        }
        if len == 0 {
            return Ok(vec![]);
        }
        // SAFETY: CFData contains len readable bytes and remains retained until copy completes.
        Ok(unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(data.0), len as usize) }.to_vec())
    }
    pub fn process(&mut self, audio: Vec<Vec<f32>>, values: &BTreeMap<String, f64>) -> Result<Vec<Vec<f32>>, String> {
        let frames = validate_audio(&audio)?;
        if audio.len() != self.channels {
            return Err("Audio Unit channels changed".into());
        }
        for (id, value) in values {
            let p = self.params.iter().find(|p| p.id == *id).ok_or("Unknown Audio Unit parameter")?;
            if !value.is_finite() || !(p.min..=p.max).contains(value) {
                return Err("Invalid Audio Unit parameter value".into());
            }
            let id = id.parse::<u32>().map_err(|e| e.to_string())?;
            // SAFETY: validated parameter and range on the owned initialized unit.
            status(unsafe { AudioUnitSetParameter(self.unit, id, 0, 0, *value as f32, 0) }, "parameter")?;
        }
        self.input.audio = audio;
        let mut out = vec![vec![0.0f32; frames]; self.channels];
        let mut buffers = Buffers { count: self.channels as u32, buffers: [Buffer { channels: 1, size: frames as u32 * 4, data: ptr::null_mut() }; 2] };
        for (buffer, channel) in buffers.buffers.iter_mut().zip(&mut out) {
            buffer.data = channel.as_mut_ptr().cast();
        }
        let time = Time { sample: self.position, flags: 1, ..Default::default() };
        let mut flags = 0;
        // SAFETY: timestamp monotonically advances; input and output buffers live through this synchronous call.
        status(unsafe { AudioUnitRender(self.unit, &mut flags, &time, 0, frames as u32, &mut buffers) }, "render")?;
        self.position += frames as f64;
        validate_audio(&out)?;
        Ok(out)
    }
}
pub fn scan(path: &str) -> Result<Vec<AudioPlugin>, String> {
    let mut result = vec![];
    for kind in [u32::from_be_bytes(*b"aufx"), u32::from_be_bytes(*b"aumf")] {
        let desc = Description { kind, ..Default::default() };
        let mut component = ptr::null_mut();
        for _ in 0..1024 {
            // SAFETY: registry enumeration uses the previous registry-owned reference and valid filter.
            component = unsafe { AudioComponentFindNext(component, &desc) };
            if component.is_null() {
                break;
            }
            let mut d = Description::default();
            let mut name = ptr::null();
            // SAFETY: writable descriptor/name outputs; name follows Copy ownership.
            unsafe {
                status(AudioComponentGetDescription(component, &mut d), "description")?;
                status(AudioComponentCopyName(component, &mut name), "name")?;
            }
            let text = cf_text(name);
            let _name = Cf(name);
            let id = format!("{}:{}:{}", d.kind, d.sub, d.vendor);
            if !path.is_empty() && path != id {
                continue;
            }
            let (vendor, name) = text.split_once(':').unwrap_or(("Audio Units", text.as_str()));
            let mut plugin = AudioPlugin {
                format: PluginFormat::AudioUnit,
                path: String::new(),
                native_id: id,
                name: name.trim().into(),
                vendor: vendor.trim().into(),
                category: "Audio Units".into(),
                parameters: vec![],
            };
            if !path.is_empty() {
                plugin.parameters =
                    Processor::open(&AudioPluginState { plugin: plugin.clone(), values: BTreeMap::new(), state: vec![] }, 48000, 2)?.params.clone();
            }
            result.push(plugin);
        }
    }
    Ok(result)
}
