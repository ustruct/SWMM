#![allow(unstable_name_collisions)]
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use swmm_core::{
    bnp::convert_bnp,
    core::Manager,
    mods::Mod,
    origin::{self, Notes},
};
use uk_mod::{
    pack::ModPacker,
    unpack::ModReader,
    Manifest, Meta,
};

use super::Message;
use crate::INTERFACE;

use swmm_i18n::string_ext::LocString;

fn is_probably_a_bnp(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|e| e.to_str().map(|e| e.to_lowercase()))
        .unwrap_or_default()
        .as_str() {
        "" => {
            let is_a_mod = ["content", "aoc", "01007EF00011E000", "01007EF00011F001", "logs"]
                .into_iter()
                .any(|root| path.join(root).exists());
            let has_meta = path.join("info.json").exists();
            is_a_mod && has_meta
        },
        "bnp" => {
            match sevenz_rust::Archive::open(path) {
                Ok(sz) => {
                    let mut is_a_mod = false;
                    let mut has_meta = false;
                    for entry in sz.files.iter() {
                        is_a_mod = is_a_mod || [
                            "content",
                            "aoc",
                            "01007EF00011E000",
                            "01007EF00011F001",
                            "logs"
                        ]
                            .into_iter()
                            .any(|root| entry.name.ends_with(root));
                        has_meta = has_meta || entry.name.ends_with("info.json");
                        if is_a_mod && has_meta {
                            break;
                        }
                    }
                    is_a_mod && has_meta
                },
                Err(_) => false
            }
        },
        "json" => {
            let is_a_mod = ["content", "aoc", "01007EF00011E000", "01007EF00011F001", "logs"]
                .into_iter()
                .any(|root| path.parent().map(|p| p.join(root).exists()).unwrap_or(false));
            let has_meta = path.file_name().map(|f| f == "info.json").unwrap_or(false);
            is_a_mod && has_meta
        }
        _ => false
    }
}

pub fn is_modpack(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(swmm_core::modpack::EXTENSION))
}

pub fn open_mod(core: &Manager, path: &Path, meta: Option<Meta>) -> Result<Message> {
    if is_modpack(path) {
        return import_modpack(core, path);
    }
    read_mod(core, path, meta).map(|(mod_, _)| Message::HandleMod(mod_))
}

pub fn test_compat(core: &Manager, path: &Path, id: u64, name: &str, file: &str, downloaded: bool) -> Result<Message> {
    log::info!("Testing how {file} converts…");
    if let Err(e) = swmm_core::savage_wind::current_game_files(&core.settings()) {
        if downloaded {
            fs_err::remove_file(path).unwrap_or(());
        }
        return Err(e).context("Compat_NeedsGame".localize().to_string());
    }
    let test = swmm_core::compat::test_file(core, path);
    if downloaded {
        fs_err::remove_file(path).unwrap_or(());
    }
    let entry = test.entry(id, name, file);
    swmm_core::compat::remember(&entry)?;
    log::info!("{}: {:?} {}", entry.name, entry.status, entry.reason);
    Ok(Message::CompatTested(Box::new(entry)))
}

pub fn refresh_pending(core: Arc<Manager>) -> Result<Message> {
    let deploy = core.deploy_manager();
    match deploy.reset_pending().and_then(|_| deploy.save()) {
        Ok(()) => log::info!("{} files differ from the deployed profile", deploy.pending_len()),
        Err(e) => log::warn!("Couldn't compare the profile with the deployed files: {e:?}"),
    }
    Ok(Message::ResetMods(None))
}

pub fn import_modpack(core: &Manager, path: &Path) -> Result<Message> {
    log::info!("Importing the modpack {}", path.display());
    let profile = swmm_core::modpack::import(core, path)?;
    Ok(Message::ModpackImported(profile))
}

pub fn export_modpack(core: &Manager, profile: &str, dest: &Path) -> Result<Message> {
    let count = swmm_core::modpack::export(core, profile, dest)?;
    send_message(Message::Toast(format!("{} ({count})", "SW_Modpack_Saved".localize())));
    Ok(Message::Finished)
}

fn backup_saves_before_launch(settings: &swmm_core::settings::Settings) {
    if !settings.savage_wind.backup_saves {
        return;
    }
    let Some(dir) = swmm_core::saves::save_dir(settings) else {
        return;
    };
    log::info!("Backing up the saves");
    match swmm_core::saves::backup(&dir, true) {
        Ok(Some(path)) => log::info!("Saves backed up to {}", path.display()),
        Ok(None) => {}
        Err(e) => log::warn!("Couldn't back up the saves: {e:?}"),
    }
}

