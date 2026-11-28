use strfmt::Format;
use uk_content::prelude::Endian;
use swmm_i18n::string_ext::LocString;
use super::*;

impl App {
    pub(super) fn handle_update(
        &mut self,
        ctx: &eframe::egui::Context,
        _frame: &mut eframe::Frame,
    ) {
        if let Ok(msg) = self.channel.1.try_recv() {
            self.handle_message(ctx, msg);
            self.persist_dirty();
        } else {
            self.handle_drops(ctx);
        }
    }

    fn show_current_profile(&mut self) {
        self.mods = self.core.mod_manager().all_mods().collect();
        self.selected = self.mods.first().cloned().into_iter().collect();
        self.drag_index = None;
        self.hover_index = None;
        self.options_mod = None;
        self.update_mod = None;
        self.do_update(Message::RefreshModsDisplay);
        self.do_update(Message::ReloadProfiles);
    }

    fn handle_message(&mut self, ctx: &eframe::egui::Context, msg: Message) {
        {
            match msg {
                Message::Noop => self.busy.set(false),
                Message::ResetMods(dirty) => {
                    self.busy.set(false);
                    chrome::celebrate_if_pending(ctx);
                    self.game_status = frame::GameStatus::check(&self.core);
                    self.dirty_mut().clear();
                    if let Some(dirty) = dirty {
                        self.dirty_mut().extend(&dirty);
                    }
                    self.mods = self.core.mod_manager().all_mods().collect();
                    self.selected.retain(|m| self.mods.contains(m));
                    self.do_update(Message::RefreshModsDisplay);
                    self.do_update(Message::ReloadProfiles);
                    ctx.data_mut(|d| {
                        d.remove::<Arc<Mutex<egui_commonmark::CommonMarkCache>>>(egui::Id::new(
                            "md_cache",
                        ))
                    });
                    mod_info::ROOTS.write().clear();
                }
                Message::RefreshModsDisplay => {
                    self.do_update(Message::ChangeSort(self.list_state.sort, self.list_state.descending));
                }
                Message::ChangeSort(sort, rev) => {
                    let orderer = sort.orderer();
                    let mut temp = self.mods.iter().cloned().enumerate().collect::<Vec<_>>();
                    temp.sort_by(orderer);
                    self.displayed_mods = if rev {
                        temp.into_iter().rev().map(|(_, m)| m).collect()
                    } else {
                        temp.into_iter().map(|(_, m)| m).collect()
                    };
                    self.list_state.sort = sort;
                    self.list_state.descending = rev;
                    self.save_ui_state();
                }
                Message::CloseError => self.error = None,
                Message::CloseConfirm => self.confirm = None,
                Message::ShowAbout => self.show_about = true,
                Message::CloseAbout => self.show_about = false,
                Message::ShowLicenses => {
                    self.show_about = false;
                    self.show_licenses = true;
                }
                Message::CloseLicenses => {
                    self.show_licenses = false;
                    self.show_about = true;
                }
                Message::Confirm(msg, prompt) => {
                    self.confirm = Some((*msg, prompt));
                }
                msg @ (Message::CheckForUpdates
                | Message::UpdateAvailable(..)
                | Message::UpToDate
                | Message::InstallUpdate(_)
                | Message::UpdateReady(_)) => self.handle_update_message(ctx, msg),
                Message::SelectOnly(_)
                | Message::SelectAlso(_)
                | Message::Deselect(_)
                | Message::MoveSelected(_)
                | Message::StartDrag(_)
                    if self.mods.is_empty() => {}
                Message::SelectOnly(i) => {
                    let index = i.clamp(0, self.mods.len() - 1);
                    let mod_ = &self.mods[index];
                    if self.selected.contains(mod_) {
                        self.selected.retain(|m| m == mod_);
                    } else {
                        self.selected.clear();
                        self.selected.push(self.mods[index].clone());
                    }
                    self.drag_index = None;
                }
                Message::SelectAlso(i) => {
                    let index = i.clamp(0, self.mods.len() - 1);
                    let mod_ = &self.mods[index];
                    if !self.selected.contains(mod_) {
                        self.selected.push(mod_.clone());
                    }
                    self.drag_index = None;
                }
                Message::SelectThrough(i) if i >= self.mods.len() => {}
                Message::SelectThrough(i) => {
                    let mods_visible = self.filtered_mods(&self.mod_list_filter);
                    if let Some(index) = mods_visible.iter()
                        .position(|(_, m)| *m == &self.mods[i])
                    {
                        if let Some(start_index) = self
                            .selected
                            .first()
                            .and_then(|sm| mods_visible.iter()
                                .position(|(_, m)| *m == sm)
                            )
                        {
                            let range = if start_index < index {
                                start_index..=index
                            } else {
                                index..=start_index
                            };
                            self.selected = mods_visible
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| range.contains(i))
                                .map(|(_, (_, m))| (*m).clone())
                                .collect();
                        }
                        self.drag_index = None;
                    }
                }
                Message::Deselect(i) => {
                    let index = i.clamp(0, self.mods.len() - 1);
                    let mod_ = &self.mods[index];
                    self.selected.retain(|m| m != mod_);
                    self.drag_index = None;
                }
                Message::SelectAll => {
                    let filter = self.mod_list_filter.clone();
                    let visible: Vec<Mod> = self
                        .filtered_mods(&filter)
                        .into_iter()
                        .map(|(_, m)| m.clone())
                        .collect();
                    self.selected = visible;
                    self.drag_index = None;
                }
                Message::ReinstallMods(mods) => {
                    let mods = mods.unwrap_or_else(|| self.selected.clone());
                    let changes = mods.iter().try_fold(Manifest::default(), |mut all, m| -> Result<Manifest> {
                        all.extend(m.manifest()?.as_ref());
                        Ok(all)
                    });
                    match changes {
                        Ok(changes) if !changes.is_empty() => {
                            log::info!("Reinstalling {} mod(s)", mods.len());
                            self.dirty_mut().extend(&changes);
                            self.do_update(Message::Apply);
                        }
                        Ok(_) => {}
                        Err(e) => self.do_update(Message::Error(e)),
                    }
                }
                Message::ClearSelect => {
                    self.selected.clear();
                    self.drag_index = None;
                }
                Message::StartDrag(i) => {
                    if ctx.input(|i| i.pointer.any_released()) {
                        self.drag_index = None;
                    }
                    let i = i.min(self.mods.len() - 1);
                    self.drag_index = Some(i);
                    let mod_ = &self.mods[i];
                    if !self.selected.contains(mod_) {
                        if !ctx.input(|i| i.modifiers.ctrl) {
                            self.selected.clear();
                        }
                        self.selected.push(mod_.clone());
                    }
                }
                Message::ClearDrag => {
                    self.drag_index = None;
                }
                Message::MoveSelected(dest_index) => {
                    let dest_index = dest_index.clamp(0, self.mods.len() - 1);
                    if self.selected.len() == self.mods.len() {
                        return;
                    }
                    self.mods.retain(|m| !self.selected.contains(m));
                    for (i, selected_mod) in self.selected.iter().enumerate() {
                        self.mods
                            .insert((dest_index + i).min(self.mods.len()), selected_mod.clone());
                    }
                    self.hover_index = None;
                    self.drag_index = None;
                    match self.selected.iter().try_for_each(|m| {
                        self.dirty_mut()
                            .extend(m.manifest_with_options(&m.enabled_options)?.as_ref());
                        Ok(())
                    }) {
                        Ok(()) => self.do_update(Message::RefreshModsDisplay),
                        Err(e) => self.do_update(Message::Error(e)),
                    };
                }
                Message::ChangeProfile(profile) => {
                    match self.core.change_profile(profile) {
                        Ok(()) => {
                            self.show_current_profile();
                            let unapplied = self.dirty().clone();
                            self.do_task(move |core| jobs::refresh_pending(core, unapplied));
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    };
                }
                Message::NewProfile => self.new_profile = Some("".into()),
                Message::EditMod(mod_) => {
                    self.edit_mod = Some(dialogs::ModDetailsEdit::new(&mod_));
                }
                Message::SaveModDetails(hash, name, description) => {
                    match self.core.mod_manager().set_custom_details(hash, &name, &description) {
                        Ok(updated) => {
                            for list in [&mut self.mods, &mut self.selected] {
                                for m in list.iter_mut().filter(|m| m.hash() == hash) {
                                    m.custom_name = updated.custom_name.clone();
                                    m.custom_description = updated.custom_description.clone();
                                }
                            }
                            self.edit_mod = None;
                            self.do_update(Message::RefreshModsDisplay);
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    }
                }
                Message::AddProfile => {
                    if let Some(profile) = self.new_profile.take() {
                        let profile = profile.trim().to_string();
                        let exists = self
                            .core
                            .settings()
                            .profiles()
                            .any(|p| p.as_str().eq_ignore_ascii_case(&profile));
                        if exists {
                            self.do_update(Message::Error(anyhow_ext::anyhow!(
                                "A profile named {profile} already exists"
                            )));
                        } else if !profile.is_empty() {
                            match self.core.change_profile(&profile) {
                                Ok(()) => {
                                    self.show_current_profile();
                                    let unapplied = self.dirty().clone();
                                    self.do_task(move |core| jobs::refresh_pending(core, unapplied));
                                }
                                Err(e) => self.do_update(Message::Error(e)),
                            };
                        }
                    }
                }
                Message::DeleteProfile(profile) => {
                    if profile == self.core.mod_manager().current_profile() {
                        self.do_update(Message::Error(anyhow_ext::anyhow!(
                            "The active profile can't be deleted. Switch to another profile first."
                        )));
                        return;
                    }
                    self.dirty.write().remove(profile.as_str());
                    self.saved_dirty.borrow_mut().remove(profile.as_str());
                    self.do_task(move |core| {
                        let path = core.settings().profiles_dir().join(profile.as_str());
                        swmm_core::util::remove_dir_all(path)?;
                        core.mod_manager().forget_profile(&profile);
                        Ok(Message::CleanProfile(profile))
                    })
                }
                Message::DuplicateProfile(profile) => {
                    self.do_task(move |core| {
                        let profiles_dir = core.settings().profiles_dir();
                        let new_profile =
                            swmm_core::modpack::free_profile_name(&core, &format!("{profile} copy"));
                        if core.mod_manager().current_profile() == profile {
                            core.mod_manager().save()?;
                        }
                        swmm_core::util::copy_dir(
                            profiles_dir.join(&profile),
                            profiles_dir.join(&new_profile),
                        )?;
                        core.mod_manager().add_profile(new_profile.into())?;
                        Ok(Message::ReloadProfiles)
                    });
                }
                Message::RenameProfile(profile, rename) => {
                    let rename = rename.trim().to_string();
                    match self.core.rename_profile(&profile, &rename) {
                        Ok(()) => {
                            let mut dirty = self.dirty.write();
                            if let Some(changes) = dirty.remove(profile.as_str()) {
                                dirty.insert(rename.as_str().into(), changes);
                            }
                            drop(dirty);
                            let mut saved = self.saved_dirty.borrow_mut();
                            if let Some(changes) = saved.remove(profile.as_str()) {
                                saved.insert(rename.as_str().into(), changes);
                            }
                            drop(saved);
                            let mut state = self.profiles_state.borrow_mut();
                            state.selected = Some(rename.as_str().into());
                            state.rename = None;
                            drop(state);
                            self.do_update(Message::ReloadProfiles);
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    }
                }
                Message::CleanProfile(profile) => {
                    let mut state = self.profiles_state.borrow_mut();
                    let mods = state
                        .profiles
                        .get(profile.as_str())
                        .map(|p| {
                            p.mods()
                                .iter()
                                .map(|(h, m)| (*h, m.path.clone()))
                                .collect::<HashMap<_, _>>()
                        })
                        .unwrap_or_default();
                    state.reload(&self.core);
                    let assigned = state.all_assigned_mod_hashes();
                    mods.iter()
                        .filter(|(h, _)| !assigned.contains(h))
                        .for_each(|(_, p)| fs::remove_file(p).unwrap_or_default());
                    self.busy.set(false);
                }
                Message::ReloadProfiles => {
                    self.profiles_state.borrow_mut().reload(&self.core);
                    self.busy.set(false);
                }
                Message::SetFocus(pane) => {
                    self.focused = pane;
                }
                Message::SetTheme(theme) => {
                    self.theme_fade = Some(super::intro::ThemeFade::new(ctx, self.theme.palette().base));
                    theme.set_theme(ctx);
                    self.theme = theme;
                    self.save_ui_state();
                }
                Message::SetLanguage(lang) => {
                    LOCALIZATION.write().update_language(&lang);
                }
                Message::SelectFile => {
                    if let Some(mut paths) = rfd::FileDialog::new()
                        .set_title("Mod_Select_Title".localize())
                        .add_filter("Any mod", swmm_core::import::MOD_EXTENSIONS)
                        .add_filter("Archives (*.zip, *.7z, *.rar, *.tar, ...)", swmm_core::import::ARCHIVE_EXTENSIONS)
                        .add_filter("BNP mod (*.bnp)", &["bnp"])
                        .add_filter("SWMM modpack (*.swmmpack)", &[swmm_core::modpack::EXTENSION])
                        .add_filter("All files (*.*)", &["*"])
                        .pick_files()
                        .filter(|p| !p.is_empty())
                    {
                        let first = paths.remove(0);
                        self.install_queue.extend(paths);
                        self.error_queue.clear();
                        self.do_task(move |core| jobs::open_mod(&core, &first, None));
                    }
                }
                Message::OpenMod(path) => {
                    let core = self.core.clone();
                    let meta = self.meta_input.take();
                    ctx.request_repaint();
                    self.do_task(move |_| jobs::open_mod(&core, &path, meta));
                }
                Message::OpenDownloadedMod(path, source) => {
                    let core = self.core.clone();
                    ctx.request_repaint();
                    self.do_task(move |_| jobs::open_downloaded_mod(&core, &path, source));
                }
                Message::HandleMod(mod_) => {
                    self.busy.set(false);
                    log::debug!("{:#?}", mod_);
                    for (hash, (name, version)) in mod_.meta.masters.iter() {
                        if !self.mods.iter().any(|m| m.hash() == *hash) {
                            self.do_update(Message::Error(anyhow_ext::anyhow!(
                                "Could not find required mod dependency {} (version {})",
                                name,
                                version
                            )));
                        }
                    }
                    if !matches!(mod_.meta.platform, ModPlatform::Universal)
                        && mod_.meta.platform != ModPlatform::Specific(self.platform().into())
                    {
                        self.do_update(Message::Error(anyhow_ext::anyhow!(
                            "Mod is for {:?}, current mode is {}",
                            match mod_.meta.platform {
                                ModPlatform::Specific(Endian::Little) => "Switch",
                                ModPlatform::Specific(Endian::Big) => "Wii U",
                                ModPlatform::Universal => "any console",
                            },
                            self.platform()
                        )));
                    } else if !mod_.meta.options.is_empty() {
                        self.do_update(Message::RequestOptions(mod_, false));
                    } else {
                        self.do_update(Message::InstallMod(mod_));
                    }
                }
                Message::InstallMod(tmp_mod_) => {
                    let update_mod = self.update_mod.take();
                    self.do_task(move |core| {
                        let mods = core.mod_manager();
                        if let Some(mod_) = update_mod {
                            let mut dirty = Manifest::default();
                            dirty.extend(&tmp_mod_.manifest().unwrap_or_default());
                            let tmp_path = tmp_mod_.path.clone();
                            mods.replace(tmp_mod_, mod_.hash())?;
                            swmm_core::util::release_temp(&tmp_path);
                            log::info!("Updated {}", mod_.meta.name);
                            dirty.extend(&mod_.manifest().unwrap_or_default());
                            Ok(Message::ResetMods(Some(dirty)))
                        } else {
                            let mod_ = mods.add(&tmp_mod_.path, None)?;
                            swmm_core::util::release_temp(&tmp_mod_.path);
                            let hash = mod_.as_map_id();
                            if !tmp_mod_.enabled_options.is_empty() {
                                mods.set_enabled_options(hash, tmp_mod_.enabled_options)?;
                            }
                            mods.save()?;
                            log::info!("Added mod {} to current profile", mod_.meta.name.as_str());
                            let mod_ = mods
                                .get_mod(hash)
                                .context("The mod was installed but isn't in the profile")?;
                            Ok(Message::AddMod(mod_))
                        }
                    });
                }
                Message::UninstallMods(mods) => {
                    let mods = mods.unwrap_or_else(|| self.selected.clone());
                    self.do_task(move |core| {
                        let manager = core.mod_manager();
                        mods.iter().try_for_each(|m| -> Result<()> {
                            manager.del(m.as_map_id(), None)?;
                            log::info!("Removed mod {} from current profile", m.meta.name.as_str());
                            Ok(())
                        })?;
                        manager.save()?;
                        Ok(Message::RemoveMods(mods))
                    });
                }
                Message::ModUpdate => {
                    if let Some(file) = rfd::FileDialog::new()
                        .set_title("Mod_Select_Title".localize())
                        .add_filter("Any mod", swmm_core::import::MOD_EXTENSIONS)
                        .add_filter("Archives (*.zip, *.7z, *.rar, *.tar, ...)", swmm_core::import::ARCHIVE_EXTENSIONS)
                        .add_filter("BNP mod (*.bnp)", &["bnp"])
                        .add_filter("SWMM modpack (*.swmmpack)", &[swmm_core::modpack::EXTENSION])
                        .add_filter("All files (*.*)", &["*"])
                        .pick_file()
                    {
                        let path = file.clone();
                        self.update_mod = self.selected.first().cloned();
                        self.do_task(move |core| jobs::open_mod(&core, &path, None));
                    }
                }
                Message::ToggleMods(mods, enabled) => {
                    let mods = mods.as_ref().unwrap_or(&self.selected);
                    let dirty = mods.iter().try_fold(
                        Manifest::default(),
                        |mut dirty, m| -> Result<Manifest> {
                            let Some(mod_) = self.mods.iter_mut().find(|m2| m.eq(m2)) else {
                                return Ok(dirty);
                            };
                            if mod_.enabled == enabled {
                                return Ok(dirty);
                            }
                            mod_.enabled = enabled;
                            dirty.extend(m.manifest()?.as_ref());
                            Ok(dirty)
                        },
                    );
                    match dirty {
                        Ok(dirty) => {
                            self.dirty_mut().extend(&dirty);
                            self.do_update(Message::RefreshModsDisplay)
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    };
                }
                Message::AddMod(mod_) => {
                    if let Ok(manifest) = mod_.manifest() {
                        self.dirty_mut().extend(&manifest);
                    }
                    self.mods = self.core.mod_manager().all_mods().collect();
                    self.do_update(Message::RefreshModsDisplay);
                    self.busy.set(false);
                    if let Some(path) = self.install_queue.pop_front() {
                        self.do_task(move |core| jobs::open_mod(&core, &path, None));
                    } else if !self.error_queue.is_empty() {
                        let msg = self
                            .error_queue
                            .drain(..)
                            .fold(String::new(), |mut acc, e| {
                                writeln!(acc, "{:?}", e).unwrap_or(());
                                acc
                            });
                        self.do_update(Message::Error(anyhow_ext::anyhow!("{msg}").context(
                            "One or more errors occured while installing your mods. Please see \
                             full details.",
                        )));
                    }
                }
                Message::AddToProfile(profile) => {
                    let mut dirty = self.dirty.write();
                    let dirty = dirty.entry(profile.as_str().into()).or_default();
                    let mut err = false;
                    for mod_ in &self.selected {
                        match self.core.mod_manager().add(&mod_.path, Some(&profile)) {
                            Ok(_) => {
                                if let Ok(manifest) = mod_.manifest() {
                                    dirty.extend(&manifest);
                                }
                            }
                            Err(e) => {
                                self.do_update(Message::Error(e));
                                err = true;
                                break;
                            }
                        };
                    }
                    if !err {
                        self.toasts.add({
                            let message = "Profile_Added".localize();
                            let vars = std::collections::HashMap::from(
                                [("profile_name".to_string(), profile.to_string())]
                            );
                            let mut toast = Toast::success(message.format(&vars).unwrap_or_else(|_| message.to_string()));
                            toast.set_duration(Some(Duration::new(2, 0)));
                            toast
                        });
                    }
                }
                Message::RemoveMods(mods) => {
                    self.mods.retain(|m| !mods.contains(m));
                    self.selected.retain(|m| !mods.contains(m));
                    mods.iter().for_each(|m| {
                        if let Ok(manifest) = m.manifest() {
                            self.dirty_mut().extend(&manifest);
                        }
                    });
                    self.do_update(Message::RefreshModsDisplay);
                    self.busy.set(false);
                }
                Message::Apply => {
                    chrome::celebrate_next(ctx);
                    let mods = self.mods.clone();
                    let dirty = std::mem::take(self.dirty_mut().deref_mut());
                    self.do_task(move |core| jobs::apply_changes(&core, mods, Some(dirty)));
                }
                Message::Deploy => {
                    chrome::celebrate_next(ctx);
                    self.do_task(move |core| {
                        log::info!("Deploying current mod configuration");
                        core.deploy_manager().ensure_merged()?;
                        core.deploy_manager().deploy()?;
                        jobs::after_deploy(&core);
                        Ok(Message::ResetMods(None))
                    })
                }
                Message::SetPage(page) => self.page = page,
                Message::Launch => {
                    let mods = self.mods.clone();
                    let dirty = std::mem::take(self.dirty_mut().deref_mut());
                    self.do_task(move |core| jobs::launch_savage_wind(&core, mods, dirty));
                }
                Message::Launched => {
                    self.busy.set(false);
                    self.toasts.add({
                        let mut toast = Toast::success("SW_Launched".localize());
                        toast.set_duration(Some(Duration::new(3, 0)));
                        toast
                    });
                    self.do_update(Message::ResetMods(None));
                    self.release_memory(ctx);
                }
                Message::SwAutoDetect => {
                    self.do_task(jobs::sw_auto_detect);
                }
                Message::GameConfigOpened => {
                    self.busy.set(false);
                    self.toasts.add({
                        let mut toast = Toast::success("SW_Config_Opened".localize());
                        toast.set_duration(Some(Duration::new(3, 0)));
                        toast
                    });
                    self.do_update(Message::ResetMods(None));
                }
                Message::OpenGameConfig => {
                    let mods = self.mods.clone();
                    let dirty = std::mem::take(self.dirty_mut().deref_mut());
                    self.do_task(move |core| jobs::open_game_config(&core, mods, dirty));
                }
                Message::ResetPending => {
                    self.do_task(|core| {
                        log::info!("Resetting pending deployment data");
                        core.deploy_manager().reset_pending()?;
                        core.deploy_manager().save()?;
                        Ok(Message::Noop)
                    })
                }
                Message::Remerge => {
                    self.do_task(|core| jobs::apply_changes(&core, vec![], None));
                }
                Message::ResetSettings => {
                    self.busy.set(false);
                    self.temp_settings = self.core.settings().clone();
                }
                Message::SaveSettings => {
                    let mut needs_reset = false;
                    let old_storage = self.core.settings().storage_dir.clone();
                    if old_storage != self.temp_settings.storage_dir {
                        swmm_core::savage_wind::follow_storage(&mut self.temp_settings, &old_storage);
                    }
                    let old_dep = self.core.settings().platform_config().and_then(|p| p.deploy_config.clone());
                    let new_dep = self.temp_settings.platform_config().and_then(|p| p.deploy_config.clone());
                    if let (Some(old_dep), Some(new_dep)) = (old_dep, new_dep) {
                        if (old_dep.layout != new_dep.layout
                            || old_dep.method != new_dep.method
                            || old_dep.output != new_dep.output)
                            && self.core.settings().wipe_output(self.core.settings().current_mode.into()).is_ok()
                        {
                            needs_reset = true;
                        }
                    }
                    let old_sw = self.core.settings().savage_wind.clone();
                    let sw_paths_changed = old_sw.executable != self.temp_settings.savage_wind.executable;
                    let active = self.core.mod_manager().current_profile().to_string();
                    if let Some(config) = self.temp_settings.platform_config_mut() {
                        config.profile = active.as_str().into();
                    }
                    let save_res = self.temp_settings.save().and_then(|_| {
                        self.core.reload()?;
                        Ok(())
                    });
                    match save_res {
                        Ok(()) => {
                            self.toasts.add({
                                let mut toast = Toast::success("Settings_Saved".localize());
                                toast.set_duration(Some(Duration::new(2, 0)));
                                toast
                            });
                            if let Some(dump) = self.core.settings().dump() {
                                dump.clear_cache()
                            }
                            jobs::sync_loader(&self.core.settings());
                            self.game_status = frame::GameStatus::check(&self.core);
                            if sw_paths_changed {
                                self.do_update(Message::SwAutoDetect);
                            }
                            self.do_update(Message::ClearSelect);
                            self.do_update(Message::ResetMods(None));
                            if needs_reset {
                                self.do_update(Message::ResetPending);
                            }
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    };
                }
                Message::HandleSettings => {
                    self.temp_settings = self.core.settings().clone();
                    self.game_status = frame::GameStatus::check(&self.core);
                    self.toasts.add({
                        let mut toast = Toast::success("Settings_Saved".localize());
                        toast.set_duration(Some(Duration::new(2, 0)));
                        toast
                    });
                    if let Some(dump) = self.core.settings().dump() {
                        dump.clear_cache()
                    }
                    self.do_update(Message::ClearSelect);
                    self.do_update(Message::ResetMods(None));
                }
                Message::RequestOptions(mut mod_, update) => {
                    if !update {
                        mod_.enable_default_options();
                    }
                    self.options_mod = Some((mod_, update));
                }
                Message::UpdateOptions(mod_) => {
                    let opts = mod_.enabled_options.clone();
                    match self
                        .core
                        .mod_manager()
                        .set_enabled_options(mod_.hash(), opts)
                    {
                        Ok(manifest) => {
                            self.dirty_mut().extend(&manifest);
                            if let Some(old_mod) =
                                self.mods.iter_mut().find(|m| m.hash() == mod_.hash())
                            {
                                *old_mod = mod_.clone();
                            }
                            if let Some(old_mod) =
                                self.selected.iter_mut().find(|m| m.hash() == mod_.hash())
                            {
                                *old_mod = mod_;
                            }
                            self.do_update(Message::RefreshModsDisplay);
                        }
                        Err(e) => self.do_update(Message::Error(e)),
                    }
                }
                Message::Error(error) => {
                    chrome::celebrate_cancel(ctx);
                    log::error!("{:?}", error);
                    if self.install_queue.is_empty() {
                        self.busy.set(false);
                        self.error = Some(error);
                    } else {
                        log::warn!("More operations in queue, stashing error and continuing…");
                        self.error_queue.push_back(error);
                        if let Some(path) = self.install_queue.pop_front() {
                            self.do_task(move |core| jobs::open_mod(&core, &path, None));
                        }
                    }
                }

                Message::CloseChangelog => self.changelog = None,
                Message::GameCrashed(report) => {
                    self.crash = Some(report);
                    ctx.request_repaint();
                }
                Message::CloseCrash => self.crash = None,
                Message::BackupSaves => {
                    self.do_task(|core| {
                        let settings = core.settings().clone();
                        let dir = swmm_core::saves::save_dir(&settings)
                            .context("Savage Wind's save folder wasn't found. Check Savage Wind's location in Settings.")?;
                        let made = swmm_core::saves::backup(&dir, false)?;
                        Ok(Message::SavesChanged(Some(
                            if made.is_some() { "SW_Saves_BackedUp" } else { "SW_Saves_Nothing" }.localize().into(),
                        )))
                    });
                }
                Message::RestoreSaves(path) => {
                    self.do_task(move |core| {
                        let settings = core.settings().clone();
                        let dir = swmm_core::saves::save_dir(&settings).context("Savage Wind's save folder wasn't found")?;
                        swmm_core::saves::restore(&dir, &path)?;
                        Ok(Message::SavesChanged(Some("SW_Saves_Restored".localize().into())))
                    });
                }
                Message::SavesChanged(note) => {
                    self.busy.set(false);
                    *self.saves.borrow_mut() = None;
                    if let Some(note) = note {
                        self.toasts.add({
                            let mut toast = Toast::success(note);
                            toast.set_duration(Some(Duration::new(3, 0)));
                            toast
                        });
                    }
                }
                Message::SelectModpack => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("SW_Modpack_Import".localize())
                        .add_filter("SWMM modpack (*.swmmpack)", &[swmm_core::modpack::EXTENSION])
                        .pick_file()
                    {
                        self.do_update(Message::ImportModpack(path));
                    }
                }
                Message::ImportModpack(path) => {
                    self.do_task(move |core| jobs::import_modpack(&core, &path));
                }
                Message::ExportModpack(profile) => {
                    if let Some(dest) = rfd::FileDialog::new()
                        .set_title("SW_Modpack_Share".localize())
                        .set_file_name(format!("{profile}.{}", swmm_core::modpack::EXTENSION))
                        .add_filter("SWMM modpack (*.swmmpack)", &[swmm_core::modpack::EXTENSION])
                        .save_file()
                    {
                        self.do_task(move |core| jobs::export_modpack(&core, &profile, &dest));
                    }
                }
                Message::ModpackImported(profile) => {
                    self.busy.set(false);
                    self.toasts.add({
                        let mut toast = Toast::success(format!("{} {profile}", "SW_Modpack_Imported".localize()));
                        toast.set_duration(Some(Duration::new(3, 0)));
                        toast
                    });
                    self.dirty_mut().clear();
                    self.show_current_profile();
                    self.do_update(Message::Remerge);
                }
                Message::SelectCompatFile => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Compat_Pick".localize())
                        .add_filter("Any mod", swmm_core::import::MOD_EXTENSIONS)
                        .add_filter("All files (*.*)", &["*"])
                        .pick_file()
                    {
                        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        let name = path.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        self.do_update(Message::TestCompat { path, id: 0, name, file, downloaded: false });
                    }
                }
                Message::TestCompat { path, id, name, file, downloaded } => {
                    ctx.request_repaint();
                    self.do_task(move |core| jobs::test_compat(&core, &path, id, &name, &file, downloaded));
                }
                Message::CompatTested(entry) => {
                    self.busy.set(false);
                    self.compat.borrow_mut().tested(*entry);
                    self.page = super::frame::Page::Compat;
                }
                Message::ShowCompat(query) => {
                    self.compat.borrow_mut().query = query;
                    self.page = super::frame::Page::Compat;
                }
                Message::Finished => {
                    self.busy.set(false);
                    chrome::celebrate(ctx);
                }
                Message::Toast(msg) => {
                    self.toasts.add({
                        let seconds = if msg.len() > 60 { 8 } else { 2 };
                        let mut toast = Toast::info(msg);
                        toast.set_duration(Some(Duration::new(seconds, 0)));
                        toast
                    });
                }
            }
        }
    }
}
