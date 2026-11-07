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
    EXE_NAMES.iter().any(|n| n.to_lowercase() == name)
        || (name.starts_with("savage_wind") && name.ends_with(".appimage"))
}

fn find_exe_in(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.is_file() && is_savage_wind_exe(p))
}

pub fn detect_executable() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        if let Some(parent) = dir.parent() {
            roots.push(parent.to_path_buf());
        }
        roots.push(dir);
    }
    for dir in [dirs2::download_dir(), dirs2::desktop_dir(), dirs2::document_dir(), dirs2::home_dir()]
        .into_iter()
        .flatten()
    {
        roots.push(dir);
    }
    if let Some(home) = dirs2::home_dir() {
        roots.push(home.join("Games"));
        roots.push(home.join("Applications"));
    }
    if cfg!(windows) {
        for drive in ["C:\\", "D:\\", "E:\\"] {
            let drive = PathBuf::from(drive);
            roots.push(drive.join("Games"));
            roots.push(drive.join("Program Files"));
        }
    }
    for root in roots {
        if let Some(exe) = find_exe_in(&root) {
            return Some(exe);
        }

        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for sub in entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
            if let Some(exe) = find_exe_in(&sub) {
                return Some(exe);
            }
            let name = sub.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name.contains("savage") || name.contains("wind") {
                if let Ok(entries) = std::fs::read_dir(&sub) {
                    for sub2 in entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
                        if let Some(exe) = find_exe_in(&sub2) {
                            return Some(exe);
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn data_root(config: &SavageWindSettings) -> Option<PathBuf> {
    if let Some(root) = std::env::var_os(DATA_ROOT_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(root));
    }
    let exe_dir = config
        .executable
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = exe_dir.as_ref() {
        if dir.join("extracted").is_dir() {
            return Some(dir.clone());
        }
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
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| dirs2::home_dir().map(|h| h.join(".local/share")))
            .map(|d| d.join("savagewind"))
    }
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
        .context("Savage Wind's data folder is unknown. Choose savagewind.exe in Settings first.")?;
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
        .map(|p| !p.as_os_str().is_empty())
        .unwrap_or(false);
    if !has_exe {
        if let Some(exe) = detect_executable() {
            log::info!("Found Savage Wind at {}", exe.display());
            settings.savage_wind.executable = Some(exe);
            result.found_executable = true;
        }
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

pub fn sync_loader(settings: &Settings) -> Result<std::string::String> {
    if cfg!(target_os = "linux") {
        return sync_linux(settings);
    }
    if !cfg!(windows) {
        return Ok("The loader is only used on Windows and Linux".into());
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
    let sw = &settings.savage_wind;
    let motion = sw.cemuhook;
    let camera = sw.camera_sensitivity.clamp(10, 500);
    let server = if sw.cemuhook_server.trim().is_empty() { "127.0.0.1:26760" } else { sw.cemuhook_server.trim() };
    let config = format!(
        "enabled = {}\noverlay = {}\nmotion = {}\nmotion_server = {}\ncamera_sensitivity = {}\n",
        if enabled { 1 } else { 0 },
        overlay.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        match (motion, sw.cemuhook_prefer) {
            (false, _) => "off",
            (true, false) => "auto",
            (true, true) => "prefer",
        },
        server,
        camera
    );
    fs::create_dir_all(Settings::config_dir())?;
    fs::write(loader_config_path(), config)?;
    if dll.exists() && !is_our_loader(&dll) {
        anyhow_ext::bail!("{} in {} doesn't belong to SWMM, so it was left alone", LOADER_DLL, dir.display());
    }
    if !enabled && !motion && camera == 100 {
        if dll.exists() {
            let _ = std::fs::remove_file(&dll);
        }
        return Ok("Mods are off when Savage Wind is started on its own".into());
    }
    if let Some(system) = std::env::var_os("SystemRoot") {
        if !Path::new(&system).join("System32").join("vcruntime140.dll").exists() {
            anyhow_ext::bail!(
                "The Microsoft Visual C++ Redistributable is missing, so the loader can't be installed. Get it \
                 from https://aka.ms/vs/17/release/vc_redist.x64.exe"
            );
        }
    }
    let build = loader_build()
        .with_context(|| format!("{LOADER_BUILD} wasn't found next to SWMM. Build it with build-windows.bat."))?;
    let new = fs::read(&build)?;
    if std::fs::read(&dll).map(|old| old != new).unwrap_or(true) {
        if let Err(e) = fs::write(&dll, &new) {
            if !dll.exists() {
                return Err(e.into());
            }
            log::warn!("Could not update {}: {e}", dll.display());
        }
    }
    Ok(format!("Savage Wind loads the mods when started on its own ({})", dll.display()))
}

fn sync_linux(settings: &Settings) -> Result<std::string::String> {
    let overlay = overlay_dir(settings).filter(|d| has_files(d));
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
        is_savage_wind_exe(path) || exe.as_deref().is_some_and(|e| e == path)
    })
    .unwrap_or_else(|e| {
        log::warn!("Couldn't update the Steam shortcuts: {e}");
        0
    });
    Ok(match (&overlay, changed) {
        (None, _) => "Mods are off when Savage Wind is started on its own".into(),
        (Some(_), 0) => "Savage Wind loads the mods when started on its own (after you log out and back in)".into(),
        (Some(_), n) => format!(
            "Savage Wind loads the mods when started on its own. {n} Steam shortcut(s) were updated: restart Steam to apply"
        ),
    })
}

pub fn remove_loader(settings: &Settings) -> Result<()> {
    if cfg!(target_os = "linux") {
        let _ = set_persistent_env(None);
        let _ = Command::new("systemctl").args(["--user", "unset-environment", OVERLAY_ENV]).status();
        let exe = settings.savage_wind.executable.clone();
        let _ = crate::steam::update_shortcuts(OVERLAY_ENV, None, |path| {
            is_savage_wind_exe(path) || exe.as_deref().is_some_and(|e| e == path)
        });
    }
    if let Some(dir) = game_dir(settings) {
        let dll = dir.join(LOADER_DLL);
        if dll.exists() && is_our_loader(&dll) {
            fs::remove_file(&dll)?;
        }
    }
    let config = loader_config_path();
    if config.exists() {
        fs::remove_file(config)?;
    }
    Ok(())
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
    let config = &settings.savage_wind;
    let exe = config
        .executable
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .context("Savage Wind's executable isn't set. Choose it on the Settings page.")?;
    if !exe.exists() {
        anyhow_ext::bail!("Savage Wind's executable was not found at {}", exe.display());
    }
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
            if let (Some(base), Some(dlc)) = (vanilla.get(&name), vanilla.get(&aoc)) {
                if dlc > base {
                    raised += table.raise(&name, dlc) as usize;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rstb_names() {
        assert_eq!(rstb_name("Map/MainField/D-7/D-7_Dynamic.smubin"), "Map/MainField/D-7/D-7_Dynamic.mubin");
        assert_eq!(rstb_name("Pack/Bootup.pack"), "Pack/Bootup.pack");
        assert_eq!(rstb_name("Layout/Common.sarc"), "Layout/Common.sarc");
        assert_eq!(rstb_name("Voice/USen/Demo131_0.bars"), "Voice/USen/Demo131_0.bars");
    }

    #[test]
    fn raw_rstb() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let mut t = RawRstb { crc: vec![], names: vec![] };
        t.set("Actor/Pack/A.bactorpack", 100);
        t.set("Actor/Pack/B.bactorpack", 200);
        assert!(t.raise("Actor/Pack/A.bactorpack", 150));
        assert!(!t.raise("Actor/Pack/B.bactorpack", 150));
        let parsed = RawRstb::parse(&t.to_bytes()).unwrap();
        assert_eq!(parsed, t);
        assert_eq!(parsed.get("Actor/Pack/A.bactorpack"), Some(150));
        let (state, deployed) = overlay_rstb(None, &t, Path::new("/nonexistent"), vec![("Actor/Pack/B.bactorpack".to_string(), Some(10), None)]);
        assert_eq!(state, t);
        assert!(deployed.is_none());
        let (_, deployed) = overlay_rstb(None, &t, Path::new("/nonexistent"), vec![("Actor/Pack/B.bactorpack".to_string(), Some(300), None)]);
        assert_eq!(deployed.unwrap().get("Actor/Pack/B.bactorpack"), Some(300));
    }

    #[test]
    fn exe_names() {
        assert!(is_savage_wind_exe(Path::new("Savage_Wind-0.4.0-alpha.4-x86_64.AppImage")));
        assert!(!is_savage_wind_exe(Path::new("other.exe")));
    }
}