fn wait_until_log_is_quiet(settings: &swmm_core::settings::Settings) {
    let Some(path) = swmm_core::crash::log_path(settings) else {
        return;
    };
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(12 * 3600) {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let age = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .unwrap_or_default();
        if age > std::time::Duration::from_secs(10) {
            break;
        }
    }
}

fn watch_game(mut child: std::process::Child, settings: swmm_core::settings::Settings, mods: Vec<Mod>) {
    let Ok(status) = child.wait() else {
        return;
    };
    let exit_ok = status.success();
    log::info!("Savage Wind's process ended ({status})");
    if exit_ok {
        wait_until_log_is_quiet(&settings);
    }
    if let Some(report) = swmm_core::crash::check(&settings, exit_ok, &mods) {
        log::warn!("Savage Wind stopped unexpectedly: {}", report.reason);
        send_message(Message::GameCrashed(Box::new(report)));
    }
}

pub fn open_downloaded_mod(
    core: &Manager,
    path: &Path,
    source: swmm_core::source::SourceInfo,
) -> Result<Message> {
    let (mod_, notes) = read_mod(core, path, None)?;
    match swmm_core::source::apply(&mod_.path, &source, notes.own_info)
        .and_then(|updated| ModReader::open_peek(updated, vec![]))
    {
        Ok(reader) => Ok(Message::HandleMod(Mod::from_reader(reader))),
        Err(e) => {
            log::warn!("Could not use the GameBanana details for {}: {e:?}", mod_.meta.name);
            Ok(Message::HandleMod(mod_))
        }
    }
}

pub struct Converted {
    pub wiiu: bool,
    pub rebased: bool,
}

pub fn read_for_install(core: &Manager, path: &Path) -> Result<(Mod, Converted)> {
    let (mod_, notes) = read_mod(core, path, None)?;
    Ok((mod_, Converted { wiiu: notes.wiiu, rebased: notes.rebased }))
}

fn read_mod(core: &Manager, path: &Path, meta: Option<Meta>) -> Result<(Mod, Notes)> {
    swmm_core::util::begin_temp_scope();
    origin::reset();
    let result = read_mod_inner(core, path, meta);
    let notes = origin::take();
    if let Ok(mod_) = result.as_ref() {
        let converted = mod_.path.starts_with(swmm_core::util::temp_root());
        if converted && (notes.wiiu || notes.kind.is_some()) {
            if let Err(e) = origin::mark(&mod_.path, notes.marker()) {
                log::warn!("Couldn't note what {} came as: {e:?}", mod_.meta.name);
            }
        }
        if notes.wiiu {
            send_message(Message::Toast(format!("{}: {}", mod_.meta.name, "SW_Notice_WiiU".localize())));
        }
        if notes.rebased {
            send_message(Message::Toast(format!("{}: {}", mod_.meta.name, "SW_Notice_Version".localize())));
        }
    }
    swmm_core::util::end_temp_scope(result.as_ref().ok().map(|m| m.path.as_path()));
    result.map(|mod_| (mod_, notes))
}

fn read_mod_inner(core: &Manager, path: &Path, meta: Option<Meta>) -> Result<Mod> {
    log::info!("Opening mod at {}", path.display());
    if is_probably_a_bnp(path) {
        origin::note_own_info();
        origin::note_kind(origin::Kind::Bnp);
        let mod_ = convert_bnp(core, path).context("Failed to convert the BNP mod")?;
        return Ok(Mod::from_reader(
            ModReader::open_peek(mod_, vec![]).context("Failed to open converted mod")?,
        ));
    }
    let direct = if meta.is_none() {
        ModReader::open_peek(path, vec![])
    } else {
        Err(anyhow_ext::anyhow!("Converting with provided details"))
    };
    let wiiu_package = |r: &ModReader| {
        r.meta.platform == uk_mod::ModPlatform::Specific(uk_content::prelude::Endian::Big)
            && uk_content::prelude::Endian::from(core.settings().current_mode) == uk_content::prelude::Endian::Little
    };
    let mod_ = match direct {
        Ok(reader) if wiiu_package(&reader) => {
            origin::note_own_info();
            origin::note_wiiu();
            log::info!("{} is a package made for Wii U: converting it", path.display());
            let converted = swmm_core::import::convert_any(core, path, None)
                .context("Failed to convert the Wii U package")?;
            Mod::from_reader(ModReader::open_peek(converted, vec![]).context("Failed to open converted mod")?)
        }
        Ok(reader) => {
            origin::note_own_info();
            Mod::from_reader(reader)
        }
        Err(err) => {
            log::info!("Not a packaged mod ({err}), looking inside it");
            let converted = swmm_core::import::convert_any(core, path, meta).with_context(|| {
                format!(
                    "Failed to open mod {}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
                )
            })?;
            Mod::from_reader(
                ModReader::open_peek(converted, vec![]).context("Failed to open converted mod")?,
            )
        }
    };
    Ok(mod_)
}

