use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};

use crate::game::{game, player_info, rd, wr, Detect, Game};

const PLAYER_ACTOR: u64 = 0x60;
const PLAYER_MAX_STAMINA: u64 = 0x186c;
const INFO_STAMINA: u64 = 0xa4;
const POWERS: [u64; 4] = [0x1cc8, 0x1ccc, 0x1cd0, 0x1cd4];
const MIPHA_TIMER: u64 = 0x1e1c;
const STEP_MS: u64 = 16;
const SLOW_MS: u64 = 250;
const MOTORCYCLE_FULL: f32 = 1000.0;
const AMIIBO_RESET: i32 = 19_700_101;

#[derive(Clone, Copy)]
enum Cheat {
    Hearts,
    Stamina,
    Motorcycle,
    Amiibo,
    Mipha,
    Revali,
    Urbosa,
    Daruk,
}

const CHEATS: [(Cheat, &str); 8] = [
    (Cheat::Hearts, "infinite_hearts"),
    (Cheat::Stamina, "infinite_stamina"),
    (Cheat::Motorcycle, "infinite_motorcycle_energy"),
    (Cheat::Amiibo, "infinite_amiibo"),
    (Cheat::Mipha, "infinite_mipha"),
    (Cheat::Revali, "infinite_revali"),
    (Cheat::Urbosa, "infinite_urbosa"),
    (Cheat::Daruk, "infinite_daruk"),
];

static ENABLED: [AtomicBool; 8] = [const { AtomicBool::new(false) }; 8];
static IN_TICK: AtomicBool = AtomicBool::new(false);
static LAST_STEP: AtomicU64 = AtomicU64::new(0);
static LAST_SLOW: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static STATUS: Mutex<String> = Mutex::new(String::new());

static BEST: Mutex<(u64, [i32; 4])> = Mutex::new((0, [0; 4]));

fn on(cheat: Cheat) -> bool {
    ENABLED[cheat as usize].load(Ordering::Relaxed)
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("SWMM"))
}

fn read_config(path: &PathBuf) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    for (cheat, key) in CHEATS {
        let value = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .is_some_and(|(_, v)| v.trim() == "1");
        ENABLED[cheat as usize].store(value, Ordering::Relaxed);
    }
}

fn set_status(text: String) {
    let Ok(mut status) = STATUS.lock() else { return };
    if *status == text {
        return;
    }
    *status = text.clone();
    if let Some(dir) = config_dir() {
        let _ = std::fs::write(dir.join("cheats_status.ini"), text);
    }
}

fn watch() {
    let Some(dir) = config_dir() else { return };
    let path = dir.join("cheats.ini");
    let mut seen: Option<SystemTime> = None;
    loop {
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified != seen {
            seen = modified;
            read_config(&path);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn start() {
    START.get_or_init(Instant::now);
    set_status("state=starting\n".into());
    let _ = std::thread::Builder::new().name("swmm-cheats".into()).spawn(|| {
        let _ = std::panic::catch_unwind(watch);
    });
}

unsafe fn fast(game: &Game, info: u64) {
    let b = game.build;
    if on(Cheat::Hearts) {
        let life = game.call(b.get_life, &[info]) as u32 as i32;
        let max = game.call(b.get_max_life, &[info]) as u32 as i32;
        if life > 0 && life < max {
            game.call(b.recover_life, &[info]);
        }
    }
    let player: u64 = rd(info + PLAYER_ACTOR);
    if player < 0x10000 {
        return;
    }
    if on(Cheat::Stamina) {
        let max: f32 = rd(player + PLAYER_MAX_STAMINA);
        let now: f32 = rd(info + INFO_STAMINA);
        if max.is_finite() && (100.0..=10000.0).contains(&max) && now.is_finite() && now < max {
            wr::<f32>(info + INFO_STAMINA, max);
        }
    }
    let Ok(mut guard) = BEST.try_lock() else { return };
    if guard.0 != player {
        *guard = (player, [0; 4]);
    }
    let best = &mut guard.1;
    for (i, cheat) in [Cheat::Mipha, Cheat::Revali, Cheat::Urbosa, Cheat::Daruk].into_iter().enumerate() {
        let at = player + POWERS[i];
        let count: i32 = rd(at);
        if !(0..=99).contains(&count) {
            continue;
        }
        if count > best[i] {
            best[i] = count;
        }
        if on(cheat) && count > 0 && count < best[i] {
            wr::<i32>(at, best[i]);
        }
    }
    if on(Cheat::Mipha) && best[0] > 0 {
        let timer: f32 = rd(player + MIPHA_TIMER);
        if timer.is_finite() && timer > 0.0 {
            wr::<f32>(player + MIPHA_TIMER, 0.0);
        }
    }
}

unsafe fn slow(game: &Game) {
    if on(Cheat::Motorcycle) && game.get_f32("Motorcycle_Energy").is_some_and(|v| v < MOTORCYCLE_FULL) {
        game.set_f32("Motorcycle_Energy", MOTORCYCLE_FULL);
    }
    if on(Cheat::Amiibo) && game.get_s32("AmiiboLastTouchDate").is_some_and(|v| v != AMIIBO_RESET) {
        game.set_s32("AmiiboLastTouchDate", AMIIBO_RESET);
    }
    set_status(format!("state=ready\nversion={}\n", game.build.name));
}

fn run(now_ms: u64) {
    let Some(detect) = game() else { return };
    let game = match detect {
        Detect::Ready(game) => game,
        Detect::Unsupported => {
            set_status("state=unsupported\n".into());
            return;
        }
    };
    let Some(info) = player_info() else {
        set_status(format!("state=loading\nversion={}\n", game.build.name));
        return;
    };
    unsafe { fast(game, info) };
    if now_ms.saturating_sub(LAST_SLOW.load(Ordering::Acquire)) >= SLOW_MS {
        LAST_SLOW.store(now_ms, Ordering::Release);
        unsafe { slow(game) };
    }
}

pub fn tick() {
    let now_ms = START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1;
    if now_ms.saturating_sub(LAST_STEP.load(Ordering::Acquire)) < STEP_MS {
        return;
    }
    if IN_TICK.swap(true, Ordering::AcqRel) {
        return;
    }
    LAST_STEP.store(now_ms, Ordering::Release);
    let _ = std::panic::catch_unwind(|| run(now_ms));
    IN_TICK.store(false, Ordering::Release);
}
