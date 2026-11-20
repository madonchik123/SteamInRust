//! Inline function hooks: overwrite a function's first instructions with a
//! jump to ours, and keep a "trampoline" (the moved instructions plus a jump
//! back) to call the original.
//!
//! Import-table patching (see `input.rs`) only catches calls a module makes
//! through its own imports. Engines look XInput up with `GetProcAddress`
//! when they start, long before this DLL is loaded, so its functions have to
//! be patched where they are.
//!
//! The moved instructions are decoded by a small length decoder that knows
//! the instructions found at the start of system functions (push, mov, sub,
//! lea, …). Anything it doesn't know, or anything relative to the
//! instruction pointer, makes the hook give up (and the function stays
//! untouched) instead of guessing.

use std::ffi::c_void;

const IS64: bool = cfg!(target_pointer_width = "64");

/// Length of the instruction at `code`, or `None` when it can't be moved
/// safely (unknown, relative jump or call, RIP-relative operand, return).
pub fn insn_len(code: &[u8]) -> Option<usize> {
    let mut i = 0;
    let mut opsize16 = false;
    let mut rex_w = false;
    // Prefixes.
    loop {
        match *code.get(i)? {
            0x66 => opsize16 = true,
            0x67 | 0xF2 | 0xF3 | 0x2E | 0x3E | 0x26 | 0x64 | 0x65 | 0x36 => {}
            _ => break,
        }
        i += 1;
    }
    if IS64 && (0x40..=0x4F).contains(code.get(i)?) {
        rex_w = code[i] & 0x08 != 0;
        i += 1;
    }
    let imm32 = if opsize16 { 2 } else { 4 };
    let op = *code.get(i)?;
    i += 1;
    // `Some(extra)`: a ModRM operand follows, then `extra` immediate bytes.
    let (modrm_imm, plain): (Option<usize>, usize) = match op {
        0x50..=0x5F | 0x90 | 0xCC => (None, 0),
        0x6A => (None, 1),
        0x68 => (None, imm32),
        0xB0..=0xB7 => (None, 1),
        0xB8..=0xBF => (None, if rex_w { 8 } else { imm32 }),
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C | 0xA8 => (None, 1),
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D | 0xA9 => (None, imm32),
        0x00..=0x03 | 0x08..=0x0B | 0x10..=0x13 | 0x18..=0x1B | 0x20..=0x23 | 0x28..=0x2B | 0x30..=0x33 | 0x38..=0x3B => (Some(0), 0),
        0x63 | 0x84..=0x8B | 0x8D | 0xD0..=0xD3 | 0xFE => (Some(0), 0),
        0x80 | 0x83 | 0xC0 | 0xC1 | 0xC6 | 0x6B => (Some(1), 0),
        0x82 if !IS64 => (Some(1), 0),
        0x81 | 0xC7 | 0x69 => (Some(imm32), 0),
        0xF6 => (Some(usize::MAX), 1),
        0xF7 => (Some(usize::MAX), imm32),
        0xFF => {
            // inc/dec/push are fine; call/jmp through memory are not moved.
            let reg = (*code.get(i)? >> 3) & 7;
            if matches!(reg, 2..=5) {
                return None;
            }
            (Some(0), 0)
        }
        0x0F => {
            let op2 = *code.get(i)?;
            i += 1;
            match op2 {
                0x1F | 0xB6 | 0xB7 | 0xBE | 0xBF | 0x10 | 0x11 | 0x28 | 0x29 | 0x40..=0x4F | 0xAF => (Some(0), 0),
                _ => return None,
            }
        }
        _ => return None,
    };
    let Some(extra) = modrm_imm else { return Some(i + plain) };
    let modrm = *code.get(i)?;
    i += 1;
    let (md, reg, rm) = (modrm >> 6, (modrm >> 3) & 7, modrm & 7);
    if md != 3 {
        if rm == 4 {
            let sib = *code.get(i)?;
            i += 1;
            if md == 0 && sib & 7 == 5 {
                i += 4;
            }
        } else if md == 0 && rm == 5 {
            // RIP-relative on x64 (moving it would point elsewhere).
            if IS64 {
                return None;
            }
            i += 4;
        }
        i += match md {
            1 => 1,
            2 => 4,
            _ => 0,
        };
    }
    // F6/F7: only `test` (/0, /1) has an immediate.
    let extra = if extra == usize::MAX { if reg <= 1 { plain } else { 0 } } else { extra };
    Some(i + extra)
}

