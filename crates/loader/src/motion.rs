use std::{
    ffi::c_void,
    io::ErrorKind,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

type Hmodule = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const u8) -> *mut c_void;
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn GetCurrentProcessId() -> u32;
}

const PAGE_READWRITE: u32 = 0x04;
const EVENT_GAMEPAD_ADDED: u32 = 0x653;
const EVENT_GAMEPAD_REMOVED: u32 = 0x654;
const EVENT_GAMEPAD_SENSOR_UPDATE: u32 = 0x659;
const SENSOR_ACCEL: i32 = 1;
const SENSOR_GYRO: i32 = 2;
const STANDARD_GRAVITY: f32 = 9.80665;
const DEG_TO_RAD: f32 = std::f32::consts::PI / 180.0;
const ALIVE_FOR: Duration = Duration::from_secs(3);
const REQUEST_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Prefer,
}

pub struct Config {
    pub mode: Mode,
    pub server: String,
    pub slot: Option<u8>,
}

type OpenGamepad = unsafe extern "C" fn(u32) -> *mut c_void;
type CloseGamepad = unsafe extern "C" fn(*mut c_void);
type HasSensor = unsafe extern "C" fn(*mut c_void, i32) -> bool;
type SetSensorEnabled = unsafe extern "C" fn(*mut c_void, i32, bool) -> bool;
type SensorDataRate = unsafe extern "C" fn(*mut c_void, i32) -> f32;
type PushEvent = unsafe extern "C" fn(*mut u8) -> bool;
type GamepadAxis = unsafe extern "C" fn(*mut c_void, i32) -> i16;
type CamStick = unsafe extern "C" fn(u64, u64) -> i32;
type CamReverse = unsafe extern "C" fn(i32, i32);
type Flag = unsafe extern "C" fn() -> bool;
type TicksNs = unsafe extern "C" fn() -> u64;

static REAL_OPEN: AtomicUsize = AtomicUsize::new(0);
static REAL_CLOSE: AtomicUsize = AtomicUsize::new(0);
static REAL_HAS: AtomicUsize = AtomicUsize::new(0);
static REAL_ENABLE: AtomicUsize = AtomicUsize::new(0);
static REAL_RATE: AtomicUsize = AtomicUsize::new(0);
static PUSH_EVENT: AtomicUsize = AtomicUsize::new(0);
static TICKS_NS: AtomicUsize = AtomicUsize::new(0);
static GET_AXIS: AtomicUsize = AtomicUsize::new(0);
static REAL_CAM_STICK: AtomicUsize = AtomicUsize::new(0);
static REAL_CAM_REVERSE: AtomicUsize = AtomicUsize::new(0);
static HAS_SENSORS: AtomicUsize = AtomicUsize::new(0);
static GYRO_KEYBOARD: AtomicUsize = AtomicUsize::new(0);
static CAMERA_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000);
static CAM_REVERSE: AtomicU32 = AtomicU32::new(0);

