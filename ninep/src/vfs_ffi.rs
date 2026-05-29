//! C ABI backing WAMR WASI file hooks — same namespace as the 9P static tree + `/tmp` memfs.

use core::cell::RefCell;
use core::ffi::{CStr, c_char};

use critical_section::Mutex;
use crate::fs::{self, Node, TaskFileKind};
use crate::readme::TEXT;
use devices::memfs;

const ID_MEM_BASE: u32 = 0x0001_0000;

static WASM_FS: Mutex<RefCell<Option<fs::FsContext<'static>>>> =
    Mutex::new(RefCell::new(None));

/// Install the same [`FsContext`] hooks the 9P server uses (call once from firmware boot).
pub fn set_fs_context(ctx: fs::FsContext<'static>) {
    critical_section::with(|cs| {
        *WASM_FS.borrow(cs).borrow_mut() = Some(ctx);
    });
}

fn wasm_ctx() -> fs::FsContext<'static> {
    critical_section::with(|cs| {
        let ctx = *WASM_FS.borrow(cs).borrow();
        ctx.unwrap_or_else(devices_wasm_fs_fallback)
    })
}

fn devices_wasm_fs_fallback() -> fs::FsContext<'static> {
    fs::FsContext {
        board_name: "stick",
        version: "wasm",
        uptime_ms: || embassy_time::Instant::now().as_millis(),
        sys_mac_line: || heapless::String::new(),
        sys_chip_line: || heapless::String::new(),
        sys_heap_line: || heapless::String::new(),
        sys_tmpfs_line: heapless::String::new,
        led_state_line: devices::led::state_line,
        on_led_ctl: |s| devices::led::handle_ctl(s).map_err(|_| "bad ctl"),
        request_reboot: || {},
        read_display_fb: devices::display::read_fb,
        write_display_fb: devices::display::write_fb,
        on_display_ctl: devices::display::handle_ctl,
        on_display_brightness: devices::display::handle_brightness,
        read_imu_accel: devices::imu::read_accel,
        read_imu_gyro: devices::imu::read_gyro,
        on_imu_ctl: devices::imu::handle_ctl,
        read_btn_a: devices::buttons::read_a,
        read_btn_b: devices::buttons::read_b,
        read_btn_event: devices::buttons::try_read_event,
        on_buttons_ctl: devices::buttons::handle_ctl,
        read_power_battery: |off, buf| copy_line(off, buf, &devices::power::battery_line()),
        read_power_vbat: |off, buf| copy_line(off, buf, &devices::power::vbat_line()),
        on_power_ctl: devices::power::handle_ctl,
        on_buzzer_ctl: devices::buzzer::handle_ctl,
        read_mic_pcm: devices::mic::try_read_pcm,
        on_mic_ctl: devices::mic::handle_ctl,
        write_spk_pcm: devices::spk::write_pcm,
        on_spk_ctl: devices::spk::handle_ctl,
        on_i2c1_ctl: |_| Err("i2c: bus not installed"),
        on_i2c1_data: |_| Err("i2c: bus not installed"),
        read_i2c1_scan: devices::i2c1::read_scan,
        on_gpio_ctl: gpio_ctl,
        on_gpio_level: gpio_level,
        refresh_gpio_level: |_| {},
        on_root_ctl: |_| Err("wasm: call ninep::vfs_ffi::set_fs_context"),
    }
}

fn copy_line(off: u64, buf: &mut [u8], line: &str) -> usize {
    if off >= line.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (line.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&line.as_bytes()[start..start + n]);
    n
}

fn gpio_ctl(pin: u8, s: &str) -> Result<(), &'static str> {
    let mode = devices::gpio::parse_mode(s)?;
    devices::gpio::set_mode(pin, mode);
    Ok(())
}

