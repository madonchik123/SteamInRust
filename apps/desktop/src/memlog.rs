//! `STEAMINRUST_MEM_LOG=<file>`: every 30 s, one line about memory: the
//! process's private memory, the pictures on the graphics card and what each
//! picture loader keeps, and the background videos. For finding leaks.

use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// (private bytes, working set) of this process.
#[cfg(windows)]
pub fn process_memory() -> (u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb) };
    if ok == 0 {
        return (0, 0);
    }
    (c.PrivateUsage as u64, c.WorkingSetSize as u64)
}

/// (bytes in use, bytes committed) in the default heap (the app's own
/// data), and committed in all other heaps.
#[cfg(windows)]
fn heaps() -> (u64, u64, u64) {
    use windows_sys::Win32::System::Memory::{GetProcessHeap, GetProcessHeaps, HEAP_SUMMARY, HeapSummary};
    let summary = |h| {
        let mut s: HEAP_SUMMARY = unsafe { std::mem::zeroed() };
        s.cb = std::mem::size_of::<HEAP_SUMMARY>() as u32;
        if unsafe { HeapSummary(h, 0, &mut s) } == 0 {
            return (0, 0);
        }
        (s.cbAllocated as u64, s.cbCommitted as u64)
    };
    let main = unsafe { GetProcessHeap() };
    let (used, committed) = summary(main);
    let mut all = vec![std::ptr::null_mut(); 256];
    let n = unsafe { GetProcessHeaps(all.len() as u32, all.as_mut_ptr()) } as usize;
    let others = all[..n.min(all.len())].iter().filter(|h| **h != main).map(|h| summary(*h).1).sum();
    (used, committed, others)
}

#[cfg(not(windows))]
fn heaps() -> (u64, u64, u64) {
    (0, 0, 0)
}

#[cfg(not(windows))]
pub fn process_memory() -> (u64, u64) {
    (0, 0)
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

/// Called every frame; writes a line every 30 s (`extra`: more counts).
pub fn tick(ctx: &egui::Context, extra: impl FnOnce() -> String) {
    static FILE: std::sync::OnceLock<Option<Mutex<(std::fs::File, Instant, Option<Instant>)>>> = std::sync::OnceLock::new();
    let Some(file) = FILE.get_or_init(|| {
        let path = std::env::var_os("STEAMINRUST_MEM_LOG")?;
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some(Mutex::new((f, Instant::now(), None)))
    }) else {
        return;
    };
    let mut g = file.lock().unwrap();
    let (f, start, last) = &mut *g;
    ctx.request_repaint_after(Duration::from_secs(30));
    if last.is_some_and(|l| l.elapsed() < Duration::from_secs(30)) {
        return;
    }
    *last = Some(Instant::now());
    let (private, ws) = process_memory();
    let (heap_used, heap_committed, other_heaps) = heaps();
    let (tex_n, tex_b) = {
        let m = ctx.tex_manager();
        let m = m.read();
        (m.num_allocated(), m.allocated().map(|(_, meta)| meta.bytes_used() as u64).sum::<u64>())
    };
    let loaders = ctx.loaders();
    let short = |id: &str| id.rsplit("::").next().unwrap_or(id).to_string();
    let mut parts = Vec::new();
    for l in loaders.bytes.lock().iter() {
        parts.push(format!("{}={:.1}", short(l.id()), mb(l.byte_size() as u64)));
    }
    for l in loaders.image.lock().iter() {
        parts.push(format!("{}={:.1}", short(l.id()), mb(l.byte_size() as u64)));
    }
    for l in loaders.texture.lock().iter() {
        parts.push(format!("{}={:.1}", short(l.id()), mb(l.byte_size() as u64)));
    }
    let _ = writeln!(
        f,
        "t={:.0}s private={:.1}MB ws={:.1}MB heap={:.1}/{:.1}MB other_heaps={:.1}MB textures={} tex={:.1}MB {} {}",
        start.elapsed().as_secs_f64(),
        mb(private),
        mb(ws),
        mb(heap_used),
        mb(heap_committed),
        mb(other_heaps),
        tex_n,
        mb(tex_b),
        parts.join(" "),
        extra()
    );
    let _ = f.flush();
}
