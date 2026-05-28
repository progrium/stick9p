fn main() {
    let mut heap = vec![0u8; wamr_sys::RUNTIME_HEAP_BYTES];
    wamr_sys::set_runtime_heap(heap.as_mut_ptr(), heap.len());

    let wasm = include_bytes!("../../wasm/zigcheck.wasm");
    let argv = &["zigcheck", "m5stick"];
    let env = &[
        "PATH=/",
        "HOME=/",
        "USER=stick",
        "ZIG_TARGET=m5stick",
    ];
    let mut err = [0u8; 256];
    match wamr_sys::run(wasm, argv, env, &mut err) {
        Ok(out) => print!("{out}"),
        Err(()) => {
            let msg = std::str::from_utf8(&err)
                .unwrap_or("wasm failed")
                .trim_end_matches('\0');
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
    }
}