fn gpio_level(pin: u8, s: &str) -> Result<(), &'static str> {
    match s.trim() {
        "0" => {
            devices::gpio::set_out_level(pin, false);
            Ok(())
        }
        "1" => {
            devices::gpio::set_out_level(pin, true);
            Ok(())
        }
        _ => Err("gpio: expected 0 or 1"),
    }
}

fn id_from_node(node: Node) -> u32 {
    node.path() as u32
}

fn id_mem(ino: u16) -> u32 {
    ID_MEM_BASE | ino as u32
}

fn is_mem_id(id: u32) -> bool {
    (id & ID_MEM_BASE) == ID_MEM_BASE
}

fn mem_ino(id: u32) -> u16 {
    (id & 0xFFFF) as u16
}

fn mem_root_id() -> u32 {
    id_from_node(Node::MemRoot)
}

fn resolve_preopen(path: &str) -> Option<u32> {
    let p = path.trim();
    if p.is_empty() || p == "." || p == "/" {
        return Some(id_from_node(Node::Root));
    }
    resolve_at(id_from_node(Node::Root), p)
}

fn resolve_at(base: u32, path: &str) -> Option<u32> {
    let mut cur = base;
    let p = path.trim();
    if p.is_empty() {
        return Some(cur);
    }
    let rest = p.strip_prefix('/').unwrap_or(p);
    if rest.is_empty() {
        return Some(cur);
    }
    for part in rest.split('/').filter(|s| !s.is_empty()) {
        cur = walk_id(cur, part)?;
    }
    Some(cur)
}

fn walk_id(id: u32, name: &str) -> Option<u32> {
    if name == "." {
        return Some(id);
    }
    if name == ".." {
        return parent_id(id);
    }
    if id == mem_root_id() || (is_mem_id(id) && memfs::is_dir(mem_ino(id))) {
        let parent = if id == mem_root_id() {
            memfs::ROOT_INO
        } else {
            mem_ino(id)
        };
        let child = memfs::walk(parent, name)?;
        return Some(id_mem(child));
    }
    let node = node_from_id(id)?;
    let next = node.walk(name)?;
    if next == Node::MemRoot {
        return Some(mem_root_id());
    }
    Some(id_from_node(next))
}

fn parent_id(id: u32) -> Option<u32> {
    if id == id_from_node(Node::Root) {
        return Some(id);
    }
    if is_mem_id(id) {
        let ino = mem_ino(id);
        if ino == memfs::ROOT_INO {
            return Some(mem_root_id());
        }
        let parent = memfs::walk(ino, "..")?;
        if parent == memfs::ROOT_INO {
            return Some(mem_root_id());
        }
        return Some(id_mem(parent));
    }
    let node = node_from_id(id)?;
    node.walk("..").map(id_from_node)
}

fn node_from_id(id: u32) -> Option<Node> {
    if is_mem_id(id) {
        return None;
    }
    Node::from_path(id as u64)
}

fn is_dir_id(id: u32) -> bool {
    if id == mem_root_id() {
        return true;
    }
    if is_mem_id(id) {
        return memfs::is_dir(mem_ino(id));
    }
    node_from_id(id).map(|n| n.is_dir()).unwrap_or(false)
}

fn name_id(id: u32, buf: &mut [u8]) -> Option<usize> {
    if is_mem_id(id) {
        let name = memfs::name(mem_ino(id));
        return copy_name(name.as_str(), buf);
    }
    let node = node_from_id(id)?;
    copy_name(node.name(), buf)
}

fn copy_name(name: &str, buf: &mut [u8]) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    let n = name.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&name.as_bytes()[..n]);
    buf[n] = 0;
    Some(n)
}

