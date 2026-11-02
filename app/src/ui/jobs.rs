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
};
use swmm_modpack::{
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

pub fn open_mod(core: &Manager, path: &Path, meta: Option<Meta>) -> Result<Message> {
    log::info!("Opening mod at {}", path.display());
    if is_probably_a_bnp(path) {
        let mod_ = convert_bnp(core, path).context("Failed to convert the BNP mod")?;
        return Ok(Message::HandleMod(Mod::from_reader(
            ModReader::open_peek(mod_, vec![]).context("Failed to open converted mod")?,
        )));
    }
    let direct = if meta.is_none() {
        ModReader::open_peek(path, vec![])
    } else {
        Err(anyhow_ext::anyhow!("Converting with provided details"))
    };
    let mod_ = match direct {
        Ok(reader) => Mod::from_reader(reader),
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
    Ok(Message::HandleMod(mod_))
}

pub fn apply_changes(core: &Manager, mods: Vec<Mod>, dirty: Option<Manifest>) -> Result<Message> {
    let mod_manager = core.mod_manager();
    log::info!("Applying pending changes to mod configuration");
    if !mods.is_empty() {
        log::info!("Updating mod states");
        mods.iter()
            .try_for_each(|m| -> Result<()> {
                let mod_ = mod_manager
                    .all_mods()
                    .find(|m2| m2.hash() == m.hash())
                    .unwrap();
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
        let order = mods.iter().map(|m| m.hash()).collect();
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
    let settings = core.settings();
    let child = swmm_core::savage_wind::launch(&settings)?;
    log::info!("Savage Wind started (process {})", child.id());
    Ok(Message::Launched)
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
    std::thread::spawn(|| match INTERFACE.claim() {
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
        let core = swmm_core::core::Manager::init().unwrap();
        super::apply_changes(&core, vec![], None).unwrap();
    }
}
