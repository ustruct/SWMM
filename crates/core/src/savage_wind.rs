use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Arc,
};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use uk_content::prelude::Endian;
use uk_reader::ResourceReader;

use uk_content::constants::Language;

use crate::settings::{DeployConfig, Platform, PlatformSettings, SavageWindSettings, Settings};

pub const OVERLAY_ENV: &str = "SAVAGEWIND_ROMFS_OVERLAY";

pub const DATA_ROOT_ENV: &str = "SAVAGEWIND_DATA_ROOT";

pub const CONTROL_ENV: &str = "SAVAGEWIND_CONTROL";

pub const CONTROL_PORT: u16 = 47123;

pub const SUPPORTED_VERSIONS: &[&str] = &["1.9.0", "1.8.2"];

pub const AOC_MAX: usize = 8;

#[cfg(windows)]
pub const EXE_NAMES: &[&str] = &["savagewind.exe", "SavageWind.exe"];
#[cfg(target_os = "macos")]
pub const EXE_NAMES: &[&str] = &["Savage Wind.app", "savagewind"];
#[cfg(all(unix, not(target_os = "macos")))]
pub const EXE_NAMES: &[&str] = &["savagewind", "Savage_Wind.AppImage"];

pub fn is_savage_wind_exe(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    let squashed: std::string::String = name.chars().filter(|c| c.is_alphanumeric() || *c == '.').collect();
    EXE_NAMES.iter().any(|n| n.to_lowercase() == name)
        || (cfg!(all(unix, not(target_os = "macos")))
            && squashed.contains("savagewind")
            && name.ends_with(".appimage"))
}

pub const CONFIG_EXE: &str = "Savage Wind Configuration.exe";

fn check_program(exe: &Path) -> Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    let mut head = [0u8; 2];
    let read = std::fs::File::open(exe).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head));
    if read.is_err() || &head != b"MZ" {
        anyhow_ext::bail!(
            "{} isn't a Windows program. Choose savagewind.exe on the Settings page.",
            exe.display()
        );
    }
    Ok(())
}

fn chosen_exe(settings: &Settings) -> Result<PathBuf> {
    let exe = settings
        .savage_wind
        .executable
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .context("Savage Wind's executable isn't set. Choose it on the Settings page.")?;
    if !exe.exists() {
        anyhow_ext::bail!("Savage Wind's executable was not found at {}", exe.display());
    }
    check_program(exe)?;
    Ok(exe.clone())
}

pub fn config_command(settings: &Settings) -> Result<Command> {
    let exe = chosen_exe(settings)?;
    let dir = exe.parent().map(Path::to_path_buf);
    let tool = dir.as_ref().map(|d| d.join(CONFIG_EXE)).filter(|p| cfg!(windows) && p.is_file());
    let mut command = match tool {
        Some(tool) => Command::new(tool),
        None => {
            let mut c = Command::new(&exe);
            c.arg("--config");
            c
        }
    };
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    if let Some(overlay) = overlay_dir(settings).filter(|d| has_files(d)) {
        command.env(OVERLAY_ENV, &overlay);
    }
    Ok(command)
}

pub fn launch_config(settings: &Settings) -> Result<Child> {
    config_command(settings)?
        .spawn()
        .context("Failed to open Savage Wind's Configuration")
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

const SKIP_DIRS: &[&str] = &[
    "windows",
    "$recycle.bin",
    "system volume information",
    "programdata",
    "appdata",
    "node_modules",
    "winsxs",
    "recovery",
    "perflogs",
    "microsoft",
    "windowsapps",
    "common files",
];

fn names_the_game(name: &str) -> bool {
    name.chars().filter(|c| c.is_alphanumeric()).collect::<std::string::String>().to_lowercase().contains("savage")
}

struct Search {
    seen: std::collections::HashSet<PathBuf>,
    found: Vec<PathBuf>,
    budget: usize,
}

impl Search {
    fn visit(&mut self, dir: &Path, depth: usize) {
        if self.budget == 0 || self.found.len() >= 12 || !self.seen.insert(dir.to_path_buf()) {
            return;
        }
        self.budget -= 1;
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut subs = Vec::new();
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if is_savage_wind_exe(&path) && (kind.is_file() || cfg!(target_os = "macos") && kind.is_dir()) {
                if !self.found.contains(&path) {
                    self.found.push(path);
                }
            } else if kind.is_dir() {
                subs.push(path);
            }
        }
        for sub in subs {
            let name = sub.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name.starts_with('.') || name.starts_with('$') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let next = if names_the_game(&name) {
                depth.max(4) - 1
            } else if depth > 0 {
                depth - 1
            } else {
                continue;
            };
            self.visit(&sub, next);
        }
    }
}

pub fn running_executable() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        crate::process::find_by_name(EXE_NAMES)
            .into_iter()
            .find(|p| p.is_file() && is_savage_wind_exe(p))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .filter_map(|e| std::fs::read_link(e.path().join("exe")).ok())
            .find(|p| is_savage_wind_exe(p))
    }
    #[cfg(target_os = "macos")]
    {
        None
    }
}

fn steam_libraries() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(var) {
                roots.push(PathBuf::from(dir).join("Steam"));
            }
        }
    } else if let Some(home) = dirs2::home_dir() {
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        roots.push(home.join("Library/Application Support/Steam"));
    }
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in roots {
        let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) else { continue };
        libraries.push(root.join("steamapps").join("common"));
        for line in text.lines() {
            let mut parts = line.split('"').filter(|p| !p.trim().is_empty());
            if parts.next().map(str::trim) == Some("path")
                && let Some(path) = parts.next()
            {
                let library = PathBuf::from(path.replace("\\\\", "\\")).join("steamapps").join("common");
                if !libraries.contains(&library) {
                    libraries.push(library);
                }
            }
        }
    }
    libraries
}

