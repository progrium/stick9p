use std::env;
use std::path::{Path, PathBuf};

fn wamr_root() -> PathBuf {
    if let Ok(root) = env::var("WAMR_ROOT") {
        return PathBuf::from(root);
    }
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../third_party/wasm-micro-runtime")
}

fn is_embedded_target(target: &str) -> bool {
    target.contains("xtensa") || target.contains("esp32") || target.contains("riscv32")
}

fn find_executable(name: &str) -> Option<PathBuf> {
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn esp_toolchain_bin() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    let root = Path::new(&home).join(".rustup/toolchains/esp/xtensa-esp-elf");
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let bin = entry.path().join("xtensa-esp-elf/bin");
        if bin.is_dir() {
            return Some(bin);
        }
    }
    None
}

/// Chip-specific GCC (e.g. `xtensa-esp32s3-elf-gcc`) emits little-endian objects;
/// generic `xtensa-esp-elf-gcc` is big-endian and will not link with esp-hal firmware.
fn embedded_c_compiler(target: &str) -> Option<String> {
    let env_key = format!("CC_{}", target.replace('-', "_"));
    if let Ok(cc) = env::var(&env_key) {
        return Some(cc);
    }
    if let Ok(cc) = env::var("CC") {
        return Some(cc);
    }

    let chip = if target.contains("esp32s3") {
        "esp32s3"
    } else if target.contains("esp32s2") {
        "esp32s2"
    } else if target.contains("esp32c6") {
        "esp32c6"
    } else if target.contains("esp32c3") {
        "esp32c3"
    } else if target.contains("esp32c2") {
        "esp32c2"
    } else if target.contains("esp32h2") {
        "esp32h2"
    } else if target.contains("esp32") {
        "esp32"
    } else if target.contains("esp8266") {
        "esp8266"
    } else {
        return None;
    };

    let gcc = format!("xtensa-{chip}-elf-gcc");
    if let Some(path) = find_executable(&gcc) {
        return Some(path.to_string_lossy().into_owned());
    }
    if let Some(bin) = esp_toolchain_bin() {
        let path = bin.join(&gcc);
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    None
}

fn wamr_platform(target: &str) -> &'static str {
    if target.contains("apple") {
        "darwin"
    } else if is_embedded_target(target) {
        "stick"
    } else {
        "linux"
    }
}

fn wamr_target(target: &str) -> &'static str {
    if target.contains("xtensa") {
        "XTENSA"
    } else if target.contains("riscv32") {
        "RISCV32"
    } else if target.contains("aarch64") {
        "AARCH64"
    } else if target.contains("x86_64") {
        "X86_64"
    } else if target.contains("x86") {
        "X86_32"
    } else {
        "X86_64"
    }
}