pub fn apply_changes(core: &Manager, mods: Vec<Mod>, dirty: Option<Manifest>) -> Result<Message> {
    log::info!("Applying pending changes to mod configuration");
    let full_remerge = dirty.is_none();
    if !mods.is_empty() {
        let mod_manager = core.mod_manager();
        log::info!("Updating mod states");
        let installed: Vec<Mod> = mod_manager.all_mods().collect();
        let mods: Vec<&Mod> = mods
            .iter()
            .filter(|m| {
                let found = installed.iter().any(|i| i.hash() == m.hash());
                if !found {
                    log::warn!("{} isn't in this profile anymore, skipping it", m.meta.name);
                }
                found
            })
            .collect();
        mods.iter()
            .try_for_each(|m| -> Result<()> {
                let Some(mod_) = installed.iter().find(|m2| m2.hash() == m.hash()) else {
                    return Ok(());
                };
                if !mod_.state_eq(m) {
                    mod_manager
                        .set_enabled(m.hash(), m.enabled, None)
                        .with_context(|| {
                            format!(
                                "Failed to {} {}",
                                if m.enabled { "enable" } else { "disable" },
                                m.meta.name.as_str()
                            )
                        })?;
                    mod_manager
                        .set_enabled_options(m.hash(), m.enabled_options.clone())
                        .with_context(|| {
                            format!("Failed to update options on {}", m.meta.name.as_str())
                        })?;
                }
                Ok(())
            })
            .context("Failed to update mod state")?;
        log::info!("Updating load order");
        let mut order: Vec<usize> = mods.iter().map(|m| m.hash()).collect();
        let missing: Vec<usize> = installed.iter().map(|m| m.hash()).filter(|h| !order.contains(h)).collect();
        order.extend(missing);
        mod_manager.set_order(order);
        mod_manager
            .save()
            .context("Failed to save mod configuration for current profile")?;
    }
    log::info!("Applying changes");
    let deploy_manager = core.deploy_manager();
    deploy_manager
        .apply(dirty)
        .context("Failed to apply pending mod changes")?;
    if full_remerge {
        if let Err(e) = deploy_manager.reset_pending().and_then(|_| deploy_manager.save()) {
            log::warn!("Couldn't compare the merged mods with the deployed ones: {e:?}");
        }
    }
    if core
        .settings()
        .platform_config()
        .and_then(|c| c.deploy_config.as_ref().map(|c| c.auto))
        .unwrap_or(false)
    {
        log::info!("Deploying changes");
        deploy_manager
            .deploy()
            .context("Failed to deploy update to merged mod(s)")?;
        drop(deploy_manager);
        after_deploy(core);
    }
    log::info!("Done");
    Ok(Message::ResetMods(None))
}

pub static UI_SENDER: std::sync::OnceLock<flume::Sender<Message>> = std::sync::OnceLock::new();

pub fn send_message(message: Message) {
    if let Some(sender) = UI_SENDER.get() {
        sender.send(message).unwrap_or(());
    }
}

pub fn sync_loader(settings: &swmm_core::settings::Settings) {
    match swmm_core::savage_wind::sync_loader(settings) {
        Ok(msg) => log::info!("{msg}"),
        Err(e) => log::warn!("Loader: {e:?}"),
    }
}

pub fn after_deploy(core: &Manager) {
    use swmm_core::savage_wind;
    let settings = core.settings();
    sync_loader(&settings);
    let Some(overlay) = savage_wind::overlay_dir(&settings) else {
        return;
    };
    match savage_wind::current_game_files(&settings) {
        Ok(game) => match savage_wind::sync_game_folders(&settings, &overlay, &game) {
            Ok(0) => (),
            Ok(count) => log::info!("Prepared {count} new game folder(s) so Savage Wind can load every mod file"),
            Err(e) => log::warn!("Could not prepare the game folders for new mod files: {e:?}"),
        },
        Err(e) => log::warn!("Game files unavailable: {e:?}"),
    }
}

