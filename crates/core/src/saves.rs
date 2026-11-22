use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow_ext::{Context, Result};
use fs_err as fs;

use crate::{settings::Settings, util};

const PROFILES_DIR: &str = "save-profiles";
const ACTIVE_FILE: &str = "save_profile.txt";
const SETTINGS_FILE: &str = "savagewind.ini";
const SWMM_LIST: &str = "swmm-backups.txt";
const SWMM_KEEP: usize = 10;
const IGNORED: &[&str] = &["option.sav"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    pub name: String,
    pub automatic: bool,
    pub by_swmm: bool,
    pub size: u64,
}

pub fn save_dir(settings: &Settings) -> Option<PathBuf> {
    let root = crate::savage_wind::data_root(&settings.savage_wind)?;
    let profile = fs::read_to_string(root.join(ACTIVE_FILE))
        .ok()
        .map(|s| s.lines().next().unwrap_or_default().trim().to_string())
        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("default"));
    if let Some(name) = profile {
        let dir = root.join(PROFILES_DIR).join(&name).join("save");
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let custom = fs::read_to_string(root.join(SETTINGS_FILE)).ok().and_then(|text| {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "save_dir").then(|| value.trim().to_string())
        })
    });
    match custom.filter(|v| !v.is_empty() && v != "default") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            Some(if dir.is_absolute() { dir } else { root.join(dir) })
        }
        None => Some(root.join("save")),
    }
}

pub fn backup_dir(save: &Path) -> PathBuf {
    let mut name = save.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "save".into());
    name.push("-backups");
    save.with_file_name(name)
}

fn dir_size(dir: &Path) -> u64 {
    jwalk::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn newest_change(dir: &Path) -> Option<u64> {
    jwalk::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !IGNORED.iter().any(|i| e.file_name().to_string_lossy().eq_ignore_ascii_case(i)))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
}

fn swmm_list(backups: &Path) -> Vec<String> {
    fs::read_to_string(backups.join(SWMM_LIST))
        .map(|s| s.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn list(save: &Path) -> Vec<Backup> {
    let backups = backup_dir(save);
    let ours = swmm_list(&backups);
    let mut found: Vec<Backup> = fs::read_dir(&backups)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let automatic = name.starts_with("auto-");
            (automatic || name.starts_with("save-")).then(|| Backup {
                size: dir_size(&e.path()),
                by_swmm: ours.contains(&name),
                path: e.path(),
                name,
                automatic,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        let key = |b: &Backup| b.name.split_once('-').map(|(_, s)| s.to_string()).unwrap_or_default();
        key(b).cmp(&key(a))
    });
    found
}

pub fn backup(save: &Path, only_if_changed: bool) -> Result<Option<PathBuf>> {
    if !save.is_dir() || fs::read_dir(save)?.next().is_none() {
        return Ok(None);
    }
    let backups = backup_dir(save);
    fs::create_dir_all(&backups)?;
    let changed = newest_change(save).unwrap_or_default();
    let last_file = backups.join("swmm-last.txt");
    if only_if_changed {
        let last: u64 = fs::read_to_string(&last_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if last >= changed {
            log::info!("Saves unchanged since the last backup");
            return Ok(None);
        }
    }
    let base = format!("save-{}", stamp());
    let mut name = base.clone();
    let mut k = 2;
    while backups.join(&name).exists() {
        name = format!("{base}-{k}");
        k += 1;
    }
    let dest = backups.join(&name);
    let staging = backups.join(format!(".{name}.partial"));
    if staging.exists() {
        util::remove_dir_all(&staging)?;
    }
    util::copy_dir(save, &staging).with_context(|| format!("Failed to copy the saves to {}", staging.display()))?;
    fs::rename(&staging, &dest)?;
    fs::write(&last_file, changed.to_string())?;
    let mut ours = swmm_list(&backups);
    ours.push(name);
    while ours.len() > SWMM_KEEP {
        let old = ours.remove(0);
        let path = backups.join(&old);
        if path.is_dir() {
            log::info!("Removing old SWMM save backup {}", path.display());
            util::remove_dir_all(&path)?;
        }
    }
    fs::write(backups.join(SWMM_LIST), ours.join("\n") + "\n")?;
    log::info!("Saves backed up to {}", dest.display());
    Ok(Some(dest))
}

pub fn restore(save: &Path, from: &Path) -> Result<()> {
    if !from.is_dir() {
        anyhow_ext::bail!("The backup {} no longer exists", from.display());
    }
    backup(save, false).context("Failed to back up the current saves before restoring")?;
    let staging = save.with_file_name(".swmm-restore");
    if staging.exists() {
        util::remove_dir_all(&staging)?;
    }
    util::copy_dir(from, &staging)?;
    if save.exists() {
        util::remove_dir_all(save)?;
    }
    fs::rename(&staging, save)?;
    log::info!("Restored the saves from {}", from.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_restore() {
        let root = tempfile::tempdir().unwrap();
        let save = root.path().join("save");
        std::fs::create_dir_all(save.join("slot0")).unwrap();
        std::fs::write(save.join("slot0/game_data.sav"), b"one").unwrap();
        let first = backup(&save, true).unwrap().expect("first backup");
        assert!(backup(&save, true).unwrap().is_none());
        std::fs::write(save.join("slot0/game_data.sav"), b"two").unwrap();
        restore(&save, &first).unwrap();
        assert_eq!(std::fs::read(save.join("slot0/game_data.sav")).unwrap(), b"one");
        let all = list(&save);
        assert!(all.len() >= 2 && all.iter().all(|b| b.by_swmm));
    }
}