#[cfg(windows)]
fn local_drives() -> Vec<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    (b'A'..=b'Z')
        .map(|letter| format!("{}:\\", letter as char))
        .filter(|root| {
            let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            kind == 2 || kind == 3
        })
        .map(PathBuf::from)
        .collect()
}

fn search_roots() -> Vec<(PathBuf, usize)> {
    let mut roots: Vec<(PathBuf, usize)> = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        if let Some(parent) = dir.parent() {
            roots.push((parent.to_path_buf(), 2));
        }
        roots.push((dir, 1));
    }
    for library in steam_libraries() {
        roots.push((library, 2));
    }
    for dir in [dirs2::download_dir(), dirs2::desktop_dir(), dirs2::document_dir()].into_iter().flatten() {
        roots.push((dir, 2));
    }
    if let Some(home) = dirs2::home_dir() {
        roots.push((home.join("Games"), 3));
        roots.push((home.join("Applications"), 2));
        roots.push((home, 1));
    }
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var) {
                roots.push((PathBuf::from(dir), 2));
            }
        }
        if let Some(dir) = dirs2::data_local_dir() {
            roots.push((dir.join("Programs"), 2));
        }
        for drive in local_drives() {
            roots.push((drive.join("Games"), 3));
            roots.push((drive, 2));
        }
    }
    #[cfg(target_os = "macos")]
    roots.push((PathBuf::from("/Applications"), 1));
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        roots.push((PathBuf::from("/opt"), 2));
        roots.push((PathBuf::from("/run/media"), 3));
        roots.push((PathBuf::from("/media"), 3));
        roots.push((PathBuf::from("/mnt"), 3));
    }
    roots
}

fn rank(exe: &Path) -> (bool, std::time::SystemTime) {
    let ready = exe.parent().is_some_and(|dir| dir.join("extracted").is_dir());
    let modified = std::fs::metadata(exe).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    (ready, modified)
}

pub fn detect_executable() -> Option<PathBuf> {
    if let Some(exe) = running_executable() {
        return Some(exe);
    }
    let mut search = Search {
        seen: Default::default(),
        found: Vec::new(),
        budget: 20_000,
    };
    for (root, depth) in search_roots() {
        search.visit(&root, depth);
        if search.found.iter().any(|exe| rank(exe).0) {
            break;
        }
    }
    log::debug!("Savage Wind candidates: {:?}", search.found);
    search.found.into_iter().max_by_key(|exe| rank(exe))
}

pub fn data_root(config: &SavageWindSettings) -> Option<PathBuf> {
    if let Some(root) = std::env::var_os(DATA_ROOT_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(root));
    }
    if let Some(dir) = config.data_dir.as_ref().filter(|p| !p.as_os_str().is_empty()) {
        return Some(dir.clone());
    }
    let exe_dir = config
        .executable
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = exe_dir.as_ref()
        && dir.join("extracted").is_dir() {
            return Some(dir.clone());
        }
    #[cfg(windows)]
    {
        exe_dir
    }
    #[cfg(target_os = "macos")]
    {
        let _ = exe_dir;
        dirs2::home_dir().map(|h| h.join("Library/Application Support/local.savagewind"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = exe_dir;
        if let Some(home) = config.executable.as_deref().and_then(appimage_portable_home) {
            return Some(home.join(".local/share/savagewind"));
        }
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| dirs2::home_dir().map(|h| h.join(".local/share")))
            .map(|d| d.join("savagewind"))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn appimage_portable_home(exe: &Path) -> Option<PathBuf> {
    let name = exe.file_name()?.to_str()?;
    if !name.to_ascii_lowercase().ends_with(".appimage") {
        return None;
    }
    Some(exe.with_file_name(format!("{name}.home"))).filter(|home| home.is_dir())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameFiles {

    pub romfs: PathBuf,

    pub aoc: Option<PathBuf>,

    pub all_aoc: Vec<PathBuf>,

    pub version: Option<String>,

    pub aoc_version: Option<String>,
}

impl GameFiles {
    #[inline]
    pub fn is_supported_version(&self) -> bool {
        self.version
            .as_deref()
            .map(|v| SUPPORTED_VERSIONS.contains(&v))
            .unwrap_or(false)
    }

    pub fn describe(&self) -> String {
        let mut text = self.version.clone().unwrap_or_else(|| "unknown version".into());
        if let Some(aoc) = self.aoc_version.as_ref() {
            text.push_str(" + DLC ");
            text.push_str(aoc);
        } else if self.aoc.is_some() {
            text.push_str(" + DLC");
        }
        text
    }
}

fn read_version(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().trim_matches(char::from(0)).to_string())
        .filter(|s| !s.is_empty())
}

fn aoc_romfs(dir: &Path) -> Option<PathBuf> {
    [dir.to_path_buf(), dir.join("romfs")]
        .into_iter()
        .find(|d| d.join("System").join("AocVersion.txt").is_file() || d.join("Pack").is_dir())
}

pub fn game_files(data_root: &Path) -> Result<GameFiles> {
    let extracted = data_root.join("extracted");
    let romfs = extracted.join("romfs");
    if !romfs.is_dir() {
        anyhow_ext::bail!(
            "No game files found in {}. Set up your game files in Savage Wind first (the folder should contain \
             extracted\\romfs).",
            data_root.display()
        );
    }
    let version = read_version(&romfs.join("System").join("Version.txt"));
    let mut all_aoc = Vec::new();
    for index in 1..=AOC_MAX {
        if let Some(dir) = aoc_romfs(&extracted.join(format!("aoc{index}"))) {
            all_aoc.push(dir);
        }
    }

    let aoc = all_aoc
        .iter()
        .rev()
        .find(|d| d.join("Pack").join("AocMainField.pack").is_file())
        .or_else(|| all_aoc.last())
        .cloned();
    let aoc_version = aoc
        .as_ref()
        .and_then(|d| read_version(&d.join("System").join("AocVersion.txt")));
    Ok(GameFiles {
        romfs,
        aoc,
        all_aoc,
        version,
        aoc_version,
    })
}

pub fn current_game_files(settings: &Settings) -> Result<GameFiles> {
    let root = data_root(&settings.savage_wind)
        .context("Savage Wind's data folder is unknown. Choose savagewind.exe or the game data folder in Settings first.")?;
    game_files(&root)
}

pub fn build_dump(game: &GameFiles) -> Result<Arc<ResourceReader>> {
    Ok(Arc::new(
        ResourceReader::from_unpacked_dirs(
            Some(&game.romfs),
            None::<&Path>,
            game.aoc.as_ref(),
            Endian::Little,
        )
        .with_context(|| format!("Could not read the game files in {}", game.romfs.display()))?,
    ))
}

pub fn game_language(data_root: &Path) -> Option<Language> {
    let ini = std::fs::read_to_string(data_root.join("savagewind.ini")).ok()?;
    ini.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "text_language" {
            return None;
        }
        let value = value.trim().trim_matches('"');
        Language::iter().copied().find(|l| l.to_str() == value)
    })
}