static PAD: AtomicUsize = AtomicUsize::new(0);
static PAD_ID: AtomicU32 = AtomicU32::new(0);
static DECIDED_PAD: AtomicUsize = AtomicUsize::new(0);
static INJECT: AtomicBool = AtomicBool::new(false);
static REAL_SENSORS: AtomicBool = AtomicBool::new(false);
static PREFER: AtomicBool = AtomicBool::new(false);
static LAST_DATA_MS: AtomicU64 = AtomicU64::new(0);
static RATE_BITS: AtomicU32 = AtomicU32::new(0);
static START: OnceLock<Instant> = OnceLock::new();

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn server_alive() -> bool {
    let last = LAST_DATA_MS.load(Ordering::Acquire);
    last != 0 && now_ms().saturating_sub(last) < ALIVE_FOR.as_millis() as u64
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn real<T: Copy>(slot: &AtomicUsize) -> Option<T> {
    let f = slot.load(Ordering::Acquire);
    (f != 0).then(|| std::mem::transmute_copy::<usize, T>(&f))
}

unsafe fn decide(pad: *mut c_void) -> bool {
    if pad.is_null() {
        return false;
    }
    if DECIDED_PAD.load(Ordering::Acquire) == pad as usize {
        return INJECT.load(Ordering::Acquire);
    }
    let has = real::<HasSensor>(&REAL_HAS);
    let real_both = has.map(|f| f(pad, SENSOR_GYRO) && f(pad, SENSOR_ACCEL)).unwrap_or(false);
    let inject = server_alive() && (PREFER.load(Ordering::Acquire) || !real_both);
    REAL_SENSORS.store(real_both, Ordering::Release);
    INJECT.store(inject, Ordering::Release);
    DECIDED_PAD.store(pad as usize, Ordering::Release);
    inject
}

unsafe extern "C" fn hook_open(id: u32) -> *mut c_void {
    let Some(open) = real::<OpenGamepad>(&REAL_OPEN) else {
        return std::ptr::null_mut();
    };
    let pad = open(id);
    if !pad.is_null() {
        PAD.store(pad as usize, Ordering::Release);
        PAD_ID.store(id, Ordering::Release);
        DECIDED_PAD.store(0, Ordering::Release);
        INJECT.store(false, Ordering::Release);
    }
    pad
}

unsafe extern "C" fn hook_close(pad: *mut c_void) {
    if PAD.load(Ordering::Acquire) == pad as usize {
        PAD.store(0, Ordering::Release);
        PAD_ID.store(0, Ordering::Release);
        DECIDED_PAD.store(0, Ordering::Release);
        INJECT.store(false, Ordering::Release);
    }
    if let Some(close) = real::<CloseGamepad>(&REAL_CLOSE) {
        close(pad);
    }
}

unsafe extern "C" fn hook_has(pad: *mut c_void, sensor: i32) -> bool {
    if (sensor == SENSOR_ACCEL || sensor == SENSOR_GYRO) && decide(pad) {
        return true;
    }
    real::<HasSensor>(&REAL_HAS).map(|f| f(pad, sensor)).unwrap_or(false)
}

unsafe extern "C" fn hook_enable(pad: *mut c_void, sensor: i32, enabled: bool) -> bool {
    if (sensor == SENSOR_ACCEL || sensor == SENSOR_GYRO) && decide(pad) {
        if REAL_SENSORS.load(Ordering::Acquire) {
            if let Some(f) = real::<SetSensorEnabled>(&REAL_ENABLE) {
                f(pad, sensor, false);
            }
        }
        return true;
    }
    real::<SetSensorEnabled>(&REAL_ENABLE).map(|f| f(pad, sensor, enabled)).unwrap_or(false)
}

unsafe extern "C" fn hook_rate(pad: *mut c_void, sensor: i32) -> f32 {
    if (sensor == SENSOR_ACCEL || sensor == SENSOR_GYRO) && decide(pad) {
        let rate = f32::from_bits(RATE_BITS.load(Ordering::Acquire));
        return if rate > 1.0 { rate } else { 100.0 };
    }
    real::<SensorDataRate>(&REAL_RATE).map(|f| f(pad, sensor)).unwrap_or(0.0)
}

const STICK_DEAD_ZONE: f32 = 0.15;

fn camera_scale() -> f32 {
    f32::from_bits(CAMERA_BITS.load(Ordering::Acquire))
}

unsafe fn stick_for_camera() -> Option<(f32, f32)> {
    if let (Some(sensors), Some(keyboard)) = (real::<Flag>(&HAS_SENSORS), real::<Flag>(&GYRO_KEYBOARD)) {
        if !sensors() && keyboard() {
            return None;
        }
    }
    let pad = PAD.load(Ordering::Acquire);
    let axis = real::<GamepadAxis>(&GET_AXIS)?;
    if pad == 0 {
        return None;
    }
    let x = axis(pad as *mut c_void, 2) as f32 / 32767.0;
    let y = -(axis(pad as *mut c_void, 3) as f32) / 32767.0;
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= STICK_DEAD_ZONE {
        return None;
    }
    let scaled = ((magnitude - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE)).min(1.0) / magnitude;
    Some((x * scaled, y * scaled))
}

unsafe extern "C" fn hook_cam_stick(out: u64, lr: u64) -> i32 {
    let used = real::<CamStick>(&REAL_CAM_STICK).map(|f| f(out, lr)).unwrap_or(0);
    let scale = camera_scale();
    if (scale - 1.0).abs() < 0.001 || out == 0 {
        return used;
    }
    let v = out as *mut f32;
    if used != 0 {
        *v *= scale;
        *v.add(1) *= scale;
        return used;
    }
    let Some((x, y)) = stick_for_camera() else {
        return 0;
    };
    let reverse = CAM_REVERSE.load(Ordering::Acquire);
    *v = if reverse & 1 != 0 { -x } else { x } * scale;
    *v.add(1) = if reverse & 2 != 0 { -y } else { y } * scale;
    1
}

unsafe extern "C" fn hook_cam_reverse(axis: i32, on: i32) {
    if (0..2).contains(&axis) {
        if on != 0 {
            CAM_REVERSE.fetch_or(1 << axis, Ordering::AcqRel);
        } else {
            CAM_REVERSE.fetch_and(!(1 << axis), Ordering::AcqRel);
        }
    }
    if let Some(f) = real::<CamReverse>(&REAL_CAM_REVERSE) {
        f(axis, on);
    }
}

unsafe fn cstr(p: *const u8) -> &'static [u8] {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    std::slice::from_raw_parts(p, n)
}

