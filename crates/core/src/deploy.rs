#![allow(clippy::unwrap_used, unstable_name_collisions)]

mod folder;
mod file;
mod pending_log;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use anyhow_ext::{Context, Result};
use dashmap::DashMap;
use fs_err as fs;
use join_str::jstr;
use parking_lot::RwLock;
use rayon::prelude::*;
use roead::yaz0::{compress, decompress};
use rstb::ResourceSizeTable;
use serde::{Deserialize, Serialize};
use smartstring::alias::String;
use swmm_formats::{constants::Language, platform_prefixes};
use swmm_modpack::{
    unpack::{ModReader, ModUnpacker},
    Manifest,
};

use crate::{
    mods,
    settings::{DeployLayout, DeployMethod, Platform, Settings},
    util,
};
use pending_log::PendingLog;

#[derive(Debug, Default, Serialize, Deserialize)]
struct OldPendingLog {
    files:  Manifest,
    delete: Manifest,
}

#[derive(Debug)]
pub struct Manager {
    settings: Weak<RwLock<Settings>>,
    mod_manager: Weak<RwLock<mods::Manager>>,
    pending_log: RwLock<PendingLog>,

}

impl Manager {
    #[inline(always)]
    fn log_path(settings: &Settings) -> PathBuf {
        settings.platform_dir().join("pending.yml")
    }

    pub fn init(
        settings: &Arc<RwLock<Settings>>,
        mod_manager: &Arc<RwLock<mods::Manager>>,
    ) -> Result<Self> {
        log::info!("Initializing deployment manager");
        let pending = match fs::read_to_string(Self::log_path(&settings.read()))
            .map_err(anyhow_ext::Error::from) {
            Ok(text) => {
                match serde_yaml::from_str::<PendingLog>(&text)
                {
                    Ok(log) => {
                        if log.has_some() {
                            log::info!("Pending deployment data found");
                            log::debug!("{:#?}", &log);
                        } else {
                            log::info!("No files pending deployment");
                        }
                        log
                    }
                    Err(_) => {
                        let old_pending = match serde_yaml::from_str::<OldPendingLog>(&text)
                        {
                            Ok(old_log) => {
                                if !old_log.files.is_empty() || !old_log.delete.is_empty() {
                                    log::info!("Pending deployment data found");
                                    log::debug!("{:#?}", &old_log);
                                } else {
                                    log::info!("No files pending deployment");
                                }
                                old_log
                            }
                            Err(e) => {
                                log::warn!("Could not load pending deployment data:\n{}", &e);
                                log::info!("No files pending deployment");
                                Default::default()
                            }
                        };
                        old_pending.try_into().unwrap_or_else(|e| {
                            log::warn!("Could not load pending deployment data:\n{}", &e);
                            log::info!("No files pending deployment");
                            Default::default()
                        })
                    }
                }
            }
            Err(e) => {
                log::warn!("Could not load pending deployment data:\n{}", &e);
                log::info!("No files pending deployment");
                Default::default()
            }
        };
        Ok(Self {
            settings: Arc::downgrade(settings),
            mod_manager: Arc::downgrade(mod_manager),
            pending_log: RwLock::new(pending),
        })
    }

    #[inline]
    pub fn pending(&self) -> bool {
        self.pending_log.read().has_some()
    }

    #[inline]
    pub fn pending_len(&self) -> usize {
        self.pending_log.read().len()
    }

    pub fn reset_pending(&self) -> Result<()> {
        self.pending_log.write().clear();
        let settings = self
            .settings
            .upgrade()
            .expect("YIKES the settings manager is gone");
        let settings = settings.read();
        let source = settings.merged_dir();
        let (content, aoc) = platform_prefixes(settings.current_mode.into());
        let config = settings
            .platform_config()
            .and_then(|c| c.deploy_config.as_ref())
            .context("No deployment config for current platform")?;
        let (dest_content, dest_aoc) = config.final_output_paths(settings.current_mode.into());

        let mut log = PendingLog::try_from((
            source.join(content), source.join(aoc), dest_content, dest_aoc
        ))?;
        if config.layout == DeployLayout::SavageWind {

            log.content_deletes.remove_folder("aoc");
        }
        *self.pending_log.write() = log;

        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        fs::write(
            Self::log_path(&self.settings.upgrade().unwrap().read()),
            serde_yaml::to_string(&self.pending_log.read().clone())?,
        )?;
        Ok(())
    }