/// Where a function really starts: follows the jump stubs some exports are.
unsafe fn follow(mut p: *const u8) -> *const u8 {
    for _ in 0..4 {
        let b = unsafe { std::slice::from_raw_parts(p, 6) };
        if b[0] == 0xE9 {
            let rel = i32::from_le_bytes([b[1], b[2], b[3], b[4]]);
            p = unsafe { p.add(5).offset(rel as isize) };
        } else if b[0] == 0xFF && b[1] == 0x25 {
            let disp = i32::from_le_bytes([b[2], b[3], b[4], b[5]]);
            let slot = if IS64 { (unsafe { p.add(6).offset(disp as isize) }) as *const usize } else { disp as u32 as usize as *const usize };
            p = unsafe { slot.read_unaligned() } as *const u8;
        } else {
            break;
        }
    }
    p
}

/// Executable memory for a trampoline, within ±2 GB of `near` on x64 (so a
/// 5-byte jump can reach it).
unsafe fn alloc_near(near: usize, size: usize) -> Option<*mut u8> {
    use windows::Win32::System::Memory::{MEM_COMMIT, MEM_FREE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualQuery};
    if !IS64 {
        let p = unsafe { VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
        return (!p.is_null()).then_some(p as *mut u8);
    }
    const STEP: usize = 0x10000;
    const RANGE: usize = 0x7000_0000;
    let lo = near.saturating_sub(RANGE) & !(STEP - 1);
    let hi = near.saturating_add(RANGE);
    // Search downwards and upwards from the target for a free region.
    for dir in [false, true] {
        let mut addr = near & !(STEP - 1);
        while addr > lo && addr < hi {
            addr = if dir { addr + STEP } else { addr - STEP };
            let mut info = MEMORY_BASIC_INFORMATION::default();
            if unsafe { VirtualQuery(Some(addr as *const c_void), &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                continue;
            }
            if info.State == MEM_FREE {
                let p = unsafe { VirtualAlloc(Some(addr as *const c_void), size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
                if !p.is_null() {
                    return Some(p as *mut u8);
                }
            }
        }
    }
    None
}

fn jmp_abs(out: &mut Vec<u8>, to: usize) {
    if IS64 {
        // jmp [rip+0]; dq to
        out.extend_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
        out.extend_from_slice(&(to as u64).to_le_bytes());
    } else {
        // push to; ret
        out.push(0x68);
        out.extend_from_slice(&(to as u32).to_le_bytes());
        out.push(0xC3);
    }
}

/// Point `target` at `detour`. `original` gets the address to call for the
/// original function before the jump goes live (the detour may run at once).
/// False when the function can't be hooked safely; then it's left as it was.
pub unsafe fn install(target: *const c_void, detour: usize, original: impl FnOnce(usize)) -> bool {
    use windows::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect};
    use windows::Win32::System::Threading::GetCurrentProcess;
    if target.is_null() {
        return false;
    }
    let target = unsafe { follow(target as *const u8) } as *mut u8;
    let code = unsafe { std::slice::from_raw_parts(target, 32) };
    // Whole instructions covering the 5-byte jump.
    let mut n = 0;
    while n < 5 {
        match insn_len(&code[n..]) {
            Some(len) => n += len,
            None => return false,
        }
    }
    let Some(mem) = (unsafe { alloc_near(target as usize, 64) }) else { return false };
    // Trampoline: the moved instructions, then back to the rest.
    let mut tramp = code[..n].to_vec();
    jmp_abs(&mut tramp, target as usize + n);
    // x64: a relay next to it reaches the detour wherever it is.
    let relay_at = tramp.len();
    if IS64 {
        jmp_abs(&mut tramp, detour);
    }
    unsafe { std::ptr::copy_nonoverlapping(tramp.as_ptr(), mem, tramp.len()) };
    original(mem as usize);
    let jump_to = if IS64 { mem as usize + relay_at } else { detour };
    let rel = (jump_to as isize - (target as isize + 5)) as i32;
    let mut patch = vec![0xE9];
    patch.extend_from_slice(&rel.to_le_bytes());
    patch.resize(n, 0x90);
    unsafe {
        let mut old = PAGE_PROTECTION_FLAGS(0);
        if VirtualProtect(target as *const c_void, n, PAGE_EXECUTE_READWRITE, &mut old).is_err() {
            return false;
        }
        std::ptr::copy_nonoverlapping(patch.as_ptr(), target, n);
        let _ = VirtualProtect(target as *const c_void, n, old, &mut old);
        let _ = FlushInstructionCache(GetCurrentProcess(), Some(target as *const c_void), n);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_common_prologues() {
        // x86: mov edi,edi; push ebp; mov ebp,esp; sub esp,10h
        if !IS64 {
            assert_eq!(insn_len(&[0x8B, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(2));
        }
        let pad = |v: &[u8]| {
            let mut b = v.to_vec();
            b.resize(16, 0xCC);
            b
        };
        assert_eq!(insn_len(&pad(&[0x55])), Some(1));
        assert_eq!(insn_len(&pad(&[0x83, 0xEC, 0x10])), Some(3));
        assert_eq!(insn_len(&pad(&[0x81, 0xEC, 0, 1, 0, 0])), Some(6));
        assert_eq!(insn_len(&pad(&[0x6A, 0x14])), Some(2));
        assert_eq!(insn_len(&pad(&[0x68, 1, 2, 3, 4])), Some(5));
        // Relative branches are never moved.
        assert_eq!(insn_len(&pad(&[0xE8, 0, 0, 0, 0])), None);
        assert_eq!(insn_len(&pad(&[0xEB, 0x10])), None);
        assert_eq!(insn_len(&pad(&[0xC3])), None);
        if IS64 {
            // mov [rsp+8], rbx; push rdi; sub rsp, 20h
            assert_eq!(insn_len(&pad(&[0x48, 0x89, 0x5C, 0x24, 0x08])), Some(5));
            assert_eq!(insn_len(&pad(&[0x40, 0x57])), Some(2));
            assert_eq!(insn_len(&pad(&[0x48, 0x83, 0xEC, 0x20])), Some(4));
            // mov rax, [rip+x] is RIP-relative: refused.
            assert_eq!(insn_len(&pad(&[0x48, 0x8B, 0x05, 1, 2, 3, 4])), None);
            // mov rax, imm64
            assert_eq!(insn_len(&pad(&[0x48, 0xB8, 1, 2, 3, 4, 5, 6, 7, 8])), Some(10));
        }
    }

    /// Hooks a function whose start looks like a system DLL's (built from
    /// raw bytes, so the compiler can't change it) and calls both ways.
    #[test]
    fn hooks_and_calls_original() {
        use windows::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
        // add(a, b) -> a + b, stdcall on x86, the Windows ABI on x64.
        let code: &[u8] = if IS64 {
            // mov [rsp+8],rbx; lea eax,[rcx+rdx]; mov rbx,[rsp+8]; ret
            &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x8D, 0x04, 0x11, 0x48, 0x8B, 0x5C, 0x24, 0x08, 0xC3]
        } else {
            // mov edi,edi; push ebp; mov ebp,esp; mov eax,[ebp+8]; add eax,[ebp+0Ch]; pop ebp; ret 8
            &[0x8B, 0xFF, 0x55, 0x8B, 0xEC, 0x8B, 0x45, 0x08, 0x03, 0x45, 0x0C, 0x5D, 0xC2, 0x08, 0x00]
        };
        let mem = unsafe { VirtualAlloc(None, 4096, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as *mut u8;
        assert!(!mem.is_null());
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), mem, code.len()) };
        type Add = unsafe extern "system" fn(u32, u32) -> u32;
        let f: Add = unsafe { std::mem::transmute::<*mut u8, Add>(mem) };
        assert_eq!(unsafe { f(2, 3) }, 5);
        static ORIG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        unsafe extern "system" fn detour(a: u32, b: u32) -> u32 {
            let orig: Add = unsafe { std::mem::transmute(ORIG.load(std::sync::atomic::Ordering::SeqCst)) };
            unsafe { orig(a, b) * 10 }
        }
        let hooked = unsafe { install(mem as *const c_void, detour as *const () as usize, |t| ORIG.store(t, std::sync::atomic::Ordering::SeqCst)) };
        assert!(hooked, "a usual prologue must be hookable");
        assert_eq!(unsafe { f(2, 3) }, 50);
    }
}