unsafe fn rd32(p: *const u8, offset: usize) -> u32 {
    std::ptr::read_unaligned(p.add(offset) as *const u32)
}

unsafe fn write_slot(slot: *mut usize, value: usize) -> bool {
    let mut old = 0u32;
    if VirtualProtect(slot as *mut c_void, 8, PAGE_READWRITE, &mut old) == 0 {
        return false;
    }
    std::ptr::write_volatile(slot, value);
    let mut ignored = 0u32;
    VirtualProtect(slot as *mut c_void, 8, old, &mut ignored);
    true
}

struct Hook {
    name: &'static [u8],
    target: usize,
    original: &'static AtomicUsize,
}

unsafe fn patch_imports(base: *mut u8, from: &[u8], source: Hmodule, hooks: &[Hook]) -> usize {
    if base.is_null() || std::ptr::read_unaligned(base as *const u16) != 0x5a4d {
        return 0;
    }
    let nt = base.add(rd32(base, 0x3c) as usize);
    if rd32(nt, 0) != 0x4550 {
        return 0;
    }
    let optional = nt.add(24);
    if std::ptr::read_unaligned(optional as *const u16) != 0x20b {
        return 0;
    }
    let imports = rd32(optional, 112 + 8) as usize;
    if imports == 0 {
        return 0;
    }
    let mut patched = 0;
    let mut desc = base.add(imports);
    loop {
        let lookup = rd32(desc, 0) as usize;
        let name = rd32(desc, 12) as usize;
        let thunks = rd32(desc, 16) as usize;
        if lookup == 0 && name == 0 && thunks == 0 {
            break;
        }
        let dll = if name != 0 { cstr(base.add(name)) } else { b"" };
        if dll.len() >= from.len() && dll[..from.len()].eq_ignore_ascii_case(from) {
            let mut i = 0;
            loop {
                let slot = base.add(thunks + i * 8) as *mut usize;
                let current = std::ptr::read_volatile(slot);
                if current == 0 {
                    break;
                }
                let by_name = if lookup != 0 {
                    let entry = std::ptr::read_unaligned(base.add(lookup + i * 8) as *const u64);
                    (entry & (1 << 63) == 0).then(|| cstr(base.add((entry & 0x7fff_ffff) as usize + 2)))
                } else {
                    None
                };
                for hook in hooks {
                    let matches = match by_name {
                        Some(n) => n == &hook.name[..hook.name.len() - 1],
                        None => GetProcAddress(source, hook.name.as_ptr()) as usize == current,
                    };
                    if matches && current != hook.target {
                        hook.original.store(current, Ordering::Release);
                        if write_slot(slot, hook.target) {
                            patched += 1;
                        }
                    }
                }
                i += 1;
            }
        }
        desc = desc.add(20);
    }
    patched
}