    pub fn deploy(&self) -> Result<()> {
        let settings = self
            .settings
            .upgrade()
            .expect("YIKES, the settings manager is gone");
        let settings = settings.read();
        let mut lang = Language::USen;
        let mut profile = String::from("");
        let config = settings
            .platform_config()
            .and_then(|c| {
                lang = c.language;
                profile = c.profile.clone();
                c.deploy_config.as_ref()
            })
            .context("No deployment config for current platform")?;
        log::debug!("Deployment config:\n{:#?}", &config);
        let method = config.effective_method();
        if method != config.method {
            log::info!(
                "The Savage Wind overlay layout can't be symlinked as one folder, deploying with hard links"
            );
        }

        let (content, aoc) = platform_prefixes(settings.current_mode.into());
        let src_content  = settings.merged_dir().join(content);
        let src_aoc = settings.merged_dir().join(aoc);
        let (dest_content, dest_aoc) = config.final_output_paths(settings.current_mode.into());

        if util::is_symlink(&config.output) {
            log::info!("Removing old symlink deployment behavior");
            util::remove_symlink(&config.output)
                .context("Failed to remove old deployment behavior symlink")?;
        }

        if method == DeployMethod::Symlink {
            log::info!("Deploy method is symlink, checking for symlink");

            for (src, dest, type_) in [
                (src_content, dest_content.clone(), "content"),
                (src_aoc, dest_aoc, "aoc")
            ] {
                let (actual_src, actual_dest): (&Path, &Path) = (src.as_ref(), dest.as_ref());
                log::info!("Generating {} links", type_);
                let parent = actual_dest.parent().context("Dest has no parent?")?;
                if actual_src.exists() && !parent.exists() {
                    fs::create_dir_all(parent)
                        .context("Failed to create parents for dest folder")?;
                }
                if actual_dest.exists() && !util::is_symlink(actual_dest) {
                    log::warn!("Removing old stuff from {} deploy folder", type_);
                    util::remove_dir_all(actual_dest)
                        .context("Failed to remove old deployment folder")?;
                }
                if actual_src.exists() && !actual_dest.exists() {
                    log::info!("Creating new symlink for {} folder", type_);
                    util::create_symlink(actual_dest, actual_src)
                        .context("Failed to deploy symlink")?;
                } else if !actual_src.exists() && actual_dest.exists() {
                    log::info!("No {} files, removing link", type_);
                    util::remove_symlink(actual_dest)
                        .context("Failed to remove symlink to non-existent folder")?;
                } else if actual_src.exists() && actual_dest.exists() &&
                    !util::is_symlink_to(actual_dest, actual_src) {
                    log::info!("Refreshing {} link to correct profile", type_);
                    util::remove_symlink(actual_dest)
                        .context("Failed to remove symlink to incorrect profile")?;
                    util::create_symlink(actual_dest, actual_src)
                        .context("Failed to create symlink to correct profile")?;
                } else {
                    log::info!("Symlink exists, no deployment needed")
                }
            }
        } else {
            if util::is_symlink(&dest_content) {
                util::remove_symlink(&dest_content)
                    .context("Failed to remove symlink to old symlinked content")?;
            }
            if util::is_symlink(&dest_aoc) {
                util::remove_symlink(&dest_aoc)
                    .context("Failed to remove symlink to old symlinked dlc")?;
            }
            if !dest_content.exists() {
                std::fs::create_dir_all(&dest_content)?;
            }
            if !dest_aoc.exists() {
                std::fs::create_dir_all(&dest_aoc)?;
            }

            if config.layout == DeployLayout::SavageWind {
                log::info!("Mirroring the merged mods into the Savage Wind overlay");
                let hard_link = method == DeployMethod::HardLink;
                mirror_dir(&src_content, &dest_content, Some("aoc"), hard_link)
                    .context("Failed to update the overlay folder")?;
                mirror_dir(&src_aoc, &dest_aoc, None, hard_link)
                    .context("Failed to update the overlay DLC folder")?;
                log::info!("Deployment complete");
                drop(settings);
                self.pending_log.write().clear();
                self.save()?;
                return Ok(());
            }

            let log = self.pending_log.read();
            log::debug!("Pending log:\n{:#?}", &log);
            log::info!("Deploying by {}", match method {
                DeployMethod::Copy => "copy",
                DeployMethod::HardLink => "hard links",
                DeployMethod::Symlink => unsafe { std::hint::unreachable_unchecked() },
            });
            log::info!("Deploy layout: {}", config.layout.name());

            log.content_deletes.delete(&dest_content)?;
            log.aoc_deletes.delete(&dest_aoc)?;

            match method {
                DeployMethod::Copy => {
                    log.content_copies.copy(&src_content, &dest_content)?;
                    log.aoc_copies.copy(&src_aoc, &dest_aoc)?;
                },
                DeployMethod::HardLink => {
                    log.content_copies.hard_link(&src_content, &dest_content)?;
                    log.aoc_copies.hard_link(&src_aoc, &dest_aoc)?;
                },
                DeployMethod::Symlink => unsafe { std::hint::unreachable_unchecked() },
            }

            log::info!("Deployment complete");
        }
        self.pending_log.write().clear();
        self.save()?;
        Ok(())
    }

