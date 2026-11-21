//! Microphones and speakers through Core Audio's audio queues. The system
//! converts our 48 kHz mono to and from whatever the device uses.

use super::{Device, Devices, OnFrame, Pull, Stream};
use crate::codec::{FRAME, SAMPLE_RATE};
use std::ffi::c_void;
use std::ptr::{null, null_mut};

type OSStatus = i32;
type AudioQueueRef = *mut c_void;
type CFStringRef = *const c_void;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const SYSTEM_OBJECT: u32 = 1;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_INPUT: u32 = fourcc(b"inpt");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");
const ELEMENT_MAIN: u32 = 0;
const PROP_DEVICES: u32 = fourcc(b"dev#");
const PROP_NAME: u32 = fourcc(b"lnam");
const PROP_UID: u32 = fourcc(b"uid ");
const PROP_STREAMS: u32 = fourcc(b"stm#");
const QUEUE_CURRENT_DEVICE: u32 = fourcc(b"aqcd");
const FORMAT_LPCM: u32 = fourcc(b"lpcm");
const FLAG_FLOAT: u32 = 1;
const FLAG_PACKED: u32 = 8;
const UTF8: u32 = 0x0800_0100;

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
struct StreamFormat {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits: u32,
    reserved: u32,
}

#[repr(C)]
struct QueueBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user: *mut c_void,
    descriptions_capacity: u32,
    descriptions: *mut c_void,
    descriptions_count: u32,
}

type InputCallback = extern "C" fn(*mut c_void, AudioQueueRef, *mut QueueBuffer, *const c_void, u32, *const c_void);
type OutputCallback = extern "C" fn(*mut c_void, AudioQueueRef, *mut QueueBuffer);

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(object: u32, address: *const PropertyAddress, qualifier_size: u32, qualifier: *const c_void, size: *mut u32) -> OSStatus;
    fn AudioObjectGetPropertyData(object: u32, address: *const PropertyAddress, qualifier_size: u32, qualifier: *const c_void, size: *mut u32, data: *mut c_void) -> OSStatus;
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewInput(format: *const StreamFormat, callback: InputCallback, user: *mut c_void, run_loop: *const c_void, mode: *const c_void, flags: u32, queue: *mut AudioQueueRef) -> OSStatus;
    fn AudioQueueNewOutput(format: *const StreamFormat, callback: OutputCallback, user: *mut c_void, run_loop: *const c_void, mode: *const c_void, flags: u32, queue: *mut AudioQueueRef) -> OSStatus;
    fn AudioQueueAllocateBuffer(queue: AudioQueueRef, size: u32, buffer: *mut *mut QueueBuffer) -> OSStatus;
    fn AudioQueueEnqueueBuffer(queue: AudioQueueRef, buffer: *mut QueueBuffer, descriptions: u32, list: *const c_void) -> OSStatus;
    fn AudioQueueSetProperty(queue: AudioQueueRef, id: u32, data: *const c_void, size: u32) -> OSStatus;
    fn AudioQueueStart(queue: AudioQueueRef, at: *const c_void) -> OSStatus;
    fn AudioQueueStop(queue: AudioQueueRef, immediate: u8) -> OSStatus;
    fn AudioQueueDispose(queue: AudioQueueRef, immediate: u8) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetLength(s: CFStringRef) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(s: CFStringRef, buffer: *mut u8, size: isize, encoding: u32) -> u8;
    fn CFStringCreateWithBytes(allocator: *const c_void, bytes: *const u8, length: isize, encoding: u32, external: u8) -> CFStringRef;
    fn CFRelease(object: *const c_void);
}

fn check(what: &str, status: OSStatus) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

/// Read a CFString property and release it.
fn string_prop(device: u32, selector: u32) -> Option<String> {
    let address = PropertyAddress { selector, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut value: CFStringRef = null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    unsafe {
        if AudioObjectGetPropertyData(device, &address, 0, null(), &mut size, (&mut value as *mut CFStringRef).cast()) != 0 || value.is_null() {
            return None;
        }
        let max = CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) + 1;
        let mut buf = vec![0u8; max as usize];
        let ok = CFStringGetCString(value, buf.as_mut_ptr(), max, UTF8) != 0;
        CFRelease(value);
        if !ok {
            return None;
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }
}

fn has_streams(device: u32, scope: u32) -> bool {
    let address = PropertyAddress { selector: PROP_STREAMS, scope, element: ELEMENT_MAIN };
    let mut size = 0u32;
    unsafe { AudioObjectGetPropertyDataSize(device, &address, 0, null(), &mut size) == 0 && size > 0 }
}

pub fn devices() -> Result<Devices, String> {
    let address = PropertyAddress { selector: PROP_DEVICES, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut size = 0u32;
    check("listing audio devices", unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &address, 0, null(), &mut size) })?;
    let mut ids = vec![0u32; size as usize / 4];
    check("listing audio devices", unsafe { AudioObjectGetPropertyData(SYSTEM_OBJECT, &address, 0, null(), &mut size, ids.as_mut_ptr().cast()) })?;
    let mut out = Devices::default();
    for id in ids {
        let Some(uid) = string_prop(id, PROP_UID) else { continue };
        let name = string_prop(id, PROP_NAME).unwrap_or_else(|| uid.clone());
        let device = Device { id: uid, name };
        if has_streams(id, SCOPE_INPUT) {
            out.inputs.push(device.clone());
        }
        if has_streams(id, SCOPE_OUTPUT) {
            out.outputs.push(device);
        }
    }
    Ok(out)
}