unsafe fn install_hooks() -> Result<usize, &'static str> {
    let sdl_name = wide("SDL3.dll");
    let mut sdl = std::ptr::null_mut();
    for _ in 0..500 {
        sdl = GetModuleHandleW(sdl_name.as_ptr());
        if !sdl.is_null() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if sdl.is_null() {
        return Err("SDL3.dll isn't loaded");
    }
    PUSH_EVENT.store(GetProcAddress(sdl, b"SDL_PushEvent\0".as_ptr()) as usize, Ordering::Release);
    TICKS_NS.store(GetProcAddress(sdl, b"SDL_GetTicksNS\0".as_ptr()) as usize, Ordering::Release);
    GET_AXIS.store(GetProcAddress(sdl, b"SDL_GetGamepadAxis\0".as_ptr()) as usize, Ordering::Release);
    let exe = GetModuleHandleW(std::ptr::null());
    HAS_SENSORS.store(GetProcAddress(exe, b"motion_has_sensors\0".as_ptr()) as usize, Ordering::Release);
    GYRO_KEYBOARD.store(GetProcAddress(exe, b"pc_gyro_keyboard_active\0".as_ptr()) as usize, Ordering::Release);
    if PUSH_EVENT.load(Ordering::Acquire) == 0 {
        return Err("SDL_PushEvent wasn't found");
    }
    let hooks = [
        Hook { name: b"SDL_OpenGamepad\0", target: hook_open as *const () as usize, original: &REAL_OPEN },
        Hook { name: b"SDL_CloseGamepad\0", target: hook_close as *const () as usize, original: &REAL_CLOSE },
        Hook { name: b"SDL_GamepadHasSensor\0", target: hook_has as *const () as usize, original: &REAL_HAS },
        Hook { name: b"SDL_SetGamepadSensorEnabled\0", target: hook_enable as *const () as usize, original: &REAL_ENABLE },
        Hook { name: b"SDL_GetGamepadSensorDataRate\0", target: hook_rate as *const () as usize, original: &REAL_RATE },
    ];
    let patched = patch_imports(exe as *mut u8, b"SDL3.dll", sdl, &hooks);
    for hook in &hooks {
        if hook.original.load(Ordering::Acquire) == 0 {
            hook.original
                .store(GetProcAddress(sdl, hook.name.as_ptr()) as usize, Ordering::Release);
        }
    }
    if REAL_OPEN.load(Ordering::Acquire) == 0 || REAL_HAS.load(Ordering::Acquire) == 0 {
        return Err("Savage Wind's controller functions weren't found");
    }
    Ok(patched)
}

unsafe fn push(event: &mut [u8; 128]) {
    if let Some(f) = real::<PushEvent>(&PUSH_EVENT) {
        f(event.as_mut_ptr());
    }
}

fn ticks() -> u64 {
    unsafe { real::<TicksNs>(&TICKS_NS).map(|f| f()).unwrap_or(0) }
}

fn device_event(kind: u32, id: u32) -> [u8; 128] {
    let mut e = [0u8; 128];
    e[0..4].copy_from_slice(&kind.to_le_bytes());
    e[8..16].copy_from_slice(&ticks().to_le_bytes());
    e[16..20].copy_from_slice(&id.to_le_bytes());
    e
}