    fn handle_orphans(
        &self,
        total_manifest: Manifest,
        manifest: &mut Manifest,
        out_dir: &Path,
        platform: Platform,
    ) -> Result<()> {
        let (orphans_content, orphans_aoc): (Vec<_>, Vec<_>) = (
            manifest
                .content_files
                .difference(&total_manifest.content_files)
                .cloned()
                .collect(),
            manifest
                .aoc_files
                .difference(&total_manifest.aoc_files)
                .cloned()
                .collect(),
        );
        if orphans_content.is_empty() && orphans_aoc.is_empty() {
            log::debug!("No orphans");
            return Ok(());
        }
        log::debug!(
            "Orphans to delete:\n{:#?}\n{:#?}",
            &orphans_content,
            &orphans_aoc
        );
        manifest
            .content_files
            .retain(|f| !orphans_content.contains(f));
        manifest.aoc_files.retain(|f| !orphans_aoc.contains(f));
        let mut log = self.pending_log.write();
        log.extend_deletes(&Manifest {
            content_files: orphans_content.iter().map(|s| s.clone()).collect(),
            aoc_files: orphans_aoc.iter().map(|s| s.clone()).collect(),
        })?;
        let (content, dlc) = platform_prefixes(platform.into());
        for (dir, orphans) in [(content, orphans_content), (dlc, orphans_aoc)] {
            let out_dir = out_dir.join(dir);
            orphans.into_par_iter().try_for_each(|f| -> Result<()> {
                let file = out_dir.join(f.as_str());
                if file.exists() {
                    fs::remove_file(&file)
                        .with_context(|| jstr!("Failed to delete orphan file {f.as_str()}"))?;
                }
                let parent = file.parent().unwrap();
                if std::fs::read_dir(parent)
                    .map(|mut f| f.next().is_none())
                    .unwrap_or(false)
                {
                    util::remove_dir_all(parent).unwrap_or(())
                }
                Ok(())
            })?;
        }
        log::info!("Deleted orphans");
        Ok(())
    }