pub fn default_overlay_dir(settings: &Settings) -> PathBuf {
    settings.get_platform_dir(Platform::Switch).join("overlay")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoConfig {
    pub found_executable: bool,
    pub configured_dump: bool,
    pub configured_deploy: bool,
}

impl AutoConfig {
    #[inline]
    pub fn changed(&self) -> bool {
        self.found_executable || self.configured_dump || self.configured_deploy
    }
}

pub fn auto_configure(settings: &mut Settings) -> Result<AutoConfig> {
    let mut result = AutoConfig::default();
    settings.current_mode = Platform::Switch;
    let has_exe = settings
        .savage_wind
        .executable
        .as_ref()
        .map(|p| !p.as_os_str().is_empty() && p.exists())
        .unwrap_or(false);
    if !has_exe
        && let Some(exe) = detect_executable() {
            log::info!("Found Savage Wind at {}", exe.display());
            settings.savage_wind.executable = Some(exe);
            result.found_executable = true;
        }
    let game = current_game_files(settings)?;
    if !game.is_supported_version() {
        log::warn!(
            "Savage Wind's game files say version {:?}; Savage Wind supports {:?}",
            game.version,
            SUPPORTED_VERSIONS
        );
    }

    let expected_host: &Path = match game.aoc.as_ref() {
        Some(_) => game.romfs.parent().unwrap_or(&game.romfs),
        None => &game.romfs,
    };
    let needs_dump = match settings.switch_config.as_ref() {
        Some(config) => config.dump.source().host_path() != expected_host,
        None => true,
    };
    let language = data_root(&settings.savage_wind)
        .and_then(|root| game_language(&root))
        .unwrap_or_default();
    let default_overlay = default_overlay_dir(settings);
    match settings.switch_config.as_mut() {
        Some(config) => {
            if needs_dump {
                config.dump = build_dump(&game)?;
                result.configured_dump = true;
            }
            if config.deploy_config.is_none() {
                config.deploy_config = Some(DeployConfig {
                    output: default_overlay,
                    ..Default::default()
                });
                result.configured_deploy = true;
            }
        }
        None => {
            settings.switch_config = Some(PlatformSettings {
                language,
                profile: "Default".into(),
                dump: build_dump(&game)?,
                deploy_config: Some(DeployConfig {
                    output: default_overlay,
                    ..Default::default()
                }),
            });
            result.configured_dump = true;
            result.configured_deploy = true;
        }
    }
    Ok(result)
}

pub fn overlay_dir(settings: &Settings) -> Option<PathBuf> {
    settings
        .platform_config()
        .and_then(|c| c.deploy_config.as_ref())
        .map(DeployConfig::overlay_dir)
        .filter(|p| !p.as_os_str().is_empty())
}

pub fn follow_storage(settings: &mut Settings, old_storage: &Path) -> bool {
    let target = default_overlay_dir(settings);
    let Some(config) = settings.platform_config_mut().and_then(|c| c.deploy_config.as_mut()) else {
        return false;
    };
    if same_file(&config.output, &target) {
        return false;
    }
    let stale = [
        old_storage.join("overlay"),
        crate::settings::default_storage().join("overlay"),
    ];
    if !stale.iter().any(|p| same_file(&config.output, p)) {
        return false;
    }
    log::info!("Moving the mod folder from {} to {}", config.output.display(), target.display());
    config.output = target;
    true
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

pub const LOADER_DLL: &str = "pdh.dll";
pub const LOADER_BUILD: &str = "swmm.dll";
const LOADER_BUILD_RAW: &str = "swmm_loader.dll";
pub const LOADER_CONFIG: &str = "loader.ini";
const LOADER_MARKER: &[u8] = b"SWMM-Loader";
const LEGACY_LOADER_FILES: &[&str] = &["swmm.ini", "swmm_hook.log"];

fn is_our_loader(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| {
            b.windows(LOADER_MARKER.len()).any(|w| w == LOADER_MARKER)
                || b.windows(24).any(|w| w == b"SavageWindModManagerHook")
        })
        .unwrap_or(false)
}

fn loader_build() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join(LOADER_BUILD),
        dir.join(LOADER_BUILD_RAW),
        dir.join("..").join(LOADER_BUILD),
        dir.join("..").join(LOADER_BUILD_RAW),
    ]
    .into_iter()
        .find(|p| p.is_file())
}

