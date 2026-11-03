#![cfg(windows)]
#![allow(non_snake_case, clippy::missing_safety_doc)]

mod motion;

use std::{
    ffi::{c_void, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

type Hmodule = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(module: Hmodule, buf: *mut u16, len: u32) -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const u8) -> *mut c_void;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, len: u32) -> u32;
}

extern "C" {
    fn _wputenv_s(name: *const u16, value: *const u16) -> i32;
}

#[no_mangle]
#[used]
pub static SWMM_LOADER_MARKER: [u8; 11] = *b"SWMM-Loader";

const OVERLAY_ENV: &str = "SAVAGEWIND_ROMFS_OVERLAY";
const DLL_PROCESS_ATTACH: u32 = 1;

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(s: &str) -> Vec<u16> {
    wide(std::ffi::OsStr::new(s))
}

fn module_path(module: Hmodule) -> Option<PathBuf> {
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(PathBuf::from(OsString::from_wide(&buf[..n])));
        }
        if buf.len() >= 32768 {
            return None;
        }
        buf.resize(buf.len() * 2, 0);
    }
}

fn get_env(name: &str) -> Option<OsString> {
    let name = wide_str(name);
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(OsString::from_wide(&buf[..n]));
        }
        buf.resize(n + 1, 0);
    }
}

fn set_env(name: &str, value: &Path) {
    let name = wide_str(name);
    let value = wide(value.as_os_str());
    unsafe {
        SetEnvironmentVariableW(name.as_ptr(), value.as_ptr());
        _wputenv_s(name.as_ptr(), value.as_ptr());
    }
}

fn ini_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            return None;
        }
        let (k, v) = line.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn is_off(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no")
}

fn log(dir: &Path, message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dir.join("loader.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}

fn append_log(dir: &Path, message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("loader.log")) {
        let _ = writeln!(file, "{message}");
    }
}

fn motion_config(config: Option<&str>) -> Result<motion::Config, String> {
    let setting = get_env("SWMM_MOTION")
        .map(|v| v.to_string_lossy().to_string())
        .or_else(|| config.and_then(|c| ini_value(c, "motion")))
        .unwrap_or_else(|| "off".into());
    let mode = match setting.to_ascii_lowercase().as_str() {
        "auto" | "on" | "1" | "true" | "yes" => motion::Mode::Auto,
        "prefer" => motion::Mode::Prefer,
        _ => return Err("Cemuhook motion is off".into()),
    };
    let server = config
        .and_then(|c| ini_value(c, "motion_server"))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "127.0.0.1:26760".into());
    let slot = config
        .and_then(|c| ini_value(c, "motion_slot"))
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 3);
    Ok(motion::Config { mode, server, slot })
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some(text.trim_start_matches('\u{feff}').to_string())
}

fn config_dir() -> Option<PathBuf> {
    get_env("APPDATA").map(|d| PathBuf::from(d).join("SWMM"))
}

struct Input {
    motion: Option<motion::Config>,
    camera: f32,
}

fn camera_config(config: Option<&str>) -> f32 {
    config
        .and_then(|c| ini_value(c, "camera_sensitivity"))
        .and_then(|v| v.trim_end_matches('%').trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 10.0 && *v <= 500.0)
        .map(|v| v / 100.0)
        .unwrap_or(1.0)
}

