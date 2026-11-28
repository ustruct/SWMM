use std::path::PathBuf;

#[cfg(windows)]
mod win {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

    const TH32CS_SNAPPROCESS: u32 = 0x2;
    const INVALID_HANDLE_VALUE: isize = -1;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> isize;
        fn QueryFullProcessImageNameW(process: isize, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetCurrentProcess() -> isize;
        fn SetProcessWorkingSetSize(process: isize, minimum: usize, maximum: usize) -> i32;
    }

    fn image_path(process_id: u32) -> Option<PathBuf> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
            if handle == 0 {
                return None;
            }
            let mut buffer = vec![0u16; 32_768];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..size as usize])))
        }
    }

    pub fn find(matches_name: impl Fn(&str) -> bool) -> Vec<PathBuf> {
        let mut found = Vec::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return found;
            }
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                let len = entry.exe_file.iter().position(|&c| c == 0).unwrap_or(entry.exe_file.len());
                let name = String::from_utf16_lossy(&entry.exe_file[..len]);
                if matches_name(&name) {
                    if let Some(path) = image_path(entry.process_id) {
                        found.push(path);
                    }
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        found
    }

    pub fn trim_memory() {
        unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    }
}

#[cfg(windows)]
pub fn find_by_name(names: &[&str]) -> Vec<PathBuf> {
    win::find(|name| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

#[cfg(not(windows))]
pub fn find_by_name(_names: &[&str]) -> Vec<PathBuf> {
    Vec::new()
}

pub fn trim_memory() {
    #[cfg(windows)]
    win::trim_memory();
}