    fn apply_rstb(
        &self,
        merged: &Path,
        platform: Platform,
        updates: DashMap<String, Option<u32>>,
        dump: &swmm_gamefiles::ResourceReader,
        game_romfs: Option<&Path>,
    ) -> Result<()> {
        static RSTB_PATH: &str = "System/Resource/ResourceSizeTable.product.srsizetable";
        log::debug!("RSTB updates:\n{:#?}", &updates);
        let content = swmm_formats::platform_content(platform.into());
        let table_path = merged.join(content).join(RSTB_PATH);

        if let Some((romfs, vanilla)) =
            game_romfs.and_then(|romfs| crate::savage_wind::game_rstb(romfs).map(|v| (romfs, v)))
        {
            let state_path = merged.join("savagewind-rstb-state");
            let state = crate::savage_wind::RawRstb::read(&state_path);
            let (state, deployed) = crate::savage_wind::overlay_rstb(
                state,
                &vanilla,
                romfs,
                updates.into_iter().map(|(name, size)| {
                    let sizes = swmm_modpack::unpack::RSTB_SIZE_HINTS.get(&name).map(|s| *s);
                    (name.to_string(), size, sizes)
                }),
            );
            fs::create_dir_all(merged)?;
            fs::write(&state_path, state.to_bytes()).context("Failed to save the RSTB state")?;
            match deployed {
                Some(table) => {
                    fs::create_dir_all(table_path.parent().unwrap())?;
                    fs::write(&table_path, compress(table.to_bytes()))
                        .context("Failed to write merged RSTB")?;
                }
                None => {
                    if table_path.exists() {
                        fs::remove_file(&table_path).context("Failed to remove merged RSTB")?;
                    }
                }
            }
            self.pending_log.write().add_rstb()?;
            return Ok(());
        }
        let mut table = if table_path.exists() {
            log::debug!("Updating existing merged RSTB");
            ResourceSizeTable::from_binary(
                decompress(fs::read(&table_path).context("Failed to open merged RSTB")?)
                    .context("Failed to decompress merged RSTB")?,
            )
            .context("Failed to parse merged RSTB")?
        } else {

            let from_dump = dump
                .source()
                .get_base_file_data(Path::new(RSTB_PATH))
                .ok()
                .and_then(|bytes| decompress(bytes).ok())
                .and_then(|bytes| ResourceSizeTable::from_binary(bytes).ok());
            match from_dump {
                Some(table) => {
                    log::debug!("Creating new RSTB from the game's own table");
                    table
                }
                None => {
                    log::debug!("Creating new RSTB");
                    ResourceSizeTable::new_from_stock(platform.into())
                }
            }
        };
        for (canon, size) in updates {
            match size {
                Some(size) => {
                    if table.get(canon.as_str()).map(|s| s < size).unwrap_or(true) {
                        table.set(canon.as_str(), size);
                    }
                }
                None => {
                    table.remove(canon.as_str());
                }
            }
        }
        log::info!("Updated RSTB");
        fs::create_dir_all(table_path.parent().unwrap())?;
        fs::write(table_path, compress(table.to_binary(platform.into())))
            .context("Failed to write merged RSTB")?;
        self.pending_log.write().add_rstb()?;
        Ok(())
    }