pub fn launch_savage_wind(core: &Manager, mods: Vec<Mod>, dirty: Manifest) -> Result<Message> {
    if !dirty.is_empty() {
        log::info!("Applying pending changes before starting Savage Wind");
        apply_changes(core, mods, Some(dirty))?;
    }
    log::info!("Checking the overlay before starting Savage Wind");
    core.deploy_manager()
        .deploy()
        .context("Failed to deploy the mods before starting Savage Wind")?;
    after_deploy(core);
    let settings = core.settings().clone();
    backup_saves_before_launch(&settings);
    let child = swmm_core::savage_wind::launch(&settings)?;
    log::info!("Savage Wind started (process {})", child.id());
    let mods: Vec<Mod> = core.mod_manager().all_mods().collect();
    std::thread::spawn(move || watch_game(child, settings, mods));
    Ok(Message::Launched)
}

pub fn open_game_config(core: &Manager, mods: Vec<Mod>, dirty: Manifest) -> Result<Message> {
    if !dirty.is_empty() {
        log::info!("Applying pending changes before opening Savage Wind's Configuration");
        apply_changes(core, mods, Some(dirty))?;
    }
    if core.deploy_manager().pending() {
        core.deploy_manager()
            .deploy()
            .context("Failed to deploy the mods before opening Savage Wind's Configuration")?;
        after_deploy(core);
    }
    let child = swmm_core::savage_wind::launch_config(&core.settings())?;
    log::info!("Savage Wind's Configuration opened (process {})", child.id());
    Ok(Message::GameConfigOpened)
}

pub fn sw_auto_detect(core: Arc<Manager>) -> Result<Message> {
    let result = {
        let mut settings = core.settings_mut();
        let result = swmm_core::savage_wind::auto_configure(&mut settings);
        settings.save()?;
        result
    };
    core.reload()?;
    let result = result?;
    log::info!("Savage Wind auto-detect: {:?}", result);
    send_message(Message::Toast(if result.changed() {
        "SW_AutoDetect_Done".localize().into()
    } else {
        "SW_AutoDetect_Nothing".localize().into()
    }));
    Ok(Message::HandleSettings)
}

pub struct PackageJob {
    pub source: PathBuf,
    pub dest: PathBuf,
    pub meta: Meta,
}

pub fn package_mod(core: &Manager, builder: PackageJob) -> Result<Message> {
    let Some(dump) = core.settings().dump() else {
        anyhow::bail!("No dump for current platform")
    };
    ModPacker::new(
        builder.source,
        builder.dest,
        Some(builder.meta),
        [dump].into_iter().collect(),
    )
    .context("Failed to initialize mod packager")?
    .pack()
    .context("Failed to package mod")?;
    Ok(Message::Noop)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
enum IpcMessage {
    OpenMod(PathBuf),
}

fn forward_to_ui(message: Message) {
    std::thread::spawn(move || {
        let mut sender = UI_SENDER.get();
        while sender.is_none() {
            std::thread::sleep(std::time::Duration::from_millis(50));
            sender = UI_SENDER.get();
        }
        if let Some(sender) = sender {
            sender.send(message).unwrap_or(());
        }
    });
}

pub fn wait_ipc() {
    std::thread::spawn(|| match (0..20)
        .map(|attempt| {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            INTERFACE.claim()
        })
        .find(|r| r.is_ok())
        .unwrap_or_else(|| INTERFACE.claim())
    {
        Ok(sock) => {
            let mut buf = [0; 4096];
            loop {
                match sock.recv(&mut buf) {
                    Ok(len) => match serde_json::from_slice::<IpcMessage>(&buf[..len]) {
                        Ok(IpcMessage::OpenMod(path)) => forward_to_ui(Message::OpenMod(path)),
                        Err(e) => log::warn!("Ignored a broken message from another SWMM window: {e}"),
                    },
                    Err(e) => log::error!("IPC error: {e}"),
                }
            }
        }
        Err(e) => log::warn!("Another SWMM window may already be open: {e}"),
    });
}

pub fn handle_mod_arg(path: PathBuf) {
    if !path.exists() {
        return;
    }
    if let Ok(client) = INTERFACE.connect() {
        if let Ok(buf) = serde_json::to_vec(&IpcMessage::OpenMod(path.clone())) {
            if client.send(&buf).is_ok() {
                std::process::exit(0);
            }
        }
    }
    log::info!("Opening mod at {} for installation", path.display());
    forward_to_ui(Message::OpenMod(path));
}

#[cfg(test)]
mod tests {
    #[test]
    fn remerge() {
        let Ok(core) = swmm_core::core::Manager::init() else {
            return;
        };
        if core.settings().dump().is_none() {
            return;
        }
        super::apply_changes(&core, vec![], None).unwrap_or_else(|e| panic!("{e:?}"));
    }
}