fn sensor_event(id: u32, sensor: i32, data: [f32; 3], sensor_ns: u64) -> [u8; 128] {
    let mut e = [0u8; 128];
    let now = ticks();
    e[0..4].copy_from_slice(&EVENT_GAMEPAD_SENSOR_UPDATE.to_le_bytes());
    e[8..16].copy_from_slice(&now.to_le_bytes());
    e[16..20].copy_from_slice(&id.to_le_bytes());
    e[20..24].copy_from_slice(&sensor.to_le_bytes());
    for (i, v) in data.iter().enumerate() {
        e[24 + i * 4..28 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let stamp = if sensor_ns != 0 { sensor_ns } else { now };
    e[40..48].copy_from_slice(&stamp.to_le_bytes());
    e
}

fn reconnect_pad() {
    let id = PAD_ID.load(Ordering::Acquire);
    if id == 0 || PAD.load(Ordering::Acquire) == 0 {
        return;
    }
    unsafe {
        push(&mut device_event(EVENT_GAMEPAD_REMOVED, id));
        push(&mut device_event(EVENT_GAMEPAD_ADDED, id));
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for (i, &b) in data.iter().enumerate() {
        crc ^= if (8..12).contains(&i) { 0 } else { b as u32 };
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn request(client: u32, kind: u32, slot: u8) -> [u8; 28] {
    let mut p = [0u8; 28];
    p[0..4].copy_from_slice(b"DSUC");
    p[4..6].copy_from_slice(&1001u16.to_le_bytes());
    p[6..8].copy_from_slice(&12u16.to_le_bytes());
    p[12..16].copy_from_slice(&client.to_le_bytes());
    p[16..20].copy_from_slice(&kind.to_le_bytes());
    if kind == 0x100001 {
        p[20..24].copy_from_slice(&4i32.to_le_bytes());
        p[24..28].copy_from_slice(&[0, 1, 2, 3]);
    } else {
        p[20] = 1;
        p[21] = slot;
    }
    let crc = crc32(&p);
    p[8..12].copy_from_slice(&crc.to_le_bytes());
    p
}

fn le16(p: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([p[o], p[o + 1]])
}

fn le32(p: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

fn lef(p: &[u8], o: usize) -> f32 {
    f32::from_bits(le32(p, o))
}

fn le64(p: &[u8], o: usize) -> u64 {
    le32(p, o) as u64 | (le32(p, o + 4) as u64) << 32
}

struct Packet<'a> {
    kind: u32,
    body: &'a [u8],
}

fn decode(p: &[u8]) -> Option<Packet<'_>> {
    if p.len() < 20 || &p[0..4] != b"DSUS" || le16(p, 4) != 1001 {
        return None;
    }
    let size = 16 + le16(p, 6) as usize;
    if size < 20 || size > p.len() || le32(p, 8) != crc32(&p[..size]) {
        return None;
    }
    let kind = le32(p, 16);
    let expected = match kind {
        0x100001 => 32,
        0x100002 => 100,
        _ => return None,
    };
    (size == expected && p[20] <= 3).then(|| Packet { kind, body: &p[..size] })
}

struct Client {
    slot: Option<u8>,
    fixed_slot: bool,
    mac: [u8; 6],
    last_seq: Option<u32>,
    last_stamp: u64,
    last_packet: Option<Instant>,
    rate: f32,
    was_alive: bool,
}

impl Client {
    fn on_port_info(&mut self, b: &[u8], log: &dyn Fn(String)) {
        let (slot, state, model) = (b[20], b[21], b[22]);
        let mac: [u8; 6] = b[24..30].try_into().unwrap_or_default();
        if self.slot == Some(slot) && (state != 2 || (!self.fixed_slot && mac != self.mac)) {
            if !self.fixed_slot {
                self.slot = None;
            }
            self.last_seq = None;
        }
        if self.slot.is_none() && state == 2 && model == 2 {
            self.slot = Some(slot);
            self.mac = mac;
            self.last_seq = None;
            log(format!("Cemuhook: using slot {slot}"));
        }
    }

    fn on_data(&mut self, b: &[u8]) {
        if self.slot != Some(b[20]) || b[21] != 2 || b[31] != 1 {
            return;
        }
        let seq = le32(b, 32);
        let stamp = le64(b, 68);
        if let Some(last) = self.last_seq {
            let delta = seq.wrapping_sub(last);
            if delta == 0 || delta >= 0x8000_0000 || (stamp != 0 && stamp <= self.last_stamp) {
                return;
            }
        }
        let a = [lef(b, 76), lef(b, 80), lef(b, 84)];
        let w = [lef(b, 88), lef(b, 92), lef(b, 96)];
        if a.iter().chain(w.iter()).any(|v| !v.is_finite() || v.abs() > 10000.0) {
            return;
        }
        if a[0] * a[0] + a[1] * a[1] + a[2] * a[2] < 0.01 {
            return;
        }
        self.last_seq = Some(seq);
        self.last_stamp = stamp;
        let now = Instant::now();
        if let Some(prev) = self.last_packet {
            let dt = now.duration_since(prev).as_secs_f32();
            if dt > 0.0005 && dt < 0.25 {
                let hz = 1.0 / dt;
                self.rate = if self.rate > 0.0 { self.rate * 0.95 + hz * 0.05 } else { hz };
                RATE_BITS.store(self.rate.to_bits(), Ordering::Release);
            }
        }
        self.last_packet = Some(now);
        LAST_DATA_MS.store(now_ms(), Ordering::Release);
        let pad = PAD.load(Ordering::Acquire);
        let id = PAD_ID.load(Ordering::Acquire);
        if pad == 0 || id == 0 || DECIDED_PAD.load(Ordering::Acquire) != pad || !INJECT.load(Ordering::Acquire) {
            return;
        }
        let sensor_ns = stamp.saturating_mul(1000);
        let accel = [-a[0] * STANDARD_GRAVITY, -a[1] * STANDARD_GRAVITY, -a[2] * STANDARD_GRAVITY];
        let gyro = [w[0] * DEG_TO_RAD, -w[1] * DEG_TO_RAD, -w[2] * DEG_TO_RAD];
        unsafe {
            push(&mut sensor_event(id, SENSOR_ACCEL, accel, sensor_ns));
            push(&mut sensor_event(id, SENSOR_GYRO, gyro, sensor_ns));
        }
    }

    fn check_alive(&mut self, log: &dyn Fn(String)) {
        let alive = server_alive();
        if alive == self.was_alive {
            return;
        }
        self.was_alive = alive;
        log(if alive {
            "Cemuhook: motion data is coming in".to_string()
        } else {
            "Cemuhook: motion data stopped".to_string()
        });
        let pad = PAD.load(Ordering::Acquire);
        if pad == 0 || DECIDED_PAD.load(Ordering::Acquire) != pad {
            return;
        }
        let injecting = INJECT.load(Ordering::Acquire);
        let wants = alive && (PREFER.load(Ordering::Acquire) || !REAL_SENSORS.load(Ordering::Acquire));
        if injecting != wants {
            reconnect_pad();
        }
    }
}

fn resolve(server: &str) -> Option<SocketAddr> {
    server.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)
}

struct GameCode {
    wanted: bool,
    hooked: bool,
}

impl GameCode {
    fn tick(&mut self, log: &dyn Fn(String)) {
        if !self.wanted {
            return;
        }
        let patched = unsafe { hook_game_code() };
        let hooked = REAL_CAM_STICK.load(Ordering::Acquire) != 0;
        if patched > 0 && !self.hooked {
            log(format!("Camera sensitivity: {:.0}%", camera_scale() * 100.0));
        }
        self.hooked = hooked;
    }
}

unsafe fn hook_game_code() -> usize {
    let name = wide("swgame.dll");
    let game = GetModuleHandleW(name.as_ptr());
    if game.is_null() {
        return 0;
    }
    let hooks = [
        Hook { name: b"rt_cam_stick\0", target: hook_cam_stick as *const () as usize, original: &REAL_CAM_STICK },
        Hook {
            name: b"rt_cam_reverse\0",
            target: hook_cam_reverse as *const () as usize,
            original: &REAL_CAM_REVERSE,
        },
    ];
    patch_imports(game as *mut u8, b"savagewind", GetModuleHandleW(std::ptr::null()), &hooks)
}

fn run(motion: Option<Config>, camera: f32, log: impl Fn(String)) {
    let mut game_code = GameCode { wanted: (camera - 1.0).abs() >= 0.001, hooked: false };
    if let Err(e) = unsafe { install_hooks() } {
        log(format!("Controller hooks: off, {e}"));
        return;
    }
    let Some(config) = motion else {
        loop {
            game_code.tick(&log);
            std::thread::sleep(REQUEST_EVERY);
        }
    };
    log(format!("Cemuhook: ready, server {}", config.server));
    let Some(server) = resolve(&config.server) else {
        log(format!("Cemuhook: off, {} isn't a valid server address", config.server));
        loop {
            game_code.tick(&log);
            std::thread::sleep(REQUEST_EVERY);
        }
    };
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        log("Cemuhook: off, no network socket".into());
        return;
    };
    if socket.connect(server).is_err() || socket.set_read_timeout(Some(Duration::from_millis(50))).is_err() {
        log("Cemuhook: off, can't reach the server".into());
        return;
    }
    let client_id = unsafe { GetCurrentProcessId() } ^ (now_ms() as u32).rotate_left(16);
    let mut client = Client {
        slot: config.slot,
        fixed_slot: config.slot.is_some(),
        mac: [0; 6],
        last_seq: None,
        last_stamp: 0,
        last_packet: None,
        rate: 0.0,
        was_alive: false,
    };
    let mut next_request = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        if Instant::now() >= next_request {
            next_request = Instant::now() + REQUEST_EVERY;
            let _ = socket.send(&request(client_id, 0x100001, 0));
            if let Some(slot) = client.slot {
                let _ = socket.send(&request(client_id, 0x100002, slot));
            }
            game_code.tick(&log);
        }
        for _ in 0..256 {
            let n = match socket.recv(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    if !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    break;
                }
            };
            let Some(packet) = decode(&buf[..n]) else { continue };
            match packet.kind {
                0x100001 => client.on_port_info(packet.body, &log),
                _ => client.on_data(packet.body),
            }
        }
        client.check_alive(&log);
    }
}

pub fn start(motion: Option<Config>, camera: f32, log: impl Fn(String) + Send + 'static) {
    PREFER.store(motion.as_ref().is_some_and(|c| c.mode == Mode::Prefer), Ordering::Release);
    CAMERA_BITS.store(camera.to_bits(), Ordering::Release);
    START.get_or_init(Instant::now);
    let _ = std::thread::Builder::new()
        .name("swmm-input".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(motion, camera, log)));
        });
}
