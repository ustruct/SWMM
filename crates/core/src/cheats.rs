use std::path::{Path, PathBuf};

use anyhow_ext::{Context, Result};
use fs_err as fs;

use crate::settings::Settings;

pub const SETTINGS_FILE: &str = "savagewind.ini";

pub const WEAR_LEVELS: [i32; 7] = [0, 50, 100, 150, 200, 250, 300];

pub const WEATHERS: [&str; 12] = [
    "game",
    "no_rain",
    "no_storms",
    "clear",
    "cloudy",
    "rain",
    "heavy_rain",
    "snow",
    "heavy_snow",
    "thunderstorm",
    "thunder_rain",
    "sun_shower",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cheat {
    GodMode,
    InfiniteStamina,
    InfiniteArrows,
    InfiniteRupees,
    WeaponWear,
    Weather,
}

impl Cheat {
    pub const ALL: [Cheat; 6] = [
        Cheat::GodMode,
        Cheat::InfiniteStamina,
        Cheat::InfiniteArrows,
        Cheat::InfiniteRupees,
        Cheat::WeaponWear,
        Cheat::Weather,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Cheat::GodMode => "god_mode",
            Cheat::InfiniteStamina => "infinite_stamina",
            Cheat::InfiniteArrows => "infinite_arrows",
            Cheat::InfiniteRupees => "infinite_rupees",
            Cheat::WeaponWear => "weapon_wear",
            Cheat::Weather => "weather",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cheats {
    pub god_mode: bool,
    pub infinite_stamina: bool,
    pub infinite_arrows: bool,
    pub infinite_rupees: bool,
    pub weapon_wear: i32,
    pub weather: usize,
}

impl Default for Cheats {
    fn default() -> Self {
        Self {
            god_mode: false,
            infinite_stamina: false,
            infinite_arrows: false,
            infinite_rupees: false,
            weapon_wear: 100,
            weather: 0,
        }
    }
}

fn split_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with(';') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    Some((key.trim(), value.trim().trim_matches('"')))
}

fn flag(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "on" | "true" | "yes")
}

fn number(value: &str) -> Option<i32> {
    value.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.round() as i32)
}

pub fn snap_wear(value: i32) -> i32 {
    WEAR_LEVELS.iter().copied().min_by_key(|v| (v - value).abs()).unwrap_or(100)
}

impl Cheats {
    pub fn parse(text: &str) -> Self {
        let mut cheats = Self::default();
        let mut no_wear = false;
        for (key, value) in text.lines().filter_map(split_line) {
            match key {
                "god_mode" => cheats.god_mode = flag(value),
                "infinite_stamina" => cheats.infinite_stamina = flag(value),
                "infinite_arrows" => cheats.infinite_arrows = flag(value),
                "infinite_rupees" => cheats.infinite_rupees = flag(value),
                "no_weapon_wear" => no_wear = flag(value),
                "weapon_wear" => {
                    if let Some(v) = number(value) {
                        cheats.weapon_wear = snap_wear(v);
                    }
                }
                "weather" => {
                    let lower = value.to_ascii_lowercase();
                    cheats.weather = WEATHERS
                        .iter()
                        .position(|w| *w == lower)
                        .or_else(|| lower.parse::<usize>().ok().filter(|i| *i < WEATHERS.len()))
                        .unwrap_or(0);
                }
                _ => {}
            }
        }
        if no_wear {
            cheats.weapon_wear = 0;
        }
        cheats
    }

