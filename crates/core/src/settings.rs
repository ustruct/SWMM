#![allow(clippy::unwrap_used)]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DefaultOnError};
use smartstring::alias::String;
use uk_content::{constants::Language, prelude::Endian};
use swmm_i18n::LocLang;
use uk_reader::ResourceReader;

use crate::util;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    #[default]
    Switch,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Switch => "Switch",
        }
        .fmt(f)
    }
}

impl std::str::FromStr for Platform {
    type Err = anyhow_ext::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "switch" | "nx" => Ok(Self::Switch),
            _ => anyhow_ext::bail!("Only Switch is supported"),
        }
    }
}

impl From<roead::Endian> for Platform {
    fn from(_: roead::Endian) -> Self {
        Self::Switch
    }
}

impl From<Platform> for roead::Endian {
    fn from(_: Platform) -> Self {
        Self::Little
    }
}

impl From<uk_content::prelude::Endian> for Platform {
    fn from(_: uk_content::prelude::Endian) -> Self {
        Self::Switch
    }
}

impl From<Platform> for uk_content::prelude::Endian {
    fn from(_: Platform) -> Self {
        Self::Little
    }
}

impl From<rstb::Endian> for Platform {
    fn from(_: rstb::Endian) -> Self {
        Self::Switch
    }
}