fn attach() -> Option<(PathBuf, String, Input)> {
    let exe = module_path(std::ptr::null_mut())?;
    let game_dir = exe.parent()?.to_path_buf();
    let exe_name = exe.file_name()?.to_string_lossy().to_ascii_lowercase();
    if !exe_name.starts_with("savagewind") {
        return None;
    }
    let config_dir_path = config_dir()?;
    let loader_config = read_text(&config_dir_path.join("loader.ini"));
    let message = (|| {
        if get_env("SWMM_LAUNCHED").is_some() {
            return "Started by SWMM".to_string();
        }
        if get_env("SWMM_MODS").map(|v| is_off(&v.to_string_lossy())).unwrap_or(false) {
            return "SWMM_MODS=0, mods off".to_string();
        }
        if let Some(ini) = read_text(&game_dir.join("savagewind.ini")) {
            if ini_value(&ini, "swmm_mods").map(|v| is_off(&v)).unwrap_or(false) {
                return "swmm_mods = 0 in savagewind.ini, mods off".to_string();
            }
        }
        let Some(config) = loader_config.as_deref() else {
            return "No loader.ini, mods off".to_string();
        };
        if ini_value(config, "enabled").map(|v| is_off(&v)).unwrap_or(false) {
            return "Mods are turned off in SWMM".to_string();
        }
        let Some(overlay) = ini_value(config, "overlay").filter(|v| !v.is_empty()).map(PathBuf::from) else {
            return "No overlay folder, mods off".to_string();
        };
        if !overlay.is_dir() {
            return format!("{} doesn't exist, mods off", overlay.display());
        }
        let previous = get_env(OVERLAY_ENV)
            .filter(|v| !v.is_empty() && PathBuf::from(v) != overlay)
            .map(|v| format!(" (replaced {})", v.to_string_lossy()))
            .unwrap_or_default();
        set_env(OVERLAY_ENV, &overlay);
        format!("{OVERLAY_ENV}={}{previous}", overlay.display())
    })();
    let camera = camera_config(loader_config.as_deref());
    match motion_config(loader_config.as_deref()) {
        Ok(motion) => Some((config_dir_path, message, Input { motion: Some(motion), camera })),
        Err(off) => Some((config_dir_path, format!("{message}\n{off}"), Input { motion: None, camera })),
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(module: Hmodule, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        let _ = module;
        if let Ok(Some((dir, message, input))) = std::panic::catch_unwind(attach) {
            log(&dir, &message);
            if input.motion.is_some() || (input.camera - 1.0).abs() >= 0.001 {
                motion::start(input.motion, input.camera, move |line| append_log(&dir, &line));
            }
        }
    }
    1
}

static REAL_PDH: AtomicUsize = AtomicUsize::new(0);
const PDH_CSTATUS_NO_OBJECT: i32 = 0xC000_0BB8_u32 as i32;

unsafe fn real(name: &[u8]) -> *mut c_void {
    let mut module = REAL_PDH.load(Ordering::Acquire);
    if module == 0 {
        let mut buf = [0u16; 520];
        let n = GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 || n >= buf.len() {
            return std::ptr::null_mut();
        }
        let mut path = PathBuf::from(OsString::from_wide(&buf[..n]));
        path.push("pdh.dll");
        let handle = LoadLibraryW(wide(path.as_os_str()).as_ptr());
        if handle.is_null() {
            return std::ptr::null_mut();
        }
        module = handle as usize;
        REAL_PDH.store(module, Ordering::Release);
    }
    GetProcAddress(module as Hmodule, name.as_ptr())
}

#[no_mangle]
pub unsafe extern "system" fn PdhOpenQueryA(
    data_source: *const u8,
    user_data: usize,
    query: *mut *mut c_void,
) -> i32 {
    let f = real(b"PdhOpenQueryA\0");
    if f.is_null() {
        return PDH_CSTATUS_NO_OBJECT;
    }
    let f: unsafe extern "system" fn(*const u8, usize, *mut *mut c_void) -> i32 = std::mem::transmute(f);
    f(data_source, user_data, query)
}

#[no_mangle]
pub unsafe extern "system" fn PdhAddEnglishCounterA(
    query: *mut c_void,
    counter_path: *const u8,
    user_data: usize,
    counter: *mut *mut c_void,
) -> i32 {
    let f = real(b"PdhAddEnglishCounterA\0");
    if f.is_null() {
        return PDH_CSTATUS_NO_OBJECT;
    }
    let f: unsafe extern "system" fn(*mut c_void, *const u8, usize, *mut *mut c_void) -> i32 =
        std::mem::transmute(f);
    f(query, counter_path, user_data, counter)
}

#[no_mangle]
pub unsafe extern "system" fn PdhCollectQueryData(query: *mut c_void) -> i32 {
    let f = real(b"PdhCollectQueryData\0");
    if f.is_null() {
        return PDH_CSTATUS_NO_OBJECT;
    }
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = std::mem::transmute(f);
    f(query)
}

#[no_mangle]
pub unsafe extern "system" fn PdhGetFormattedCounterArrayA(
    counter: *mut c_void,
    format: u32,
    buffer_size: *mut u32,
    item_count: *mut u32,
    items: *mut c_void,
) -> i32 {
    let f = real(b"PdhGetFormattedCounterArrayA\0");
    if f.is_null() {
        return PDH_CSTATUS_NO_OBJECT;
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut u32, *mut u32, *mut c_void) -> i32 =
        std::mem::transmute(f);
    f(counter, format, buffer_size, item_count, items)
}
