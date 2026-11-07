use std::{ffi::c_void, sync::OnceLock};

type Hmodule = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const u8) -> *mut c_void;
}

const PLAYER_INFO_SITE: u64 = 10 * 8;
const MAIN_DATA_SITE: u64 = 12 * 8;
const CPU_SIZE: usize = 1024;
const CPU_SP: usize = 256;
const CPU_V0: usize = 512;

pub(crate) struct Build {
    pub name: &'static str,
    data_offset: u64,
    pub recover_life: u64,
    pub get_life: u64,
    pub get_max_life: u64,
    gdt_got: u64,
    handles: u64,
    get_s32: u64,
    set_s32: u64,
    get_f32: u64,
    set_f32: u64,
    signatures: &'static [(u64, [u32; 8])],
}

const BUILDS: &[Build] = &[
    Build {
        name: "1.8.2",
        data_offset: 0x1a1f000,
        recover_life: 0xbb3ee4,
        get_life: 0xbb3ca4,
        get_max_life: 0xbb3bec,
        gdt_got: 0x1c054a8,
        handles: 0x102fbc0,
        get_s32: 0xc21840,
        set_s32: 0xc21920,
        get_f32: 0xc22340,
        set_f32: 0xc22390,
        signatures: &[
            (0xbb3ee4, [0xa9be7bfd, 0xa9014ff4, 0x910003fd, 0xaa0003f4, 0xf9403000, 0xb4000160, 0xf9400008, 0xf9407908]),
            (0xbb3ca4, [0xf9403000, 0xb4000120, 0xa9bf7bfd, 0x910003fd, 0xf9400008, 0xf9415d08, 0xd63f0100, 0xb4000080]),
            (0xbb3bec, [0xf9403000, 0xb4000080, 0xf9400008, 0xf9407901, 0xd61f0020, 0xd65f03c0, 0xa9be7bfd, 0xa9014ff4]),
            (0x102fbc0, [0x91002000, 0x17ffba46, 0x91006000, 0x17ffba44, 0x9100a000, 0x17ffba42, 0x9100e000, 0x17ffba40]),
            (0xc21840, [0xd10083ff, 0xa9017bfd, 0x910043fd, 0xb81fc3bf, 0x90007f28, 0x2a0003e2, 0xf9425508, 0xf9400100]),
            (0xc21920, [0xd10103ff, 0xa9017bfd, 0xf90013f5, 0xa9034ff4, 0x910043fd, 0x90007f28, 0xf9425508, 0xf9400115]),
            (0xc22340, [0xd10083ff, 0xa9017bfd, 0x910043fd, 0xb81fc3bf, 0xf0007f08, 0x2a0003e2, 0xf9425508, 0xf9400100]),
            (0xc22390, [0xd10103ff, 0xfd000be8, 0xa9027bfd, 0xa9034ff4, 0x910083fd, 0xf0007f08, 0xf9425508, 0xf9400114]),
        ],
    },
    Build {
        name: "1.9.0",
        data_offset: 0x1b7a000,
        recover_life: 0xcc73c4,
        get_life: 0xcc7184,
        get_max_life: 0xcc70cc,
        gdt_got: 0x1d66478,
        handles: 0x1143950,
        get_s32: 0xd34ae0,
        set_s32: 0xd34bc0,
        get_f32: 0xd355e0,
        set_f32: 0xd35630,
        signatures: &[
            (0xcc73c4, [0xa9be7bfd, 0xa9014ff4, 0x910003fd, 0xaa0003f4, 0xf9403000, 0xb4000160, 0xf9400008, 0xf9407908]),
            (0xcc7184, [0xf9403000, 0xb4000120, 0xa9bf7bfd, 0x910003fd, 0xf9400008, 0xf9415d08, 0xd63f0100, 0xb4000080]),
            (0xcc70cc, [0xf9403000, 0xb4000080, 0xf9400008, 0xf9407901, 0xd61f0020, 0xd65f03c0, 0xa9be7bfd, 0xa9014ff4]),
            (0x1143950, [0x91002000, 0x17ffba46, 0x91006000, 0x17ffba44, 0x9100a000, 0x17ffba42, 0x9100e000, 0x17ffba40]),
            (0xd34ae0, [0xd10083ff, 0xa9017bfd, 0x910043fd, 0xb81fc3bf, 0xd0008188, 0x2a0003e2, 0xf9423d08, 0xf9400100]),
            (0xd34bc0, [0xd10103ff, 0xa9017bfd, 0xf90013f5, 0xa9034ff4, 0x910043fd, 0xd0008188, 0xf9423d08, 0xf9400115]),
            (0xd355e0, [0xd10083ff, 0xa9017bfd, 0x910043fd, 0xb81fc3bf, 0xb0008188, 0x2a0003e2, 0xf9423d08, 0xf9400100]),
            (0xd35630, [0xd10103ff, 0xfd000be8, 0xa9027bfd, 0xa9034ff4, 0x910083fd, 0xb0008188, 0xf9423d08, 0xf9400114]),
        ],
    },];

