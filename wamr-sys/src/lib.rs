//! Thin Rust wrapper around WAMR for WASI guests (stdout captured via `stick_wamr_vprintf`).

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use core::ffi::{c_char, CStr};

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// WAMR interpreter allocator pool (bytes).
///
/// Must be at least the guest's **max** linear memory (wasm pages × 64 KiB) plus
/// ~1 MiB for module/instance/WAMR. 5 MiB fits StickS3 PSRAM after the `/tmp`
/// arena and accommodates a guest with `--max-memory` up to 4 MiB (64 pages);
/// raise if your guest declares a larger memory.
pub const RUNTIME_HEAP_BYTES: usize = 5 * 1024 * 1024;

/// Provide WAMR's interpreter allocator pool (required on ESP before [`run`]).
pub fn set_runtime_heap(heap: *mut u8, len: usize) {
    unsafe {
        stick_wamr_set_runtime_heap(heap as *mut core::ffi::c_void, len as u32);
    }
}

/// Initialize WAMR once after [`set_runtime_heap`] (call from ProCpu boot, not AppCpu).
pub fn init_runtime() -> Result<(), ()> {
    let rc = unsafe { stick_wamr_init_runtime() };
    if rc != 0 {
        Err(())
    } else {
        Ok(())
    }
}

fn copy_err_msg(src: &[u8], dst: &mut [u8]) {
    let n = src
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(src.len())
        .min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
}

#[cfg(not(feature = "std"))]
fn c_string_table_argv(strings: &[&str]) -> alloc::vec::Vec<alloc::ffi::CString> {
    strings
        .iter()
        .map(|s| alloc::ffi::CString::new(*s).unwrap_or_else(|_| alloc::ffi::CString::new("").unwrap()))
        .collect()
}

#[cfg(not(feature = "std"))]
fn c_string_table_env(strings: &[&str]) -> alloc::vec::Vec<alloc::ffi::CString> {
    c_string_table_argv(strings)
}

/// Run a WASI module (`_start`); guest stdout/stderr are captured.
///
/// On failure, writes a NUL-terminated message into `err` (if non-empty).
pub fn run(wasm: &[u8], argv: &[&str], env: &[&str], err: &mut [u8]) -> Result<&'static str, ()> {
    if !err.is_empty() {
        err[0] = 0;
    }

    let mut err_buf = [0u8; 256];

    #[cfg(feature = "std")]
    let rc = {
        let c_argv: Vec<std::ffi::CString> = argv
            .iter()
            .map(|s| std::ffi::CString::new(*s).unwrap())
            .collect();
        let c_env: Vec<std::ffi::CString> = env
            .iter()
            .map(|s| std::ffi::CString::new(*s).unwrap())
            .collect();
        let mut argv_ptrs: Vec<*mut c_char> =
            c_argv.iter().map(|s| s.as_ptr() as *mut c_char).collect();
        let mut env_ptrs: Vec<*const c_char> = c_env.iter().map(|s| s.as_ptr()).collect();
        unsafe {
            stick_wamr_run(
                wasm.as_ptr(),
                wasm.len() as u32,
                argv_ptrs.as_mut_ptr(),
                argv.len() as u32,
                env_ptrs.as_mut_ptr(),
                env.len() as u32,
                err_buf.as_mut_ptr() as *mut c_char,
                err_buf.len() as u32,
            )
        }
    };

    #[cfg(not(feature = "std"))]
    let rc = {
        let c_argv = c_string_table_argv(argv);
        let c_env = c_string_table_env(env);
        let mut argv_ptrs: alloc::vec::Vec<*mut c_char> =
            c_argv.iter().map(|s| s.as_ptr() as *mut c_char).collect();
        let env_ptrs: alloc::vec::Vec<*const c_char> =
            c_env.iter().map(|s| s.as_ptr()).collect();
        unsafe {
            stick_wamr_run(
                wasm.as_ptr(),
                wasm.len() as u32,
                argv_ptrs.as_mut_ptr(),
                argv.len() as u32,
                env_ptrs.as_ptr() as *mut *const c_char,
                env.len() as u32,
                err_buf.as_mut_ptr() as *mut c_char,
                err_buf.len() as u32,
            )
        }
    };

    if rc != 0 {
        if !err.is_empty() {
            copy_err_msg(&err_buf, err);
        }
        return Err(());
    }
    let out = unsafe {
        let ptr = stick_wamr_capture_ptr();
        if ptr.is_null() {
            ""
        } else {
            CStr::from_ptr(ptr).to_str().unwrap_or("")
        }
    };
    Ok(out)
}
