use std::{
    hash::{Hash, Hasher},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Weak},
};

use anyhow_ext::{Context, Result};
use dashmap::{mapref::one::MappedRef, DashMap};
use fs_err as fs;
use lenient_semver::Version;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use sanitise_file_name as sfn;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use smartstring::alias::String;
use uk_mod::{unpack::ModReader, Manifest, Meta, ModOption};

use crate::{
    settings::Settings,
    util::{self, HashMap},
};

type ManifestCache = LazyLock<RwLock<HashMap<(usize, Vec<PathBuf>), Result<Arc<Manifest>>>>>;

#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Mod {
    pub meta: Meta,
    pub enabled_options: Vec<ModOption>,
    pub enabled: bool,
    pub path: PathBuf,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) hash: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_description: Option<String>,
}

impl std::fmt::Debug for Mod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mod")
            .field("meta", &self.meta)
            .field("enabled_options", &self.enabled_options)
            .field("enabled", &self.enabled)
            .field("path", &self.path)
            .field("hash", &self.hash)
            .field("custom_name", &self.custom_name)
            .field("custom_description", &self.custom_description)
            .finish()
    }
}

impl Hash for Mod {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.hash)
    }
}

impl PartialEq for Mod {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Mod {
    pub fn from_reader(reader: ModReader) -> Self {
        let mut hasher = rustc_hash::FxHasher::default();
        reader.meta.hash(&mut hasher);
        Self {
            hash: hasher.finish() as usize,
            meta: reader.meta,
            enabled_options: vec![],
            path: reader.path,
            enabled: false,
            custom_name: None,
            custom_description: None,
        }
    }

    pub fn display_name(&self) -> &str {
        self.custom_name.as_deref().unwrap_or(self.meta.name.as_str())
    }

    pub fn display_description(&self) -> &str {
        self.custom_description
            .as_deref()
            .unwrap_or(self.meta.description.as_str())
    }

    pub fn set_custom_details(&mut self, name: &str, description: &str) {
        let name = name.trim();
        self.custom_name =
            (!name.is_empty() && name != self.meta.name.as_str()).then(|| name.into());
        self.custom_description =
            (description.trim() != self.meta.description.trim()).then(|| description.into());
    }

    fn keep_custom_details(&mut self, old: &Mod) {
        self.custom_name = old.custom_name.clone();
        self.custom_description = old.custom_description.clone();
    }

    pub fn manifest(&self) -> Result<Arc<Manifest>> {
        self.manifest_with_options(&self.enabled_options)
    }

    pub fn manifest_with_options(&self, options: impl AsRef<[ModOption]>) -> Result<Arc<Manifest>> {
        static MANIFEST_CACHE: ManifestCache = LazyLock::new(|| RwLock::new(HashMap::default()));
        match MANIFEST_CACHE
            .write()
            .entry((
                self.hash,
                options.as_ref().iter().map(|o| o.path.clone()).collect(),
            ))
            .or_insert_with(|| {
                ModReader::open(&self.path, options.as_ref()).map(|r| Arc::new(r.manifest))
            }) {
            Ok(manifest) => Ok(manifest.clone()),
            Err(e) => Err(anyhow::format_err!("{:?}", e)),
        }
    }

    pub fn state_eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled && self.enabled_options == other.enabled_options
    }

    #[inline(always)]
    pub fn hash(&self) -> usize {
        self.hash
    }

    pub fn enable_default_options(&mut self) {
        if !self.meta.options.is_empty() {
            for group in self.meta.options.iter_mut() {
                match group {
                    uk_mod::OptionGroup::Exclusive(group) => {
                        self.enabled_options
                            .extend(group.default.iter().filter_map(|path| {
                                group.options.iter().find(|o| &o.path == path).cloned()
                            }));
                    }
                    uk_mod::OptionGroup::Multiple(group) => {
                        self.enabled_options
                            .extend(group.defaults.iter().filter_map(|path| {
                                group.options.iter().find(|o| &o.path == path).cloned()
                            }));
                    }
                };
            }
        }
    }
}