type CurrentCpu = unsafe extern "C" fn() -> *mut u8;
type GuestCall = unsafe extern "C" fn(*mut u8, u64);

pub(crate) struct Game {
    pub base: u64,
    pub build: &'static Build,
    current_cpu: CurrentCpu,
    guest_call: GuestCall,
}

pub(crate) enum Detect {
    Ready(Game),
    Unsupported,
}

static GAME: OnceLock<Detect> = OnceLock::new();

pub(crate) unsafe fn rd<T: Copy>(address: u64) -> T {
    std::ptr::read_unaligned(address as *const T)
}

pub(crate) unsafe fn wr<T: Copy>(address: u64, value: T) {
    std::ptr::write_unaligned(address as *mut T, value)
}

fn export(name: &[u8]) -> usize {
    unsafe { GetProcAddress(GetModuleHandleW(std::ptr::null()), name.as_ptr()) as usize }
}

fn sites() -> u64 {
    export(b"g_site\0") as u64
}

fn detect() -> Option<Detect> {
    let sites = sites();
    let current_cpu = export(b"kernel_current_cpu\0");
    let guest_call = export(b"guest_call\0");
    if sites == 0 || current_cpu == 0 || guest_call == 0 {
        return Some(Detect::Unsupported);
    }
    let main_data: u64 = unsafe { rd(sites + MAIN_DATA_SITE) };
    if main_data == 0 {
        return None;
    }
    for build in BUILDS {
        let base = main_data.wrapping_sub(build.data_offset);
        let matches = build.signatures.iter().all(|(offset, words)| {
            words.iter().enumerate().all(|(i, w)| unsafe { rd::<u32>(base + offset + i as u64 * 4) } == *w)
        });
        if matches {
            return Some(Detect::Ready(Game {
                base,
                build,
                current_cpu: unsafe { std::mem::transmute::<usize, CurrentCpu>(current_cpu) },
                guest_call: unsafe { std::mem::transmute::<usize, GuestCall>(guest_call) },
            }));
        }
    }
    Some(Detect::Unsupported)
}

pub(crate) fn game() -> Option<&'static Detect> {
    if let Some(found) = GAME.get() {
        return Some(found);
    }
    let found = detect()?;
    Some(GAME.get_or_init(|| found))
}

pub(crate) fn player_info() -> Option<u64> {
    let sites = sites();
    if sites == 0 {
        return None;
    }
    unsafe {
        let site: u64 = rd(sites + PLAYER_INFO_SITE);
        if site < 0x10000 {
            return None;
        }
        let info: u64 = rd(site);
        (info >= 0x10000 && rd::<u64>(info + 0x60) >= 0x10000).then_some(info)
    }
}

