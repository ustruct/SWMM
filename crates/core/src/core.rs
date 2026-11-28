use std::sync::Arc;

use anyhow_ext::{Context, Result};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{deploy, mods, settings::Settings};

#[derive(Debug, Clone)]
pub struct Manager {
    mod_manager: Arc<RwLock<mods::Manager>>,
    deploy_manager: Arc<RwLock<deploy::Manager>>,
    settings: Arc<RwLock<Settings>>,
}

impl std::panic::RefUnwindSafe for Manager {}

impl Manager {
    pub fn init() -> Result<Self> {
        let settings = Settings::load();
        crate::util::set_temp_root(&settings.read().storage_dir);
        let mod_manager = Arc::new(RwLock::new(
            mods::Manager::init(&settings).context("Failed to initialize mod manager")?,
        ));
        Ok(Self {
            deploy_manager: Arc::new(RwLock::new(
                deploy::Manager::init(&settings, &mod_manager)
                    .context("Failed to initialize deployment manager")?,
            )),
            mod_manager,
            settings,
        })
    }

    pub fn reload(&self) -> Result<()> {
        self.settings.write().reload();
        crate::util::set_temp_root(&self.settings.read().storage_dir);
        *self.mod_manager.write() =
            mods::Manager::init(&self.settings).context("Failed to initialize mod manager")?;
        *self.deploy_manager.write() = deploy::Manager::init(&self.settings, &self.mod_manager)
            .context("Failed to initialize deployment manager")?;
        Ok(())
    }

    pub fn change_profile(&self, profile: impl AsRef<str>) -> Result<()> {
        let profile = profile.as_ref();
        {
            let mut mods = self.mod_manager.write();
            mods.ensure_profile(profile).unwrap_or(());
            mods.set_profile(profile)?;
        }
        self.remember_profile(profile);
        Ok(())
    }

    fn remember_profile(&self, profile: &str) {
        let mut settings = self.settings.write();
        if let Some(config) = settings.platform_config_mut() {
            config.profile = profile.into();
        }
        if let Err(e) = settings.save() {
            log::warn!("Couldn't save which profile is active: {e:?}");
        }
    }

    pub fn rename_profile(&self, old: &str, new: &str) -> Result<()> {
        let was_current = {
            let mut mods = self.mod_manager.write();
            let was_current = mods.current_profile() == old;
            mods.rename_profile(old, new)?;
            was_current
        };
        if was_current {
            self.remember_profile(new.trim());
        }
        Ok(())
    }

    #[inline(always)]
    pub fn settings(&self) -> RwLockReadGuard<'_, Settings> {
        self.settings.read()
    }

    #[inline(always)]
    pub fn settings_mut(&self) -> RwLockWriteGuard<'_, Settings> {
        self.settings.write()
    }

    #[inline(always)]
    pub fn mod_manager(&self) -> RwLockReadGuard<'_, mods::Manager> {
        self.mod_manager.read()
    }

    #[inline(always)]
    pub fn deploy_manager(&self) -> RwLockReadGuard<'_, deploy::Manager> {
        self.deploy_manager.read()
    }
}