pub trait LookupMod {
    fn as_map_id(&self) -> usize;
}

impl LookupMod for Mod {
    #[inline(always)]
    fn as_map_id(&self) -> usize {
        self.hash
    }
}

impl LookupMod for &Mod {
    #[inline(always)]
    fn as_map_id(&self) -> usize {
        self.hash
    }
}

impl LookupMod for usize {
    #[inline(always)]
    fn as_map_id(&self) -> usize {
        *self
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Profile {
    mods: RwLock<HashMap<usize, Mod>>,
    load_order: RwLock<Vec<usize>>,
}

impl Clone for Profile {
    fn clone(&self) -> Self {
        Self {
            mods: RwLock::new(self.mods.read().clone()),
            load_order: RwLock::new(self.load_order.read().clone()),
        }
    }
}

impl Profile {
    pub fn mods(&self) -> RwLockReadGuard<'_, HashMap<usize, Mod>> {
        self.mods.read()
    }

    pub fn mods_mut(&self) -> RwLockWriteGuard<'_, HashMap<usize, Mod>> {
        self.mods.write()
    }

    #[allow(unused)]
    pub fn load_order(&self) -> RwLockReadGuard<'_, Vec<usize>> {
        self.load_order.read()
    }

    pub fn load_order_mut(&self) -> RwLockWriteGuard<'_, Vec<usize>> {
        self.load_order.write()
    }

    pub fn iter(self_: MappedRef<'_, String, Profile, Profile>) -> ModIterator<'_> {
        ModIterator {
            profile: self_,
            index:   0,
        }
    }

    pub fn validate(&mut self, all_mods: &HashMap<String, Mod>) {
        let mut mods = self.mods.write();
        let mut mods_by_invalid_hash = HashMap::<usize, Mod>::default();
        mods.retain(|h, m| {
            let found = all_mods
                .get(&path_key(&m.path))
                .or_else(|| all_mods.values().find(|x| x.meta.name == m.meta.name));
            match found {
                None => {
                    log::warn!(
                        "{} not found at {}, removing from mod list...",
                        m.meta.name,
                        m.path.display()
                    );
                    false
                },
                Some(m_) => {
                    if m.path != m_.path {
                        log::warn!(
                            "{} not found at {}, loading from {} instead...",
                            m.meta.name,
                            m.path.display(),
                            m_.path.display()
                        );
                        m.path = m_.path.clone();
                    }
                    if *h != m_.hash {
                        log::info!(
                            "{} has a hash mismatch. Reassigning hash...",
                            m.meta.name
                        );
                        m.hash = m_.hash;
                        mods_by_invalid_hash.insert(*h, m.clone());
                        false
                    } else {
                        true
                    }
                },
            }
        });
        for m in mods_by_invalid_hash.values() {
            mods.insert(m.hash, m.clone());
        }
        let mut load_order = self.load_order.write();
        for i in 0..load_order.len() {
            if let Some(mod_) = mods_by_invalid_hash.get(&load_order[i]) {
                load_order[i] = mod_.hash;
            }
        }
        for h in load_order.iter() {
            if !mods.contains_key(h)
                && let Some((_, m)) = all_mods.iter().find(|(_, m)| *h == m.hash) {
                    log::warn!(
                        "{} found in load order but not in profile mod list. Adding to profile \
                        with no options selected. You will need to reselect mod options from the \
                        Info tab if you want this mod to continue working.",
                        m.meta.name
                    );
                    mods.insert(*h, m.clone());
                }
        }
        load_order.retain(|k| mods.contains_key(k));
        let keys_missing_from_order = mods.keys()
            .filter(|&k| !load_order.contains(k))
            .collect::<Vec<_>>();
        load_order.extend(keys_missing_from_order);
    }
}