pub fn loader_config_path() -> PathBuf {
    Settings::config_dir().join(LOADER_CONFIG)
}

fn game_dir(settings: &Settings) -> Option<PathBuf> {
    settings
        .savage_wind
        .executable
        .as_ref()
        .filter(|p| p.is_file())
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

pub const LAUNCH_FLAGS: &str = "launch-flags.txt";
const LAUNCH_FLAG_VALUE_MAX: usize = 127;
const LAUNCH_FLAGS_HEADER: &str = "# Savage Wind launch options: environment flags set at every start. One NAME=value per line.\n";

fn flag_name(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    line.split('=').next().map(str::trim)
}

fn is_flag(line: &str, name: &str) -> bool {
    flag_name(line).is_some_and(|n| n.eq_ignore_ascii_case(name))
}

fn flag_value<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .filter(|l| is_flag(l, name))
        .find_map(|l| l.split_once('=').map(|(_, v)| v.trim().trim_matches('"')))
}

fn port_value(value: &str) -> Option<u16> {
    value.trim().parse::<u16>().ok().filter(|p| *p >= 1024)
}

pub fn control_port(settings: &Settings) -> u16 {
    if let Some(port) = std::env::var(CONTROL_ENV).ok().as_deref().and_then(port_value) {
        return port;
    }
    data_root(&settings.savage_wind)
        .and_then(|root| std::fs::read_to_string(root.join(LAUNCH_FLAGS)).ok())
        .and_then(|text| flag_value(&text, CONTROL_ENV).and_then(port_value))
        .unwrap_or(CONTROL_PORT)
}

fn control_line(old: &str) -> std::string::String {
    let port = flag_value(old, CONTROL_ENV).and_then(port_value).unwrap_or(CONTROL_PORT);
    format!("{CONTROL_ENV}={port}")
}

fn write_launch_flags(
    settings: &Settings,
    names: &[&str],
    lines: impl FnOnce(&str) -> Result<Vec<std::string::String>>,
) -> Result<()> {
    let root = data_root(&settings.savage_wind).context("Savage Wind's data folder isn't known")?;
    let path = root.join(LAUNCH_FLAGS);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let ours = lines(&old)?;
    match launch_flags_text(&old, &ours, names) {
        None => {
            if path.exists() {
                fs::remove_file(&path)?;
            }
        }
        Some(text) if text != old => {
            fs::create_dir_all(&root)?;
            fs::write(&path, text)?;
        }
        Some(_) => {}
    }
    Ok(())
}

pub fn sync_control_flag(settings: &Settings) -> Result<()> {
    write_launch_flags(settings, &[CONTROL_ENV], |old| Ok(vec![control_line(old)]))
}

fn flag_line(name: &str, dir: &Path) -> Result<std::string::String> {
    let value = dir.display().to_string();
    if value.len() > LAUNCH_FLAG_VALUE_MAX || value.chars().any(char::is_control) {
        anyhow_ext::bail!(
            "The folder {value} is too long for Savage Wind's launch options ({} bytes, at most {LAUNCH_FLAG_VALUE_MAX}). Choose a shorter storage folder in Settings.",
            value.len()
        );
    }
    Ok(format!("{name}={value}"))
}

pub fn sync_launch_flags(settings: &Settings, overlay: Option<&Path>) -> Result<bool> {
    write_launch_flags(settings, &[OVERLAY_ENV, CONTROL_ENV], |old| {
        let mut lines = Vec::new();
        if let Some(dir) = overlay {
            lines.push(flag_line(OVERLAY_ENV, dir)?);
        }
        lines.push(control_line(old));
        Ok(lines)
    })?;
    Ok(overlay.is_some())
}

fn launch_flags_text(old: &str, ours: &[std::string::String], names: &[&str]) -> Option<std::string::String> {
    let mut lines: Vec<&str> = old.lines().filter(|l| !names.iter().any(|n| is_flag(l, n))).collect();
    lines.extend(ours.iter().map(std::string::String::as_str));
    if !lines.iter().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')) {
        return None;
    }
    let mut text = if old.trim().is_empty() { LAUNCH_FLAGS_HEADER.to_owned() } else { std::string::String::new() };
    text.push_str(&lines.join("\n"));
    text.push('\n');
    Some(text)
}