    pub fn from_game(data: &serde_json::Value) -> Option<Self> {
        let object = data.as_object()?;
        if !object.contains_key("god_mode") {
            return None;
        }
        let mut cheats = Self::default();
        let flag = |key: &str| match object.get(key) {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
            Some(serde_json::Value::String(s)) => self::flag(s),
            _ => false,
        };
        let int = |key: &str, default: i32| match object.get(key) {
            Some(serde_json::Value::Number(n)) => n.as_f64().map(|v| v.round() as i32).unwrap_or(default),
            Some(serde_json::Value::String(s)) => number(s).unwrap_or(default),
            _ => default,
        };
        cheats.god_mode = flag("god_mode");
        cheats.infinite_stamina = flag("infinite_stamina");
        cheats.infinite_arrows = flag("infinite_arrows");
        cheats.infinite_rupees = flag("infinite_rupees");
        cheats.weapon_wear = if flag("no_weapon_wear") { 0 } else { snap_wear(int("weapon_wear", 100)) };
        cheats.weather = match object.get("weather") {
            Some(serde_json::Value::String(s)) => WEATHERS.iter().position(|w| w.eq_ignore_ascii_case(s)).unwrap_or(0),
            Some(serde_json::Value::Number(n)) => n.as_u64().map(|i| i as usize).filter(|i| *i < WEATHERS.len()).unwrap_or(0),
            _ => 0,
        };
        Some(cheats)
    }

    pub fn changed(&self, other: &Cheats) -> Vec<Cheat> {
        Cheat::ALL.iter().copied().filter(|c| self.value(*c) != other.value(*c)).collect()
    }

    pub fn command(&self, cheat: Cheat) -> String {
        format!("set {} {}", cheat.key(), self.value(cheat))
    }

    pub fn active(&self) -> usize {
        let defaults = Self::default();
        Cheat::ALL.iter().filter(|c| self.value(**c) != defaults.value(**c)).count()
    }

    pub fn value(&self, cheat: Cheat) -> String {
        let on = |b: bool| if b { "1" } else { "0" }.to_string();
        match cheat {
            Cheat::GodMode => on(self.god_mode),
            Cheat::InfiniteStamina => on(self.infinite_stamina),
            Cheat::InfiniteArrows => on(self.infinite_arrows),
            Cheat::InfiniteRupees => on(self.infinite_rupees),
            Cheat::WeaponWear => snap_wear(self.weapon_wear).to_string(),
            Cheat::Weather => WEATHERS[self.weather.min(WEATHERS.len() - 1)].to_string(),
        }
    }

    pub fn apply_to(&self, text: &str) -> String {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let mut written = [false; Cheat::ALL.len()];
        let mut wrote_no_wear = false;
        let mut lines: Vec<String> = Vec::new();
        for line in text.lines() {
            let Some((key, _)) = split_line(line) else {
                lines.push(line.to_string());
                continue;
            };
            if key == "no_weapon_wear" {
                if !wrote_no_wear {
                    lines.push("no_weapon_wear = 0".to_string());
                    wrote_no_wear = true;
                }
                continue;
            }
            match Cheat::ALL.iter().position(|c| c.key() == key) {
                Some(index) if written[index] => {}
                Some(index) => {
                    written[index] = true;
                    lines.push(format!("{key} = {}", self.value(Cheat::ALL[index])));
                }
                None => lines.push(line.to_string()),
            }
        }
        for (index, cheat) in Cheat::ALL.iter().enumerate() {
            if !written[index] {
                lines.push(format!("{} = {}", cheat.key(), self.value(*cheat)));
            }
        }
        let mut out = lines.join(newline);
        out.push_str(newline);
        out
    }
}

pub fn settings_file(settings: &Settings) -> Option<PathBuf> {
    crate::savage_wind::data_root(&settings.savage_wind).map(|root| root.join(SETTINGS_FILE))
}