impl From<Platform> for rstb::Endian {
    fn from(_: Platform) -> Self {
        Self::Little
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployConfig {
    pub output: PathBuf,
    pub method: DeployMethod,
    pub auto: bool,
    #[serde(default)]
    pub cemu_rules: bool,
    #[serde(default)]
    pub executable: Option<std::string::String>,
    #[serde(default)]
    pub layout: DeployLayout,
}

impl DeployConfig {
    pub fn final_output_paths(&self, _endian: Endian) -> (PathBuf, PathBuf) {
        match self.layout {
            DeployLayout::SavageWind => (self.output.clone(), self.output.join("aoc").join("0010")),
            DeployLayout::WithoutName => (
                self.output.join("01007EF00011E000").join("romfs"),
                self.output.join("01007EF00011F001").join("romfs"),
            ),
            DeployLayout::WithName => (
                self.output
                    .join("01007EF00011E000")
                    .join("BreathOfTheWild_SWMM")
                    .join("romfs"),
                self.output
                    .join("01007EF00011F001")
                    .join("BreathOfTheWild_SWMM")
                    .join("romfs"),
            ),
        }
    }

    #[inline]
    pub fn overlay_dir(&self) -> PathBuf {
        self.final_output_paths(Endian::Little).0
    }

    #[inline]
    pub fn effective_method(&self) -> DeployMethod {
        if self.layout == DeployLayout::SavageWind && self.method == DeployMethod::Symlink {
            DeployMethod::HardLink
        } else {
            self.method
        }
    }
}

impl Default for DeployConfig {
    fn default() -> Self {
        DeployConfig {
            output: "".into(),
            method: DeployMethod::HardLink,
            auto: true,
            cemu_rules: false,
            executable: None,
            layout: DeployLayout::SavageWind,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployMethod {
    Copy,
    HardLink,
    Symlink,
}

impl DeployMethod {
    #[inline(always)]
    pub fn name(&self) -> &'static str {
        match self {
            DeployMethod::Copy => "Settings_Platform_Deploy_Method_Copy",
            DeployMethod::HardLink => "Settings_Platform_Deploy_Method_HardLink",
            DeployMethod::Symlink => "Settings_Platform_Deploy_Method_Symlink",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DeployLayout {
    WithoutName,
    WithName,

    #[default]
    SavageWind,
}

impl DeployLayout {
    #[inline(always)]
    pub fn name(&self) -> &str {
        match self {
            DeployLayout::WithoutName => "SD Card",
            DeployLayout::WithName => "Emulator",
            DeployLayout::SavageWind => "Savage Wind overlay",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformSettings {
    pub language: Language,
    pub profile: String,
    pub dump: Arc<ResourceReader>,
    pub deploy_config: Option<DeployConfig>,
}

fn replace_in_file(path: &Path, from: &str, to: &str) {
    if let Ok(text) = std::fs::read_to_string(path) {
        if text.contains(from) {
            let _ = std::fs::write(path, text.replace(from, to));
        }
    }
}

fn move_into(src: &Path, dst: &Path) {
    if !dst.exists() {
        if std::fs::rename(src, dst).is_ok() {
            return;
        }
    }
    if src.is_dir() && dst.is_dir() {
        if let Ok(entries) = std::fs::read_dir(src) {
            for entry in entries.flatten() {
                move_into(&entry.path(), &dst.join(entry.file_name()));
            }
        }
        let _ = std::fs::remove_dir(src);
    }
}

fn flatten_switch_folder(storage: &Path) {
    let nx = storage.join("nx");
    if !nx.is_dir() {
        return;
    }
    log::info!("Moving files from {} to {}", nx.display(), storage.display());
    if let Ok(entries) = std::fs::read_dir(&nx) {
        for entry in entries.flatten() {
            move_into(&entry.path(), &storage.join(entry.file_name()));
        }
    }
    let _ = std::fs::remove_dir(&nx);
    let sep = std::path::MAIN_SEPARATOR;
    let from = format!("{}{sep}", nx.display());
    let to = format!("{}{sep}", storage.display());
    let config = Settings::config_dir();
    replace_in_file(&config.join("settings.yml"), &from, &to);
    replace_in_file(&config.join("loader.ini"), &from, &to);
    replace_in_file(&storage.join("pending.yml"), &from, &to);
    if let Ok(entries) = std::fs::read_dir(storage.join("profiles")) {
        for entry in entries.flatten() {
            replace_in_file(&entry.path().join("profile.yml"), &from, &to);
        }
    }
}

fn migrate_legacy_folders() {
    let legacy = "SavageWindModManager";
    let (Some(config), Some(local)) = (dirs2::config_dir(), dirs2::data_local_dir()) else {
        return;
    };
    let old_config = config.join(legacy);
    let new_config = config.join("SWMM");
    if old_config.is_dir() && !new_config.exists() && std::fs::rename(&old_config, &new_config).is_err() {
        return;
    }
    let old_local = local.join(legacy);
    let new_local = local.join("SWMM");
    if old_local.is_dir() && !new_local.exists() && std::fs::rename(&old_local, &new_local).is_err() {
        return;
    }
    let from = old_local.display().to_string();
    let to = new_local.display().to_string();
    replace_in_file(&new_config.join("settings.yml"), &from, &to);
    for platform in ["nx"] {
        if let Ok(entries) = std::fs::read_dir(new_local.join(platform).join("profiles")) {
            for entry in entries.flatten() {
                replace_in_file(&entry.path().join("profile.yml"), &from, &to);
            }
        }
    }
}

#[inline]
fn default_storage() -> PathBuf {
    if std::env::args().any(|a| a == "--portable") {
        std::env::current_exe().unwrap().with_file_name("data")
    } else {
        dirs2::data_local_dir().unwrap().join("SWMM")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SavageWindSettings {
    pub executable: Option<PathBuf>,
    pub standalone_mods: bool,
    pub browse_gamebanana: bool,
}

impl Default for SavageWindSettings {
    fn default() -> Self {
        Self {
            executable: None,
            standalone_mods: true,
            browse_gamebanana: true,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UpdatePreference {
    None,
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[serde_as]
pub struct Settings {
    pub current_mode: Platform,
    pub system_7z: bool,
    #[serde(default = "default_storage")]
    pub storage_dir: PathBuf,
    #[serde(deserialize_with = "serde_with::As::<DefaultOnError>::deserialize")]
    pub check_updates: UpdatePreference,
    pub show_changelog: bool,
    pub last_version: Option<String>,
    pub switch_config: Option<PlatformSettings>,
    pub lang: LocLang,
    pub savage_wind: SavageWindSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            current_mode: Platform::Switch,
            system_7z: true,
            storage_dir: default_storage(),
            switch_config: None,
            check_updates: UpdatePreference::None,
            show_changelog: true,
            last_version: None,
            lang: LocLang::English,
            savage_wind: SavageWindSettings::default(),
        }
    }
}

impl Settings {
    pub fn path() -> &'static Path {
        static PATH: LazyLock<PathBuf> =
            LazyLock::new(|| Settings::config_dir().join("settings.yml"));
        PATH.as_path()
    }

    pub fn config_dir() -> &'static Path {
        static PATH: LazyLock<PathBuf> = LazyLock::new(|| {
            if std::env::args().any(|a| a == "--portable") {
                std::env::current_exe()
                    .expect("No current executable???")
                    .parent()
                    .expect("Executable has no parent???")
                    .join("config")
            } else {
                migrate_legacy_folders();
                dirs2::config_dir().unwrap().join("SWMM")
            }
        });
        PATH.as_path()
    }

    pub fn load() -> Arc<RwLock<Settings>> {
        let storage = Settings::read(Self::path())
            .map(|s| s.storage_dir)
            .unwrap_or_else(|_| default_storage());
        flatten_switch_folder(&storage);
        Arc::new(RwLock::new(match Settings::read(Self::path()) {
            Ok(settings) => {
                log::debug!("{:#?}", settings);
                crate::util::USE_SZ.store(settings.system_7z, std::sync::atomic::Ordering::Release);
                settings
            }
            Err(e) => {
                log::error!("Failed to read settings file:\n{}", e);
                log::info!("Loading default settings instead");
                Settings::default()
            }
        }))
    }

    pub fn reload(&mut self) {
        *self = match Settings::read(Self::path()) {
            Ok(settings) => {
                log::debug!("{:#?}", settings);
                settings
            }
            Err(e) => {
                log::error!("Failed to read settings file:\n{}", e);
                log::info!("Loading default settings instead");
                Settings::default()
            }
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        Ok(serde_yaml::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn apply(&mut self, apply_fn: impl Fn(&mut Self)) -> Result<()> {
        apply_fn(self);
        self.save().context("Failed to save settings file")?;
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        if !Self::path().parent().unwrap().exists() {
            fs::create_dir_all(Self::path().parent().unwrap())?;
        }
        log::debug!("Saving settings:\n{:#?}", self);
        let _ = crate::util::USE_SZ.compare_exchange_weak(
            !self.system_7z,
            self.system_7z,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
        fs::write(Self::path(), serde_yaml::to_string(self)?)?;
        log::info!("Settings saved");
        Ok(())
    }

    pub fn wipe_output(&self, endian: Endian) -> Result<()> {
        let (content, aoc) = self
            .switch_config
            .as_ref()
            .context("Savage Wind isn't set up")?
            .deploy_config
            .as_ref()
            .context("No deploy settings")?
            .final_output_paths(endian);
        if util::is_symlink(content.as_ref()) {
            util::remove_symlink(content)?;
        } else if content.exists() {
            util::remove_dir_all(content)?;
        }
        if util::is_symlink(aoc.as_ref()) {
            util::remove_symlink(aoc)?;
        } else if aoc.exists() {
            util::remove_dir_all(aoc)?;
        }
        Ok(())
    }

    #[inline]
    pub fn platform_dir(&self) -> PathBuf {
        self.get_platform_dir(self.current_mode)
    }

    #[inline]
    pub fn get_platform_dir(&self, platform: Platform) -> PathBuf {
        match platform {
            Platform::Switch => self.storage_dir.clone(),
        }
    }

    #[inline]
    pub fn profiles_dir(&self) -> PathBuf {
        self.platform_dir().join("profiles")
    }

    #[inline]
    pub fn profile_dir(&self) -> PathBuf {
        self.platform_dir().join("profiles").join(
            self.platform_config()
                .map(|c| c.profile.as_str())
                .unwrap_or("Default"),
        )
    }

    #[inline]
    pub fn profiles(&self) -> impl Iterator<Item = String> {
        fs::read_dir(self.profiles_dir())
            .into_iter()
            .flat_map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .filter_map(|entry| {
                        entry
                            .path()
                            .is_dir()
                            .then(|| entry.file_name().to_string_lossy().into())
                    })
            })
    }

    #[inline]
    pub fn mods_dir(&self) -> PathBuf {
        self.platform_dir().join("mods")
    }

    #[inline]
    pub fn dump(&self) -> Option<Arc<ResourceReader>> {
        match self.current_mode {
            Platform::Switch => self.switch_config.as_ref().map(|c| c.dump.clone()),
        }
    }

    #[inline(always)]
    pub fn platform_config(&self) -> Option<&PlatformSettings> {
        match self.current_mode {
            Platform::Switch => self.switch_config.as_ref(),
        }
    }

    #[inline(always)]
    pub fn platform_config_mut(&mut self) -> Option<&mut PlatformSettings> {
        match self.current_mode {
            Platform::Switch => self.switch_config.as_mut(),
        }
    }

    #[inline]
    pub fn merged_dir(&self) -> PathBuf {
        self.profile_dir().join("merged")
    }

    #[inline]
    pub fn deploy_dir(&self) -> Option<&Path> {
        let config = self.platform_config();
        config
            .and_then(|c| c.deploy_config.as_ref())
            .map(|c| c.output.as_ref())
    }

    #[inline]
    pub fn state_file(&self) -> PathBuf {
        Self::config_dir().join("ui.json")
    }

    #[inline]
    pub fn projects_dir(&self) -> PathBuf {
        self.storage_dir.join("projects")
    }
}