fn child_at_id(id: u32, index: u32) -> Option<u32> {
    if id == mem_root_id() {
        let ino = memfs::child_ino_at(memfs::ROOT_INO, index as usize)?;
        return Some(id_mem(ino));
    }
    if is_mem_id(id) && memfs::is_dir(mem_ino(id)) {
        let ino = memfs::child_ino_at(mem_ino(id), index as usize)?;
        return Some(id_mem(ino));
    }
    if id == id_from_node(Node::Task) {
        if index == 0 {
            return Some(id_from_node(Node::TaskAlloc));
        }
        let mut rids = heapless::Vec::<u8, 32>::new();
        devices::task::list_rids(&mut rids);
        let rid = *rids.get(index as usize - 1)?;
        return Some(id_from_node(Node::TaskDir(rid)));
    }
    if let Some(node) = node_from_id(id) {
        if let Node::TaskDir(rid) = node {
            let kind = TaskFileKind::from_index(index as u64)?;
            return Some(id_from_node(Node::TaskFile(rid, kind)));
        }
        if !node.uses_custom_dir_list() {
            return node.children().get(index as usize).map(|&n| id_from_node(n));
        }
    }
    None
}

fn length_id(id: u32) -> u64 {
    if is_mem_id(id) {
        return memfs::length(mem_ino(id));
    }
    node_from_id(id)
        .map(|n| n.length())
        .unwrap_or(0)
}

fn read_id(id: u32, off: u64, buf: &mut [u8]) -> usize {
    if is_mem_id(id) {
        return memfs::read(mem_ino(id), off, buf);
    }
    let Some(node) = node_from_id(id) else {
        return 0;
    };
    if node == Node::Readme {
        return copy_bytes(TEXT.as_bytes(), off, buf);
    }
    let ctx = wasm_ctx();
    fs::read_file(node, &ctx, off, buf)
}

fn write_id(id: u32, off: u64, data: &[u8]) -> Result<usize, ()> {
    if is_mem_id(id) {
        return memfs::write(mem_ino(id), off, data).map_err(|_| ());
    }
    let Some(node) = node_from_id(id) else {
        return Err(());
    };
    if let Node::TaskFile(rid, TaskFileKind::Data) = node {
        return devices::task::write_data(rid, off, data).map_err(|_| ());
    }
    if !fs::is_writable(node) {
        return Err(());
    }
    let ctx = wasm_ctx();
    fs::write_file(node, &ctx, off, data).map_err(|_| ())
}

fn copy_bytes(src: &[u8], off: u64, buf: &mut [u8]) -> usize {
    if off >= src.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (src.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&src[start..start + n]);
    n
}

/// Monotonic boot time for WAMR (`os_time_get_boot_us` on core 1).
#[unsafe(no_mangle)]
pub extern "C" fn stick_time_boot_us() -> u64 {
    embassy_time::Instant::now().as_micros()
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_ready() -> bool {
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_preopen_ino(path: *const u8) -> i32 {
    if path.is_null() {
        return id_from_node(Node::Root) as i32;
    }
    let Ok(s) = unsafe { CStr::from_ptr(path as *const c_char) }.to_str() else {
        return -1;
    };
    resolve_preopen(s).map(|id| id as i32).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_walk(parent: u32, name: *const u8) -> i32 {
    if name.is_null() {
        return -1;
    }
    let Ok(s) = unsafe { CStr::from_ptr(name as *const c_char) }.to_str() else {
        return -1;
    };
    walk_id(parent, s).map(|id| id as i32).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_is_dir(id: u32) -> bool {
    is_dir_id(id)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_length(id: u32) -> u64 {
    length_id(id)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_read(id: u32, off: u64, buf: *mut u8, len: usize) -> isize {
    if buf.is_null() || len == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    read_id(id, off, slice) as isize
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_write(id: u32, off: u64, buf: *const u8, len: usize) -> isize {
    if buf.is_null() || len == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };
    match write_id(id, off, slice) {
        Ok(n) => n as isize,
        Err(()) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_child_at(parent: u32, index: u32) -> i32 {
    child_at_id(parent, index)
        .map(|id| id as i32)
        .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn stick_vfs_name(id: u32, buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, buflen) };
    name_id(id, slice).map(|n| n as i32).unwrap_or(-1)
}