fn format() -> StreamFormat {
    StreamFormat {
        sample_rate: SAMPLE_RATE as f64,
        format_id: FORMAT_LPCM,
        format_flags: FLAG_FLOAT | FLAG_PACKED,
        bytes_per_packet: 4,
        frames_per_packet: 1,
        bytes_per_frame: 4,
        channels: 1,
        bits: 32,
        reserved: 0,
    }
}

const BUFFER_BYTES: u32 = (FRAME * 4) as u32;
const BUFFERS: usize = 3;

/// A running queue; `state` is what its callback uses.
struct Queue<T: Send> {
    queue: AudioQueueRef,
    state: *mut T,
}

// The queue may be stopped from any thread; `state` is only used by its
// callback until then.
unsafe impl<T: Send> Send for Queue<T> {}

impl<T: Send> Stream for Queue<T> {}

impl<T: Send> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            AudioQueueStop(self.queue, 1);
            AudioQueueDispose(self.queue, 1);
            drop(Box::from_raw(self.state));
        }
    }
}

fn set_device(queue: AudioQueueRef, device: Option<&str>) {
    let Some(id) = device else { return };
    unsafe {
        let uid = CFStringCreateWithBytes(null(), id.as_ptr(), id.len() as isize, UTF8, 0);
        if uid.is_null() {
            return;
        }
        // If the device is gone this fails and the default is used.
        let status = AudioQueueSetProperty(queue, QUEUE_CURRENT_DEVICE, (&uid as *const CFStringRef).cast(), std::mem::size_of::<CFStringRef>() as u32);
        if status != 0 {
            tracing::info!("voice: the chosen audio device is gone, using the default");
        }
        CFRelease(uid);
    }
}

struct InputState {
    on_frame: OnFrame,
    frame: Vec<f32>,
}

extern "C" fn input_callback(user: *mut c_void, queue: AudioQueueRef, buffer: *mut QueueBuffer, _start: *const c_void, _packets: u32, _descriptions: *const c_void) {
    unsafe {
        let state = &mut *(user as *mut InputState);
        let b = &*buffer;
        let samples = std::slice::from_raw_parts(b.data as *const f32, b.size as usize / 4);
        for s in samples {
            state.frame.push(*s);
            if state.frame.len() == FRAME {
                (state.on_frame)(&state.frame);
                state.frame.clear();
            }
        }
        AudioQueueEnqueueBuffer(queue, buffer, 0, null());
    }
}

pub fn open_input(device: Option<&str>, on_frame: OnFrame) -> Result<Box<dyn Stream>, String> {
    let state = Box::into_raw(Box::new(InputState { on_frame, frame: Vec::with_capacity(FRAME) }));
    let mut queue: AudioQueueRef = null_mut();
    unsafe {
        if let Err(e) = check("opening the microphone", AudioQueueNewInput(&format(), input_callback, state.cast(), null(), null(), 0, &mut queue)) {
            drop(Box::from_raw(state));
            return Err(e);
        }
    }
    let running = Queue { queue, state };
    set_device(queue, device);
    unsafe {
        for _ in 0..BUFFERS {
            let mut b = null_mut();
            check("opening the microphone", AudioQueueAllocateBuffer(queue, BUFFER_BYTES, &mut b))?;
            check("opening the microphone", AudioQueueEnqueueBuffer(queue, b, 0, null()))?;
        }
        check("starting the microphone", AudioQueueStart(queue, null()))?;
    }
    Ok(Box::new(running))
}

struct OutputState {
    pull: Pull,
}

extern "C" fn output_callback(user: *mut c_void, queue: AudioQueueRef, buffer: *mut QueueBuffer) {
    unsafe {
        let state = &mut *(user as *mut OutputState);
        let b = &mut *buffer;
        let out = std::slice::from_raw_parts_mut(b.data as *mut f32, FRAME);
        (state.pull)(out);
        b.size = BUFFER_BYTES;
        AudioQueueEnqueueBuffer(queue, buffer, 0, null());
    }
}

pub fn open_output(device: Option<&str>, pull: Pull) -> Result<Box<dyn Stream>, String> {
    let state = Box::into_raw(Box::new(OutputState { pull }));
    let mut queue: AudioQueueRef = null_mut();
    unsafe {
        if let Err(e) = check("opening the speakers", AudioQueueNewOutput(&format(), output_callback, state.cast(), null(), null(), 0, &mut queue)) {
            drop(Box::from_raw(state));
            return Err(e);
        }
    }
    let running = Queue { queue, state };
    set_device(queue, device);
    unsafe {
        for _ in 0..BUFFERS {
            let mut b = null_mut();
            check("opening the speakers", AudioQueueAllocateBuffer(queue, BUFFER_BYTES, &mut b))?;
            output_callback(state.cast(), queue, b);
        }
        check("starting the speakers", AudioQueueStart(queue, null()))?;
    }
    Ok(Box::new(running))
}