pub fn load_unapplied(profiles_dir: &Path, profile: &str) -> Manifest {
    let path = profiles_dir.join(profile).join(UNAPPLIED_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => serde_yaml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("Couldn't read the unapplied changes of {profile}: {e}");
            Manifest::default()
        }),
        Err(_) => Manifest::default(),
    }
}

pub fn save_unapplied(profiles_dir: &Path, profile: &str, changes: &Manifest) -> Result<()> {
    let dir = profiles_dir.join(profile);
    let path = dir.join(UNAPPLIED_FILE);
    if changes.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else if dir.is_dir() {
        fs::write(&path, serde_yaml::to_string(changes)?)?;
    }
    Ok(())
}

const UNAPPLIED_FILE: &str = "unapplied.yml";

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase().into()
}

fn meta_hash(path: &Path) -> Option<usize> {
    ModReader::open_peek(path, vec![]).ok().map(|r| Mod::from_reader(r).hash)
}

pub struct ModIterator<'a> {
    profile: MappedRef<'a, String, Profile, Profile>,
    index:   usize,
}

impl<'a> Iterator for ModIterator<'a> {
    type Item = Mod;

    fn next(&mut self) -> Option<Self::Item> {
        let loads = self.profile.load_order();
        let mods = self.profile.mods();
        while let Some(hash) = loads.get(self.index) {
            self.index += 1;
            if let Some(mod_) = mods.get(hash) {
                return Some(mod_.clone());
            }
        }
        None
    }
}

#[derive(Debug)]
pub struct Manager {
    dir: PathBuf,
    profiles: DashMap<String, Profile>,
    current_profile: String,
    settings: Weak<RwLock<Settings>>,
}

impl Manager {
    #[inline(always)]
    pub fn path(&self) -> PathBuf {
        self.dir.join(self.current_profile.as_str())
    }