impl Game {
    unsafe fn call_with(&self, function: u64, args: &[u64], float: Option<f32>) -> u64 {
        let cpu = (self.current_cpu)();
        if cpu.is_null() {
            return 0;
        }
        let mut saved = [0u8; CPU_SIZE];
        std::ptr::copy_nonoverlapping(cpu, saved.as_mut_ptr(), CPU_SIZE);
        let x = cpu as *mut u64;
        for (i, a) in args.iter().enumerate() {
            *x.add(i) = *a;
        }
        std::ptr::write_bytes(cpu.add(CPU_V0), 0, 16);
        if let Some(value) = float {
            std::ptr::write_unaligned(cpu.add(CPU_V0) as *mut f32, value);
        }
        let sp = cpu.add(CPU_SP) as *mut u64;
        *sp = (*sp - 0x800) & !0xf;
        (self.guest_call)(cpu, self.base + function);
        let result = *x;
        std::ptr::copy_nonoverlapping(saved.as_ptr(), cpu, CPU_SIZE);
        result
    }

    pub unsafe fn call(&self, function: u64, args: &[u64]) -> u64 {
        self.call_with(function, args, None)
    }

    unsafe fn call_returning_float(&self, function: u64, args: &[u64]) -> f32 {
        let cpu = (self.current_cpu)();
        if cpu.is_null() {
            return f32::NAN;
        }
        let mut saved = [0u8; CPU_SIZE];
        std::ptr::copy_nonoverlapping(cpu, saved.as_mut_ptr(), CPU_SIZE);
        let x = cpu as *mut u64;
        for (i, a) in args.iter().enumerate() {
            *x.add(i) = *a;
        }
        std::ptr::write_bytes(cpu.add(CPU_V0), 0, 16);
        let sp = cpu.add(CPU_SP) as *mut u64;
        *sp = (*sp - 0x800) & !0xf;
        (self.guest_call)(cpu, self.base + function);
        let value = std::ptr::read_unaligned(cpu.add(CPU_V0) as *const f32);
        std::ptr::copy_nonoverlapping(saved.as_ptr(), cpu, CPU_SIZE);
        value
    }

    unsafe fn gdt(&self) -> Option<u64> {
        let var: u64 = rd(self.base + self.build.gdt_got);
        if var < 0x10000 {
            return None;
        }
        let mgr: u64 = rd(var);
        (mgr >= 0x10000).then_some(mgr)
    }

    unsafe fn handle(&self, kind: u64, name: &str) -> Option<u32> {
        let mgr = self.gdt()?;
        let trigger_var: u64 = rd(mgr + 0xbd8);
        if trigger_var < 0x10000 {
            return None;
        }
        let trigger: u64 = rd(trigger_var);
        if trigger < 0x10000 {
            return None;
        }
        let hash = crc32(name.as_bytes());
        let index = self.call(self.build.handles + kind * 8, &[trigger, hash as u64]) as u32;
        if index == u32::MAX {
            return None;
        }
        let buffer = rd::<u32>(mgr + 0xc28);
        Some(index | (buffer << 24))
    }

    pub unsafe fn get_s32(&self, name: &str) -> Option<i32> {
        let h = self.handle(1, name)?;
        Some(self.call(self.build.get_s32, &[h as u64, 0]) as u32 as i32)
    }

    pub unsafe fn set_s32(&self, name: &str, value: i32) -> bool {
        let Some(h) = self.handle(1, name) else { return false };
        self.call(self.build.set_s32, &[value as u32 as u64, h as u64, 0]);
        true
    }

    pub unsafe fn get_f32(&self, name: &str) -> Option<f32> {
        let h = self.handle(2, name)?;
        Some(self.call_returning_float(self.build.get_f32, &[h as u64, 0]))
    }

    pub unsafe fn set_f32(&self, name: &str, value: f32) -> bool {
        let Some(h) = self.handle(2, name) else { return false };
        self.call_with(self.build.set_f32, &[h as u64, 0], Some(value));
        true
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