    pub fn apply(&self, manifest: Option<Manifest>) -> Result<()> {
        let mod_manager = self
            .mod_manager
            .upgrade()
            .context("YIKES, the mod manager system is gone")?;
        let settings = self
            .settings
            .upgrade()
            .context("YIKES, the settings manager is gone")?;
        let settings = settings.try_read()
            .context("Could not read settings")?;
        let dump = settings
            .dump()
            .context("No dump available for current platform")?;
        let endian = settings.current_mode.into();
        let out_dir = settings.merged_dir();
        let unpacker = if let Some(mut manifest) = manifest {
            log::info!("Manifest provided, applying limited changes");
            let mut total_manifest = Manifest::default();
            let mods = mod_manager
                .read()
                .mods_by_manifest(&manifest)
                .map(|m| {
                    ModReader::open(&m.path, m.enabled_options.clone())
                        .inspect(|m| total_manifest.extend(&m.manifest))
                        .with_context(|| jstr!("Failed to open mod: {&m.meta.name}"))
                })
                .collect::<Result<Vec<_>>>()?;
            self.handle_orphans(
                total_manifest,
                &mut manifest,
                &out_dir,
                settings.current_mode,
            )?;
            log::debug!("Change manifest: {:#?}", &manifest);
            self.pending_log.write().extend_copies(&manifest)?;
            ModUnpacker::new(
                dump.clone(),
                endian,
                settings.platform_config().context("No config for platform")?.language,
                mods,
                out_dir.clone(),
            )
            .with_manifest(manifest)
        } else {
            log::info!("Manifest not provided, remerging all mods");
            let mut total_manifest = Manifest::default();
            let mods = mod_manager
                .read()
                .mods()
                .map(|m| {
                    ModReader::open(&m.path, m.enabled_options.clone())
                        .inspect(|m| total_manifest.extend(&m.manifest))
                        .with_context(|| jstr!("Failed to open mod: {&m.meta.name}"))
                })
                .collect::<Result<Vec<_>>>()?;
            util::remove_dir_all(&out_dir).context("Failed to clear merged folder")?;
            self.pending_log.write().extend_copies(&total_manifest)?;
            ModUnpacker::new(
                dump.clone(),
                endian,
                settings.platform_config().context("No config for platform")?.language,
                mods,
                out_dir.clone(),
            )
        };
        let unpacker = unpacker.with_text_override("LayoutMsg/Title_00", "0005", "SWMM Enabled");
        log::info!("Applying changes");
        let rstb_updates = unpacker.unpack()?;
        let game_romfs = crate::savage_wind::current_game_files(&settings)
            .ok()
            .map(|game| game.romfs);
        self.apply_rstb(
            &out_dir,
            settings.current_mode,
            rstb_updates,
            &dump,
            game_romfs.as_deref(),
        )?;
        self.save()?;
        log::info!("All changed applied successfully");
        Ok(())
    }
}

fn collect_files(root: &Path, skip_top: Option<&str>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if dir == root
                && skip_top
                    .map(|name| path.file_name().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))
                    .unwrap_or(false)
            {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    if let Ok(rel) = path.strip_prefix(root) {
                        files.push(rel.to_path_buf());
                    }
                }
                _ => (),
            }
        }
    }
    files
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    }
}

fn remove_empty_dirs(dir: &Path, keep: bool) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut empty = true;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !remove_empty_dirs(&path, false) {
                empty = false;
            }
        } else {
            empty = false;
        }
    }
    if empty && !keep {
        std::fs::remove_dir(dir).is_ok()
    } else {
        empty
    }
}

fn mirror_dir(src: &Path, dest: &Path, skip_top: Option<&str>, hard_link: bool) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let wanted: std::collections::HashSet<PathBuf> = if src.exists() {
        collect_files(src, skip_top).into_iter().collect()
    } else {
        Default::default()
    };
    let mut removed = 0usize;
    for rel in collect_files(dest, skip_top) {
        let target = dest.join(&rel);
        if !wanted.contains(&rel) || !same_file(&src.join(&rel), &target) {
            std::fs::remove_file(&target)
                .with_context(|| format!("Failed to remove {}", target.display()))?;
            removed += 1;
        }
    }
    let mut placed = 0usize;
    for rel in wanted.iter() {
        let from = src.join(rel);
        let to = dest.join(rel);
        if to.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let linked = hard_link && std::fs::hard_link(&from, &to).is_ok();
        if !linked {
            std::fs::copy(&from, &to)
                .with_context(|| format!("Failed to copy {} to the overlay", rel.display()))?;
        }
        placed += 1;
    }
    remove_empty_dirs(dest, true);
    log::info!(
        "Overlay {}: {} files, {} placed, {} removed",
        dest.display(),
        wanted.len(),
        placed,
        removed
    );
    Ok(())
}