pub fn sync_loader(settings: &Settings) -> Result<std::string::String> {
    if cfg!(target_os = "linux") {
        return sync_linux(settings);
    }
    if !cfg!(windows) {
        let overlay = overlay_dir(settings).filter(|d| has_files(d));
        return Ok(match sync_launch_flags(settings, overlay.as_deref())? {
            true => "Savage Wind loads the mods however it's started".into(),
            false => "Mods are off when Savage Wind is started on its own".into(),
        });
    }
    if let Err(e) = sync_control_flag(settings) {
        log::warn!("Couldn't update Savage Wind's launch options: {e:?}");
    }
    let dir = game_dir(settings).context("Savage Wind's executable isn't set")?;
    for name in LEGACY_LOADER_FILES {
        let path = dir.join(name);
        if path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
    let dll = dir.join(LOADER_DLL);
    let overlay = overlay_dir(settings);
    let enabled = overlay.is_some();
    let config = format!(
        "enabled = {}\noverlay = {}\n",
        if enabled { 1 } else { 0 },
        overlay.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
    );
    fs::create_dir_all(Settings::config_dir())?;
    fs::write(loader_config_path(), config)?;
    if dll.exists() && !is_our_loader(&dll) {
        anyhow_ext::bail!("{} in {} doesn't belong to SWMM, so it was left alone", LOADER_DLL, dir.display());
    }
    if !enabled {
        if dll.exists() {
            let _ = std::fs::remove_file(&dll);
        }
        return Ok("Mods are off when Savage Wind is started on its own".into());
    }
    if let Some(system) = std::env::var_os("SystemRoot")
        && !Path::new(&system).join("System32").join("vcruntime140.dll").exists() {
            anyhow_ext::bail!(
                "The Microsoft Visual C++ Redistributable is missing, so the loader can't be installed. Get it \
                 from https://aka.ms/vs/17/release/vc_redist.x64.exe"
            );
        }
    let build = loader_build()
        .with_context(|| format!("{LOADER_BUILD} wasn't found next to SWMM. Build it with build-windows.bat."))?;
    let new = fs::read(&build)?;
    if std::fs::read(&dll).map(|old| old != new).unwrap_or(true)
        && let Err(e) = fs::write(&dll, &new) {
            if !dll.exists() {
                return Err(e.into());
            }
            log::warn!("Could not update {}: {e}", dll.display());
        }
    Ok(format!("Savage Wind loads the mods when started on its own ({})", dll.display()))
}

fn sync_linux(settings: &Settings) -> Result<std::string::String> {
    let overlay = overlay_dir(settings).filter(|d| has_files(d));
    let flags = sync_launch_flags(settings, overlay.as_deref()).unwrap_or_else(|e| {
        log::warn!("Couldn't update Savage Wind's launch options: {e:?}");
        false
    });
    set_persistent_env(overlay.as_deref())?;
    let _ = match &overlay {
        Some(path) => Command::new("systemctl")
            .args(["--user", "set-environment"])
            .arg(format!("{OVERLAY_ENV}={}", path.display()))
            .status(),
        None => Command::new("systemctl").args(["--user", "unset-environment", OVERLAY_ENV]).status(),
    };
    let exe = settings.savage_wind.executable.clone();
    let changed = crate::steam::update_shortcuts(OVERLAY_ENV, overlay.as_deref(), |path| {
        is_savage_wind_exe(path) || exe.as_deref().is_some_and(|e| same_file(e, path))
    })
    .unwrap_or_else(|e| {
        log::warn!("Couldn't update the Steam shortcuts: {e}");
        0
    });
    Ok(match (&overlay, changed) {
        (None, _) => "Mods are off when Savage Wind is started on its own".into(),
        (Some(_), _) if flags => "Savage Wind loads the mods however it's started".into(),
        (Some(_), 0) => "Savage Wind loads the mods when started on its own (after you log out and back in)".into(),
        (Some(_), n) => format!(
            "Savage Wind loads the mods when started on its own. {n} Steam shortcut(s) were updated: restart Steam to apply"
        ),
    })
}

fn created_dirs_record(settings: &Settings) -> PathBuf {
    settings.storage_dir.join("game-folders.txt")
}

pub fn sync_game_folders(settings: &Settings, overlay: &Path, game: &GameFiles) -> Result<usize> {
    let record = created_dirs_record(settings);
    let mut previous: Vec<PathBuf> = std::fs::read_to_string(&record)
        .map(|t| t.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default();
    previous.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in &previous {
        let _ = std::fs::remove_dir(dir);
    }
    let mut needed: BTreeSet<PathBuf> = BTreeSet::new();
    for issue in validate_overlay(overlay, game) {
        if let Some(rest) = issue.missing_dir.strip_prefix("aoc/0010/") {
            for aoc in &game.all_aoc {
                needed.insert(aoc.join(rest));
            }
        } else {
            needed.insert(game.romfs.join(&issue.missing_dir));
        }
    }
    let mut created: BTreeSet<PathBuf> = previous.into_iter().filter(|p| p.is_dir()).collect();
    for dir in &needed {
        let mut current = dir.clone();
        while !current.exists() {
            created.insert(current.clone());
            if !current.pop() {
                break;
            }
        }
        fs::create_dir_all(dir)?;
    }
    let remaining: Vec<std::string::String> = created
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string())
        .collect();
    if remaining.is_empty() {
        if record.exists() {
            let _ = std::fs::remove_file(&record);
        }
    } else {
        fs::write(&record, remaining.join("\n"))?;
    }
    Ok(needed.len())
}

pub fn clear_legacy_environment() {
    if cfg!(windows) && std::env::var_os(OVERLAY_ENV).is_some() {
        let _ = set_persistent_env(None);
    }
}

pub fn launch_command(settings: &Settings) -> Result<Command> {
    let exe = chosen_exe(settings)?;
    let exe = &exe;
    #[cfg(target_os = "macos")]
    let mut command = if exe.extension().map(|e| e == "app").unwrap_or(false) {
        let mut c = Command::new("open");
        c.arg("-W").arg("-a").arg(exe).arg("--args");
        c
    } else {
        Command::new(exe)
    };
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new(exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    match overlay_dir(settings).filter(|d| has_files(d)) {
        Some(overlay) => {
            log::info!("Starting Savage Wind with {}={}", OVERLAY_ENV, overlay.display());
            command.env(OVERLAY_ENV, &overlay);
        }
        None => {
            log::info!("No mods deployed, starting Savage Wind without an overlay");
            command.env_remove(OVERLAY_ENV);
        }
    }

    if std::env::var_os(CONTROL_ENV).is_none() {
        if let Err(e) = sync_control_flag(settings) {
            log::warn!("Couldn't update Savage Wind's launch options: {e:?}");
        }
        command.env(CONTROL_ENV, control_port(settings).to_string());
    }
    command.env("SWMM_LAUNCHED", "1");
    Ok(command)
}

pub fn launch(settings: &Settings) -> Result<Child> {
    let mut command = launch_command(settings)?;
    command.spawn().context("Failed to start Savage Wind")
}

pub fn set_persistent_env(value: Option<&Path>) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let status = match value {
            Some(path) => Command::new("setx")
                .arg(OVERLAY_ENV)
                .arg(path)
                .creation_flags(CREATE_NO_WINDOW)
                .status(),
            None => Command::new("reg")
                .args(["delete", "HKCU\\Environment", "/v", OVERLAY_ENV, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .status(),
        }
        .context("Failed to run setx / reg")?;
        if !status.success() && value.is_some() {
            anyhow_ext::bail!("setx failed to set {OVERLAY_ENV} ({status})");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let dir = dirs2::config_dir()
            .context("No config folder")?
            .join("environment.d");
        let file = dir.join("60-savagewind-overlay.conf");
        match value {
            Some(path) => {
                fs::create_dir_all(&dir)?;
                fs::write(&file, format!("{}=\"{}\"\n", OVERLAY_ENV, path.display()))?;
            }
            None => {
                if file.exists() {
                    fs::remove_file(&file)?;
                }
            }
        }
        Ok(())
    }
}

const RSTB_PATH: &str = "System/Resource/ResourceSizeTable.product.srsizetable";

fn rstb_name(path: &str) -> String {
    match path.rfind('.') {
        Some(dot)
            if path[dot + 1..].starts_with('s')
                && &path[dot..] != ".sarc"
                && !path[dot..].contains('/') =>
        {
            format!("{}{}", &path[..dot + 1], &path[dot + 2..])
        }
        _ => path.to_string(),
    }
}

fn walk_files(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk_files(root, &path, out),
            Ok(_) => {
                if let (Ok(rel), Ok(meta)) = (path.strip_prefix(root), entry.metadata()) {
                    out.push((rel.to_string_lossy().replace('\\', "/"), meta.len()));
                }
            }
            Err(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRstb {
    crc: Vec<(u32, u32)>,
    names: Vec<([u8; 128], u32)>,
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *entry = c;
        }
        table
    });
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

impl RawRstb {

    pub fn parse(d: &[u8]) -> Option<Self> {
        let u32_at = |o: usize| -> Option<u32> {
            d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        if d.get(0..4)? != b"RSTB" {
            return None;
        }
        let ncrc = u32_at(4)? as usize;
        let nname = u32_at(8)? as usize;
        if 12 + ncrc * 8 + nname * 132 > d.len() {
            return None;
        }
        let crc = (0..ncrc)
            .map(|i| Some((u32_at(12 + i * 8)?, u32_at(16 + i * 8)?)))
            .collect::<Option<Vec<_>>>()?;
        let base = 12 + ncrc * 8;
        let names = (0..nname)
            .map(|i| {
                let o = base + i * 132;
                let mut name = [0u8; 128];
                name.copy_from_slice(d.get(o..o + 128)?);
                Some((name, u32_at(o + 128)?))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { crc, names })
    }

    pub fn read(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let bytes = if bytes.starts_with(b"Yaz0") {
            roead::yaz0::decompress(bytes).ok()?
        } else {
            bytes
        };
        Self::parse(&bytes)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.crc.len() * 8 + self.names.len() * 132);
        out.extend_from_slice(b"RSTB");
        out.extend_from_slice(&(self.crc.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.names.len() as u32).to_le_bytes());
        for (crc, size) in &self.crc {
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
        }
        for (name, size) in &self.names {
            out.extend_from_slice(name);
            out.extend_from_slice(&size.to_le_bytes());
        }
        out
    }

    fn name_index(&self, name: &str) -> Option<usize> {
        let bytes = name.as_bytes();
        self.names.iter().position(|(n, _)| {
            let len = n.iter().position(|&b| b == 0).unwrap_or(128);
            &n[..len] == bytes
        })
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        if let Some(i) = self.name_index(name) {
            return Some(self.names[i].1);
        }
        let crc = crc32(name.as_bytes());
        self.crc
            .binary_search_by_key(&crc, |(c, _)| *c)
            .ok()
            .map(|i| self.crc[i].1)
    }

    pub fn set(&mut self, name: &str, value: u32) {
        if let Some(i) = self.name_index(name) {
            self.names[i].1 = value;
            return;
        }
        let crc = crc32(name.as_bytes());
        match self.crc.binary_search_by_key(&crc, |(c, _)| *c) {
            Ok(i) => self.crc[i].1 = value,
            Err(i) => self.crc.insert(i, (crc, value)),
        }
    }

    pub fn raise(&mut self, name: &str, value: u32) -> bool {
        if self.get(name).map(|v| v < value).unwrap_or(true) {
            self.set(name, value);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy)]
enum RstbRule {
    Over(u32),
    Line(f64, u32),
    Scale(f64),
}

const RSTB_BASE: u32 = 448;

const RSTB_RULES: &[(&str, RstbRule)] = &[
    ("Tex.bfres", RstbRule::Over(8192)),
    ("bitemico", RstbRule::Over(8192)),
    ("bmaptex", RstbRule::Over(8192)),
    ("bstftex", RstbRule::Over(8192)),
    ("breviewtex", RstbRule::Over(8192)),
    ("bmapopen", RstbRule::Over(8192)),
    ("jpg", RstbRule::Over(8192)),
    ("bfarc", RstbRule::Over(8192)),
    ("blarc", RstbRule::Over(4096)),
    ("esetlist", RstbRule::Over(16736)),
    ("genvb", RstbRule::Over(6624)),
    ("hktmrb", RstbRule::Over(1344)),
    ("bars", RstbRule::Over(840)),
    ("bactorpack", RstbRule::Over(576)),
    ("beventpack", RstbRule::Over(512)),
    ("stats", RstbRule::Over(512)),
    ("sarc", RstbRule::Over(552)),
    ("byml", RstbRule::Over(456)),
    ("bxml", RstbRule::Over(2424)),
    ("baischedule", RstbRule::Over(1088)),
    ("bumii", RstbRule::Over(1088)),
    ("bdemo", RstbRule::Over(3232)),
    ("bawareness", RstbRule::Over(3264)),
    ("blod", RstbRule::Over(1344)),
    ("brgconfig", RstbRule::Over(17504)),
    ("brgcon", RstbRule::Over(7096)),
    ("mubin", RstbRule::Over(9696)),
    ("baniminfo", RstbRule::Over(1160)),
    ("hkcl", RstbRule::Line(2.113, 0)),
    ("brgbw", RstbRule::Line(5.472, 1088)),
    ("batcl", RstbRule::Line(4.343, 1440)),
    ("batcllist", RstbRule::Line(3.52, 1408)),
    ("bbonectrl", RstbRule::Line(4.33, 2656)),
    ("bcamanim", RstbRule::Line(1.311, 9024)),
    ("brgconfiglist", RstbRule::Line(3.359, 2360)),
    ("bmscdef", RstbRule::Line(4.606, 0)),
    ("bdmgparam", RstbRule::Line(5.628, 0)),
    ("bassetting", RstbRule::Line(5.93, 0)),
    ("bfres", RstbRule::Scale(1.04)),
    ("bas", RstbRule::Scale(1.012)),
    ("hknm2", RstbRule::Scale(1.015)),
];

fn rstb_kind(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    if file.contains(".Tex") && (file.ends_with("bfres")) {
        return "Tex.bfres";
    }
    let ext = file.rsplit('.').next().unwrap_or("");
    match ext.strip_prefix('s') {
        Some(rest) if ext != "sarc" && ext != "stats" && ext != "stera" => rest,
        _ => ext,
    }
}

pub fn rstb_estimate(data: &[u8], name: &str) -> Option<u32> {
    let size = if data.len() >= 8 && data.starts_with(b"Yaz0") {
        u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as u64
    } else {
        data.len() as u64
    };
    let rounded = (size + 31) & !31;
    let parsed = rstb::calc::estimate_from_slice_and_name(data, name, rstb::Endian::Little).map(u64::from);
    let rule = RSTB_RULES.iter().find(|(k, _)| *k == rstb_kind(name)).map(|(_, r)| *r);
    let ruled = match rule {
        Some(RstbRule::Over(o)) => Some(rounded + o as u64),
        Some(RstbRule::Line(a, b)) => Some((rounded as f64 * a).ceil() as u64 + b as u64),
        Some(RstbRule::Scale(k)) => parsed.map(|p| (p as f64 * k).ceil() as u64),
        None => None,
    };
    let floor = rounded + RSTB_BASE as u64;
    let best = [parsed.map(|p| p + 32), ruled, Some(floor)].into_iter().flatten().max()?;
    Some(((best + 31) & !31).min(u32::MAX as u64) as u32)
}

fn estimate_into(
    name: &str,
    data: &[u8],
    prefix: &str,
    wanted: &std::collections::HashSet<String>,
    out: &mut std::collections::HashMap<String, u32>,
) {
    let decoded;
    let data = if data.starts_with(b"Yaz0") {
        match roead::yaz0::decompress(data) {
            Ok(d) => {
                decoded = d;
                decoded.as_slice()
            }
            Err(_) => return,
        }
    } else {
        data
    };
    let key = format!("{prefix}{name}");
    if wanted.contains(&key)
        && let Some(value) = rstb_estimate(data, name) {
            let slot = out.entry(key).or_insert(0);
            *slot = (*slot).max(value);
        }
    if data.len() > 4 && &data[..4] == b"SARC"
        && let Ok(sarc) = roead::sarc::Sarc::new(data) {
            for file in sarc.files() {
                if let Some(inner) = file.name {
                    estimate_into(&rstb_name(inner), file.data, prefix, wanted, out);
                }
            }
        }
}

pub fn deployed_rstb_estimates(
    roots: &[(PathBuf, &str)],
    wanted: &std::collections::HashSet<String>,
) -> std::collections::HashMap<String, u32> {
    use rayon::prelude::*;
    let mut files = Vec::new();
    for (root, prefix) in roots {
        let mut found = Vec::new();
        walk_files(root, root, &mut found);
        files.extend(found.into_iter().map(|(rel, _)| (root.clone(), *prefix, rel)));
    }
    files
        .par_iter()
        .filter(|(_, prefix, rel)| {
            let name = format!("{prefix}{}", rstb_name(rel));
            wanted.contains(&name) || matches!(rstb_kind(rel), "pack" | "bactorpack" | "beventpack" | "sarc" | "bmodelsh" | "blarc" | "bfarc")
        })
        .fold(std::collections::HashMap::new, |mut out, (root, prefix, rel)| {
            if let Ok(data) = std::fs::read(root.join(rel)) {
                estimate_into(&rstb_name(rel), &data, prefix, wanted, &mut out);
            }
            out
        })
        .reduce(std::collections::HashMap::new, |mut a, b| {
            for (k, v) in b {
                let slot = a.entry(k).or_insert(0);
                *slot = (*slot).max(v);
            }
            a
        })
}

pub fn game_rstb(romfs: &Path) -> Option<RawRstb> {
    RawRstb::read(&romfs.join(RSTB_PATH))
}

pub fn adjust_rstb(table: &mut RawRstb, vanilla: &RawRstb, romfs: &Path) -> usize {
    let mut raised = 0;

    const HUD_LAYOUTS_MAX: u32 = 4 * (0x80 + 0x2000);
    if let Some(base) = vanilla.get("Layout/Common.blarc") {
        let grow = ((HUD_LAYOUTS_MAX + 31) & !31) + 32;
        raised += table.raise("Layout/Common.blarc", base.saturating_add(grow)) as usize;
    }

    let voice_dir = romfs.join("Voice");
    let mut banks: Vec<(String, String, u64)> = Vec::new();
    if let Ok(langs) = std::fs::read_dir(&voice_dir) {
        for lang in langs.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
            let lang_name = lang.file_name().to_string_lossy().to_string();
            if lang_name.eq_ignore_ascii_case("Common") {
                continue;
            }
            let mut files = Vec::new();
            walk_files(&lang.path(), &lang.path(), &mut files);
            for (file, size) in files {
                if file.ends_with(".bars") {
                    banks.push((lang_name.clone(), file, size));
                }
            }
        }
    }
    let mut biggest: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
    for (_, file, size) in &banks {
        let entry = biggest.entry(file.as_str()).or_insert(0);
        *entry = (*entry).max(*size);
    }
    for (lang, file, size) in &banks {
        let max = biggest.get(file.as_str()).copied().unwrap_or(*size);
        if max <= *size {
            continue;
        }
        let name = format!("Voice/{lang}/{file}");
        if let Some(base) = vanilla.get(&name) {
            let grow = (((max - size) + 31) & !31) as u32 + 32;
            raised += table.raise(&name, base.saturating_add(grow)) as usize;
        }
    }

    let merged_dlc = romfs.join("System").join("AocVersion.txt").is_file()
        || romfs.join("Pack").join("AocMainField.pack").is_file();
    if merged_dlc {
        let mut files = Vec::new();
        walk_files(romfs, romfs, &mut files);
        for (file, _) in files {
            let name = rstb_name(&file);
            let aoc = format!("Aoc/0010/{name}");
            if let (Some(base), Some(dlc)) = (vanilla.get(&name), vanilla.get(&aoc))
                && dlc > base {
                    raised += table.raise(&name, dlc) as usize;
                }
        }
    }
    raised
}

pub fn overlay_rstb(
    state: Option<RawRstb>,
    vanilla: &RawRstb,
    romfs: &Path,
    updates: impl IntoIterator<Item = (String, Option<u32>, Option<(u32, u32)>)>,
) -> (RawRstb, Option<RawRstb>) {
    let mut state = state.unwrap_or_else(|| vanilla.clone());
    let mut raised = 0usize;
    for (name, size, sizes) in updates {

        let scaled = match (vanilla.get(&name), sizes) {
            (Some(entry), Some((built, game))) if built > game && game > 0 => {
                let entry = entry as u64;
                let grown = entry + (entry * (built - game) as u64).div_ceil(game as u64);
                Some((((grown + 31) & !31) + 32).min(u32::MAX as u64) as u32)
            }
            _ => None,
        };

        let Some(value) = size.into_iter().chain(scaled).max() else { continue };
        if state.raise(&name, value) {
            raised += 1;
        }
    }
    log::info!("ResourceSizeTable: {raised} entries raised for the mods");
    if state == *vanilla {
        log::info!("No mod needs a bigger ResourceSizeTable entry: Savage Wind keeps the game's own table");
        return (state, None);
    }
    let mut deployed = state.clone();
    let fixes = adjust_rstb(&mut deployed, vanilla, romfs);
    log::info!("ResourceSizeTable: Savage Wind's own adjustments applied ({fixes} entries)");
    (state, Some(deployed))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayIssue {

    pub file: String,

    pub missing_dir: String,
}

pub fn validate_overlay(overlay: &Path, game: &GameFiles) -> Vec<OverlayIssue> {
    fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>, skip_aoc: bool) {
        let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            if skip_aoc && rel.as_os_str().is_empty() && name.to_string_lossy().eq_ignore_ascii_case("aoc") {
                continue;
            }
            let rel_path = rel.join(&name);
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(root, &rel_path, out, skip_aoc),
                Ok(_) => out.push(rel_path),
                Err(_) => {}
            }
        }
    }
    let mut issues = Vec::new();
    let mut reported: BTreeSet<String> = BTreeSet::new();

    let mut parts = vec![(overlay.to_path_buf(), "", false)];
    if !game.all_aoc.is_empty() {
        parts.push((overlay.join("aoc").join("0010"), "aoc/0010/", true));
    }
    for (dir, prefix, is_aoc) in parts {
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(&dir, Path::new(""), &mut files, !is_aoc);
        for file in files {
            let Some(parent) = file.parent() else { continue };
            if parent.as_os_str().is_empty() {
                continue;
            }

            let present = if is_aoc {
                game.all_aoc.iter().any(|a| a.join(parent).is_dir())
            } else {
                game.romfs.join(parent).is_dir()
            };
            if present {
                continue;
            }
            let key = format!("{prefix}{}", file.to_string_lossy().replace('\\', "/"));
            if reported.insert(key.clone()) {
                issues.push(OverlayIssue {
                    file: key,
                    missing_dir: format!("{prefix}{}", parent.to_string_lossy().replace('\\', "/")),
                });
            }
        }
    }
    issues
}