fn read_text(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

pub fn load_from(path: &Path) -> Result<Cheats> {
    Ok(Cheats::parse(&read_text(path)?))
}

pub fn save_to(path: &Path, cheats: &Cheats) -> Result<()> {
    let text = read_text(path)?;
    let out = cheats.apply_to(&text);
    if out == text {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("ini.swmm");
    fs::write(&temp, out.as_bytes())?;
    fs::rename(&temp, path).with_context(|| format!("Could not write {}", path.display()))?;
    Ok(())
}

pub fn load(settings: &Settings) -> Result<(PathBuf, Cheats)> {
    let path = settings_file(settings)
        .context("Savage Wind's data folder is unknown. Choose savagewind.exe or the game data folder in Settings first.")?;
    let cheats = load_from(&path)?;
    Ok((path, cheats))
}

pub fn game_running() -> bool {
    crate::savage_wind::running_executable().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Savage Wind settings\r\nsettings_version = 4\r\nno_weapon_wear = 0\r\nweapon_wear = 100\r\ninfinite_arrows = 0\r\ninfinite_stamina = 0\r\ninfinite_rupees = 0\r\nweather = game\r\ncolour_vision = off\r\ngod_mode = 0\r\none_hit_kill = 0\r\ndamage_taken = 100\r\ndamage_dealt = 100\r\nhaze = 100\r\n";

    #[test]
    fn reads_defaults() {
        let cheats = Cheats::parse(SAMPLE);
        assert_eq!(cheats, Cheats::default());
        assert_eq!(cheats.active(), 0);
        assert_eq!(cheats.apply_to(SAMPLE), SAMPLE);
    }

    #[test]
    fn writes_in_place() {
        let mut cheats = Cheats::parse(SAMPLE);
        cheats.god_mode = true;
        cheats.infinite_stamina = true;
        cheats.weapon_wear = 0;
        cheats.weather = 3;
        let out = cheats.apply_to(SAMPLE);
        assert!(out.contains("\r\ngod_mode = 1\r\n"));
        assert!(out.contains("\r\ninfinite_stamina = 1\r\n"));
        assert!(out.contains("\r\nweapon_wear = 0\r\n"));
        assert!(out.contains("\r\none_hit_kill = 0\r\ndamage_taken = 100\r\ndamage_dealt = 100\r\n"));
        assert!(out.contains("\r\nweather = clear\r\n"));
        assert!(out.contains("\r\ncolour_vision = off\r\ngod_mode = 1\r\n"));
        assert_eq!(out.lines().count(), SAMPLE.lines().count());
        assert_eq!(Cheats::parse(&out), cheats);
        assert_eq!(cheats.active(), 4);
    }

    #[test]
    fn adds_missing_keys_and_clamps() {
        let text = "fullscreen = 1\nweapon_wear = 120\ndamage_taken = 9000\nweather = 99\nno_weapon_wear = 1\n";
        let cheats = Cheats::parse(text);
        assert_eq!(cheats.weapon_wear, 0);
        assert_eq!(cheats.weather, 0);
        let out = cheats.apply_to(text);
        assert!(out.starts_with("fullscreen = 1\n"));
        assert!(out.contains("no_weapon_wear = 0\n"));
        assert!(out.contains("god_mode = 0\n"));
        assert!(out.contains("damage_taken = 9000\n"));
        assert_eq!(Cheats::parse(&out), cheats);
        assert_eq!(Cheats::parse("weapon_wear = 120").weapon_wear, 100);
    }

    #[test]
    fn from_game() {
        let data = serde_json::json!({
            "fullscreen": true,
            "no_weapon_wear": false,
            "weapon_wear": 120,
            "infinite_arrows": false,
            "infinite_stamina": true,
            "infinite_rupees": false,
            "weather": "clear",
            "god_mode": true,
            "one_hit_kill": false,
            "damage_taken": 30,
            "damage_dealt": 400
        });
        let cheats = Cheats::from_game(&data).unwrap();
        assert!(cheats.god_mode && cheats.infinite_stamina && !cheats.infinite_arrows);
        assert_eq!(cheats.weapon_wear, 100);
        assert_eq!(cheats.weather, 3);
        assert!(Cheats::from_game(&serde_json::json!({"pong": true})).is_none());
        let changed = Cheats::default().changed(&cheats);
        assert_eq!(changed.len(), 3);
        assert_eq!(cheats.command(Cheat::GodMode), "set god_mode 1");
        assert_eq!(cheats.command(Cheat::Weather), "set weather clear");
        assert_eq!(cheats.command(Cheat::WeaponWear), "set weapon_wear 100");
    }

    #[test]
    fn empty_file() {
        let out = Cheats::default().apply_to("");
        assert_eq!(out.lines().count(), Cheat::ALL.len());
        assert_eq!(Cheats::parse(&out), Cheats::default());
    }
}