    fn profile_named(&self, name: &str) -> MappedRef<'_, String, Profile, Profile> {
        loop {
            if let Some(profile) = self.profiles.get(name) {
                return profile.map(|f| f);
            }
            log::warn!("Profile {name} is missing, using an empty one");
            self.profiles.entry(name.into()).or_default();
        }
    }

    #[inline(always)]
    pub fn profile(&self) -> MappedRef<'_, String, Profile, Profile> {
        self.profile_named(self.current_profile.as_str())
    }

    #[inline(always)]
    pub fn get_profile(&self, profile: Option<&String>) -> MappedRef<'_, String, Profile, Profile> {
        let profile = profile.unwrap_or(&self.current_profile);
        self.profile_named(profile.as_str())
    }

    pub fn forget_profile(&self, name: &str) {
        if name != self.current_profile {
            self.profiles.remove(name);
        }
    }

    pub fn ensure_profile(&self, name: &str) -> Result<()> {
        if self.profiles.contains_key(name) {
            return Ok(());
        }
        let path = self.dir.join(name).join("profile.yml");
        if !path.is_file() {
            anyhow_ext::bail!("The profile {name} doesn't exist");
        }
        self.add_profile(name.into())
    }

    pub fn rename_profile(&mut self, old: &str, new: &str) -> Result<()> {
        let new = new.trim();
        if new.is_empty() || new == old {
            return Ok(());
        }
        let (from, to) = (self.dir.join(old), self.dir.join(new));
        let only_case_differs = old.eq_ignore_ascii_case(new);
        if to.exists() && !only_case_differs {
            anyhow_ext::bail!("A profile named {new} already exists");
        }
        if old == self.current_profile && from.is_dir() {
            self.save().context("Couldn't save the profile before renaming it")?;
        }
        fs::rename(&from, &to).with_context(|| format!("Couldn't rename the profile {old}"))?;
        if let Some((_, profile)) = self.profiles.remove(old) {
            self.profiles.insert(new.into(), profile);
        }
        if self.current_profile == old {
            self.current_profile = new.into();
        }
        log::info!("Renamed profile {old} to {new}");
        Ok(())
    }

    fn path_used_elsewhere(&self, path: &Path, except: &str) -> bool {
        let key = path_key(path);
        self.profiles.iter().any(|p| {
            p.key().as_str() != except && p.value().mods().values().any(|m| path_key(&m.path) == key)
        })
    }

    fn free_stored_path(mods_dir: &Path, sanitized: &str, hash: Option<usize>) -> PathBuf {
        let mut k = 2;
        loop {
            let candidate = mods_dir.join(format!("{sanitized}-{k}.zip"));
            if !candidate.exists() || (hash.is_some() && meta_hash(&candidate) == hash) {
                return candidate;
            }
            k += 1;
        }
    }

    pub fn add_profile(&self, name: String) -> Result<()> {
        let path = self
            .settings
            .upgrade()
            .context("Settings are unavailable")?
            .read()
            .profiles_dir()
            .join(name.as_str())
            .join("profile.yml");
        let text = fs_err::read_to_string(&path).context("Couldn't read the copied profile")?;
        let profile = serde_yaml::from_str(&text).context("The copied profile has errors")?;
        self.profiles.insert(name, profile);
        Ok(())
    }

    pub fn create_profile_if(&self, profile: &str) -> Result<()> {
        let path = self.dir.join(profile);
        if !path.exists() {
            log::info!("Profile {profile} does not exist, creating it now");
            fs::create_dir_all(path)?;
            self.profiles.insert(profile.into(), Default::default());
            self.save()?;
        }
        Ok(())
    }

    pub fn set_profile(&mut self, profile: &str) -> Result<()> {
        if self.current_profile != profile && self.path().is_dir() && self.profiles.contains_key(self.current_profile.as_str()) {
            self.save().context("Couldn't save the current profile before switching")?;
        }
        self.current_profile = profile.into();
        self.create_profile_if(profile)?;
        Ok(())
    }

    pub fn init(settings: &Arc<RwLock<Settings>>) -> Result<Self> {
        log::info!("Initializing mod manager");
        let all_mods = glob::glob(
            &settings.read().mods_dir().join("*.zip").to_string_lossy()
        )?.map(|p| {
            let mod_ = Mod::from_reader(ModReader::open(p?, vec![])?);
            Ok((path_key(&mod_.path), mod_))
        }).collect::<Result<HashMap<String, Mod>>>()?;
        let current_profile = settings
            .read()
            .platform_config()
            .as_ref()
            .map(|c| c.profile.clone())
            .unwrap_or_else(|| "Default".into());
        log::info!("Current profile: {}", current_profile);
        let path = settings.read().profiles_dir();
        let profiles = settings
            .read()
            .profiles()
            .map(|profile| {
                let profile_path = path.join(profile.as_str()).join("profile.yml");
                fs::read_to_string(&profile_path)
                    .with_context(|| format!(
                        "Failed to read profile data from {}",
                        profile_path.to_string_lossy()
                    ))
                    .and_then(|t|
                        serde_yaml::from_str::<Profile>(&t)
                            .with_context(|| format!(
                                "Failed to parse profile data from {}",
                                profile_path.to_string_lossy()
                            ))
                            .map(|mut p| {
                                p.validate(&all_mods);
                                p
                            })
                    )
                    .map(|v| (profile, v))
            })
            .collect::<Result<_>>()?;
        let self_ = Self {
            dir: path,
            profiles,
            current_profile: current_profile.clone(),
            settings: Arc::downgrade(settings),
        };
        self_.create_profile_if(&current_profile)?;
        Ok(self_)
    }

    pub fn save(&self) -> Result<()> {
        fs::write(
            self.path().join("profile.yml"),
            serde_yaml::to_string(self.profile().deref())?,
        )?;
        log::info!("Saved profile data");
        log::debug!("{:#?}", self.profile());
        Ok(())
    }

    pub fn set_custom_details(&self, hash: usize, name: &str, description: &str) -> Result<Mod> {
        let mut updated = None;
        let mut changed_profiles = vec![];
        for profile in self.profiles.iter() {
            if let Some(mod_) = profile.value().mods_mut().get_mut(&hash) {
                mod_.set_custom_details(name, description);
                updated.get_or_insert_with(|| mod_.clone());
                changed_profiles.push(profile.key().clone());
            }
        }
        let updated = updated.context("The mod isn't installed")?;
        for name in changed_profiles {
            let dir = self.dir.join(name.as_str());
            if !dir.is_dir() {
                continue;
            }
            let text = serde_yaml::to_string(self.profile_named(name.as_str()).deref())?;
            fs::write(dir.join("profile.yml"), text)
                .with_context(|| format!("Couldn't save the profile {name}"))?;
        }
        log::info!(
            "Set the details of {} (shown as {})",
            updated.meta.name,
            updated.display_name()
        );
        Ok(updated)
    }

    pub fn current_profile(&self) -> &str {
        self.current_profile.as_str()
    }

    pub fn all_mods(&self) -> ModIterator<'_> {
        Profile::iter(self.profile())
    }

    pub fn mods(&self) -> impl Iterator<Item = Mod> + '_ {
        self.all_mods().filter(|m| m.enabled)
    }

    pub fn mods_by_manifest<'a: 'm, 'm>(
        &'a self,
        ref_manifest: &'m Manifest,
    ) -> impl Iterator<Item = Mod> + 'm {
        let ref_edits_text = !ref_manifest.languages().is_empty();
        self.mods().filter(move |mod_| {
            match mod_.manifest() {
                Ok(manifest) => {
                    !ref_manifest.content_files.is_disjoint(&manifest.content_files)
                    || !ref_manifest.aoc_files.is_disjoint(&manifest.aoc_files)
                    || (ref_edits_text && !manifest.languages().is_empty())
                }
                Err(_) => false,
            }
        })
    }

    fn slim_stored(&self, path: &Path, name: &str) {
        if !path.is_file() {
            return;
        }
        let Some(dump) = self.settings.upgrade().and_then(|s| s.read().dump()) else { return };
        match crate::slim::slim_mod(path, &dump) {
            Ok(s) if s.total() > 0 => log::info!(
                "Removed what {name} copies unchanged from the game: {} files, {} layout files, {} textures",
                s.files,
                s.layout_files,
                s.textures
            ),
            Ok(_) => {}
            Err(e) => log::warn!("Could not remove unchanged game files from {name}: {e:?}"),
        }
    }

    pub fn add(&self, mod_path: &Path, profile: Option<&String>) -> Result<Mod> {
        let mut old_version = None;
        let (mod_name, new_hash) = {
            let peeker = ModReader::open_peek(mod_path, vec![])?;
            let name = peeker.meta.name.as_str();
            if let Some(mod_) =
                Profile::iter(self.get_profile(profile)).find(|m| m.meta.name == name)
            {
                if Version::parse(peeker.meta.version.as_str())
                    .and_then(|pv| Version::parse(mod_.meta.version.as_str()).map(|mv| pv > mv))
                    .map_err(|e| anyhow_ext::anyhow!("{e}"))
                    .context("Bad version strings")?
                {
                    log::info!("Updating {name} to version {}", peeker.meta.version);
                    old_version = Some(mod_);
                } else {
                    anyhow_ext::bail!("Mod \"{}\" already installed", peeker.meta.name);
                }
            }
            let name = peeker.meta.name.clone();
            (name, Some(Mod::from_reader(peeker).hash))
        };
        let san_opts: sfn::Options<Option<char>> = sfn::Options {
            url_safe: true,
            collapse_replacements: true,
            ..Default::default()
        };
        let sanitized = sfn::sanitise_with_options(&mod_name, &san_opts);
        let mods_dir = self
            .settings
            .upgrade()
            .context("Settings are unavailable")?
            .read()
            .mods_dir();
        let profile_name = profile.unwrap_or(&self.current_profile).to_string();
        let mut stored_path = mods_dir.join(sanitized.clone() + ".zip");
        if stored_path.exists() && meta_hash(&stored_path) != new_hash {
            let can_overwrite = old_version.as_ref().is_some_and(|old| {
                path_key(&old.path) == path_key(&stored_path)
                    && !self.path_used_elsewhere(&stored_path, &profile_name)
            });
            if !can_overwrite {
                stored_path = Self::free_stored_path(&mods_dir, &sanitized, new_hash);
                log::info!("Another version of {mod_name} is already stored, keeping this one as {}", stored_path.display());
            }
        }
        if stored_path.exists() && meta_hash(&stored_path) == new_hash {
            log::debug!("Mod already stored, no need to store it");
        } else {
            stored_path.parent().map(fs::create_dir_all).transpose()?;
            if mod_path.is_file() {
                fs::copy(mod_path, &stored_path).context("Failed to copy mod to storage folder")?;
            } else {
                util::copy_dir(mod_path, &stored_path)
                    .context("Failed to copy mod to storage folder")?;
            }
        }
        self.slim_stored(&stored_path, &mod_name);
        crate::origin::forget(&stored_path);
        let reader = ModReader::open_peek(&stored_path, vec![])?;
        let mut mod_ = Mod::from_reader(reader);
        mod_.enabled = true;
        let profile_data = self.get_profile(profile);
        profile_data.load_order_mut().push(mod_.hash);
        profile_data.mods_mut().insert(mod_.hash, mod_.clone());
        if let Some(old_mod) = old_version {
            let mut order = profile_data.load_order_mut();
            order.retain(|h| *h != mod_.hash);
            if let Some(idx) = order.iter().position(|h| *h == old_mod.hash) {
                order[idx] = mod_.hash;
            } else {
                order.push(mod_.hash);
            }
            drop(order);
            profile_data.mods_mut().remove(&old_mod.hash);
            drop(profile_data);
            if path_key(&old_mod.path) != path_key(&stored_path)
                && !self.path_used_elsewhere(&old_mod.path, "")
            {
                let removed = if old_mod.path.is_dir() {
                    util::remove_dir_all(&old_mod.path)
                } else {
                    fs::remove_file(&old_mod.path).map_err(Into::into)
                };
                match removed {
                    Ok(()) => crate::origin::forget(&old_mod.path),
                    Err(e) => log::warn!("Couldn't remove the old version of {}: {e}", old_mod.meta.name),
                }
            }
            let profile_data = self.get_profile(profile);
            mod_.enabled = old_mod.enabled;
            mod_.keep_custom_details(&old_mod);
            if let Some(m) = profile_data.mods_mut().get_mut(&mod_.hash) {
                m.enabled = old_mod.enabled;
                m.keep_custom_details(&old_mod);
            }
            log::info!(
                "Updated mod {} in profile {} to version {}",
                mod_.meta.name,
                profile.unwrap_or(&self.current_profile).as_str(),
                mod_.meta.version
            );
        } else {
            log::info!(
                "Added mod {} to profile {}",
                mod_.meta.name,
                profile.unwrap_or(&self.current_profile).as_str()
            );
        }
        log::debug!("{:#?}", mod_);
        Ok(mod_)
    }

    pub fn del(&self, mod_: impl LookupMod, profile: Option<&String>) -> Result<Arc<Manifest>> {
        let hash = mod_.as_map_id();
        let profile_data = self.get_profile(profile);
        let mod_ = profile_data.mods_mut().remove(&hash);
        if let Some(mod_) = mod_ {
            let manifest = mod_.manifest()?;

            if !self
                .profiles
                .iter()
                .any(|p| p.value().mods().contains_key(&hash))
            {
                if mod_.path.is_dir() {
                    util::remove_dir_all(&mod_.path)?;
                } else {
                    fs::remove_file(&mod_.path)?;
                }
                crate::origin::forget(&mod_.path);
            }
            profile_data.load_order_mut().retain(|m| m != &hash);
            log::info!(
                "Deleted mod {} from profile {}",
                mod_.meta.name,
                profile.unwrap_or(&self.current_profile).as_str()
            );
            Ok(manifest)
        } else {
            log::warn!("Mod with ID {} does not exist, doing nothing", hash);
            Ok(Default::default())
        }
    }

    pub fn replace(&self, mut mod_: Mod, old_hash: usize) -> Result<Mod> {
        let old_path = self
            .profile()
            .mods()
            .get(&old_hash)
            .map(|m| m.path.clone())
            .context("The mod being updated isn't in this profile")?;
        let mut stored_path = old_path.clone();
        if mod_.path != old_path && mod_.path.is_file() {
            let shared = self.path_used_elsewhere(&old_path, &self.current_profile);
            if shared || old_path.is_dir() {
                let mods_dir = old_path.parent().context("The stored mod has no folder")?;
                let stem = old_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "mod".into());
                stored_path = Self::free_stored_path(mods_dir, &stem, Some(mod_.hash));
            }
            fs::copy(&mod_.path, &stored_path).context("Failed to copy the updated mod to the storage folder")?;
            self.slim_stored(&stored_path, &mod_.meta.name);
            crate::origin::forget(&stored_path);
        }
        let profile_data = self.profile();
        let old_mod = profile_data
            .mods_mut()
            .remove(&old_hash)
            .context("The mod being updated isn't in this profile")?;
        mod_.enabled = old_mod.enabled;
        mod_.keep_custom_details(&old_mod);
        mod_.path = stored_path;
        profile_data.mods_mut().insert(mod_.hash, mod_.clone());
        let mut load_order = profile_data.load_order_mut();
        load_order.retain(|m| *m != mod_.hash || old_hash == mod_.hash);
        if let Some(idx) = load_order.iter().position(|m| *m == old_hash) {
            load_order[idx] = mod_.hash;
        } else {
            load_order.push(mod_.hash);
        }
        Ok(mod_)
    }

    pub fn set_enabled(
        &self,
        mod_: impl LookupMod,
        enabled: bool,
        profile: Option<&String>,
    ) -> Result<Arc<Manifest>> {
        let hash = mod_.as_map_id();
        let manifest;
        let profile_data = self.get_profile(profile);
        if let Some(mod_) = profile_data.mods_mut().get_mut(&hash) {
            mod_.enabled = enabled;
            manifest = mod_.manifest()?;
            log::info!(
                "{} mod {} in profile {}",
                if enabled { "Enabled" } else { "Disabled" },
                mod_.meta.name,
                profile.unwrap_or(&self.current_profile).as_str()
            );
        } else {
            log::warn!("Mod with ID {} does not exist, doing nothing", hash);
            return Ok(Default::default());
        }
        Ok(manifest)
    }

    pub fn set_enabled_options(
        &self,
        mod_: impl LookupMod,
        options: Vec<ModOption>,
    ) -> Result<Arc<Manifest>> {
        let hash = mod_.as_map_id();
        let manifest;
        if let Some(mod_) = self.profile().mods_mut().get_mut(&hash) {
            manifest = mod_.manifest_with_options(&options)?;
            mod_.enabled_options = options;
        } else {
            log::warn!("Mod with ID {} does not exist, doing nothing", hash);
            return Ok(Default::default());
        }
        Ok(manifest)
    }

    pub fn set_order(&self, order: Vec<usize>) {
        *self.profile().load_order_mut() = order;
    }

    pub fn get_mod(&self, hash: usize) -> Option<Mod> {
        self.profile().mods().get(&hash).cloned()
    }
}