fn main() {
    let wamr_root = wamr_root();
    if !wamr_root.join("build-scripts/runtime_lib.cmake").exists() {
        panic!(
            "WAMR source not found at {} — run: git submodule update --init",
            wamr_root.display()
        );
    }

    let target = env::var("TARGET").unwrap_or_default();
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let stick_platform_cmake = manifest_dir.join("platform/stick/shared_platform.cmake");

    println!("cargo:rerun-if-changed=../third_party/wasm-micro-runtime");
    println!("cargo:rerun-if-changed=../third_party/wasm-micro-runtime/core/iwasm/common/wasm_runtime_common.c");
    println!("cargo:rerun-if-changed=c/stick_wamr.c");
    println!("cargo:rerun-if-changed=c/stick_wamr.h");
    println!("cargo:rerun-if-changed=platform/stick");
    println!("cargo:rerun-if-changed=platform/stick/stick_wasi_socket.c");
    println!("cargo:rerun-if-changed=platform/stick/stick_wasi_libc.c");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let vmbuild = out_dir.join("vmbuild");

    let mut cfg = cmake::Config::new(&wamr_root);
    cfg.define("WAMR_BUILD_TARGET", wamr_target(&target))
        .define("WAMR_BUILD_PLATFORM", wamr_platform(&target))
        .define("WAMR_BUILD_INTERP", "1")
        .define("WAMR_BUILD_FAST_INTERP", "0")
        .define("WAMR_BUILD_AOT", "0")
        .define("WAMR_BUILD_JIT", "0")
        .define("WAMR_BUILD_LIBC_BUILTIN", "0")
        .define("WAMR_BUILD_LIBC_WASI", "1")
        .define("WAMR_BUILD_BULK_MEMORY", "1")
        .define("WAMR_BUILD_REF_TYPES", "1")
        .define("WAMR_BUILD_SIMD", "0")
        .define("WAMR_BUILD_MINI_LOADER", "0")
        .define("WAMR_BH_VPRINTF", "stick_wamr_vprintf")
        .define("WAMR_DISABLE_HW_BOUND_CHECK", "1")
        .define("WAMR_DISABLE_WAKEUP_BLOCKING_OP", "1");

    if wamr_platform(&target) == "stick" {
        cfg.define(
            "SHARED_PLATFORM_CONFIG",
            stick_platform_cmake.to_string_lossy().as_ref(),
        );
    }

    if is_embedded_target(&target) {
        cfg.no_default_flags(true);
        if target.contains("xtensa") {
            // PATH_MAX=256: wasm_runtime_init_wasi keeps resolved_path[PATH_MAX] on stack.
            let xtensa_cflags =
                "-mlongcalls -ffunction-sections -fdata-sections -w -DPATH_MAX=256";
            cfg.define("CMAKE_C_FLAGS", xtensa_cflags);
            cfg.define("CMAKE_ASM_FLAGS", xtensa_cflags);
        }
    }

    if let Some(cc_path) = embedded_c_compiler(&target) {
        let mut c_cfg = cc::Build::new();
        c_cfg
            .compiler(&cc_path)
            .no_default_flags(true)
            .cargo_metadata(false);
        cfg.init_c_cfg(c_cfg);

        let cxx_path = cc_path.replace("-gcc", "-g++");
        if Path::new(&cxx_path).is_file() || find_executable(&cxx_path).is_some() {
            let mut cxx_cfg = cc::Build::new();
            cxx_cfg
                .compiler(&cxx_path)
                .no_default_flags(true)
                .cargo_metadata(false);
            cfg.init_cxx_cfg(cxx_cfg);
        }

        // Cross-compiling on macOS: skip link-based compiler sanity checks and
        // avoid injecting host -arch flags into the Xtensa toolchain.
        cfg.define("CMAKE_TRY_COMPILE_TARGET_TYPE", "STATIC_LIBRARY");
        if env::var("HOST").map(|h| h.contains("apple")).unwrap_or(false) {
            cfg.define("CMAKE_OSX_ARCHITECTURES", "");
        }
    }

    let dst = cfg.out_dir(vmbuild).build_target("vmlib").build();
    println!(
        "cargo:rustc-link-search=native={}/build",
        dst.display()
    );
    println!("cargo:rustc-link-lib=static=iwasm");

    if target.contains("apple") || target.contains("linux") {
        println!("cargo:rustc-link-lib=pthread");
    }

    let wamr_include = wamr_root.join("core/iwasm/include");
    let shared_include = wamr_root.join("core/shared/utils");
    let platform_include = wamr_root.join("core/shared/platform/include");

    let mut stick = cc::Build::new();
    if let Some(cc_path) = embedded_c_compiler(&target) {
        stick.compiler(cc_path).no_default_flags(true);
    }
    stick.define("WASM_ENABLE_LIBC_WASI", "1");
    if is_embedded_target(&target) {
        stick.define("STICK_WAMR_HEAP_EXTERNAL", None);
    }
    if target.contains("xtensa") {
        stick.flag("-mlongcalls");
    }
    stick
        .file(manifest_dir.join("platform/stick/stick_libc.c"))
        .file(manifest_dir.join("c/stick_wamr.c"))
        .include(&wamr_include)
        .include(&shared_include)
        .include(&platform_include)
        .include(manifest_dir.join("platform/stick"))
        .compile("stick_wamr");

    let bindings_path = out_dir.join("bindings.rs");
    if is_embedded_target(&target) {
        std::fs::write(
            &bindings_path,
            r#"unsafe extern "C" {
    pub fn stick_wamr_set_runtime_heap(heap_buf: *mut core::ffi::c_void, heap_size: u32);
    pub fn stick_wamr_init_runtime() -> i32;
    pub fn stick_wamr_run(
        wasm_bytes: *const u8,
        wasm_len: u32,
        argv: *mut *mut ::core::ffi::c_char,
        argc: u32,
        env: *const *const ::core::ffi::c_char,
        env_count: u32,
        preopen_dir: *const ::core::ffi::c_char,
        err: *mut ::core::ffi::c_char,
        err_len: u32,
    ) -> i32;
    pub fn stick_wamr_terminate();
    pub fn stick_wamr_capture_ptr() -> *const ::core::ffi::c_char;
}
"#,
        )
        .expect("write embedded bindings");
    } else {
        let bindings = bindgen::Builder::default()
            .use_core()
            .ctypes_prefix("::core::ffi")
            .header(manifest_dir.join("c/stick_wamr.h").to_string_lossy())
            .allowlist_function("stick_wamr_.*")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("bind stick_wamr.h");

        bindings
            .write_to_file(&bindings_path)
            .expect("write bindings");
    }
}
