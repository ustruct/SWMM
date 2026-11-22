use std::{
    io::{Read, Write},
    path::Path,
};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uk_mod::ModOption;

use crate::core::Manager;

pub const EXTENSION: &str = "swmmpack";
const INDEX: &str = "modpack.json";
const FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackEntry {
    pub file: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    #[serde(default)]
    pub options: Vec<ModOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackIndex {
    pub format: u32,
    pub name: String,
    pub mods: Vec<PackEntry>,
}

pub fn export(core: &Manager, profile: &str, dest: &Path) -> Result<usize> {
    let mods: Vec<_> = {
        let manager = core.mod_manager();
        manager
            .ensure_profile(profile)
            .with_context(|| format!("The profile {profile} couldn't be loaded"))?;
        let name: smartstring::alias::String = profile.into();
        let data = manager.get_profile(Some(&name));
        let all = data.mods();
        data.load_order().iter().filter_map(|h| all.get(h).cloned()).collect()
    };
    if mods.is_empty() {
        anyhow_ext::bail!("The profile {profile} has no mods to share");
    }
    let partial = dest.with_extension(format!("{EXTENSION}.partial"));
    let file = fs::File::create(&partial)?;
    let mut zip = zip::ZipWriter::new(file);
    let mut entries = Vec::with_capacity(mods.len());
    for (i, mod_) in mods.iter().enumerate() {
        let file = format!("mods/{i:03}.zip");
        let added = add_mod(&mut zip, &file, &mod_.path).with_context(|| format!("Couldn't add {} to the modpack", mod_.meta.name));
        if let Err(e) = added {
            drop(zip);
            fs::remove_file(&partial).unwrap_or(());
            return Err(e);
        }
        entries.push(PackEntry {
            file,
            name: mod_.meta.name.to_string(),
            version: mod_.meta.version.to_string(),
            enabled: mod_.enabled,
            options: mod_.enabled_options.clone(),
        });
    }
    let index = PackIndex {
        format: FORMAT,
        name: profile.to_string(),
        mods: entries,
    };
    let finished = (|| -> Result<()> {
        zip.start_file(INDEX, zip::write::SimpleFileOptions::default())?;
        zip.write_all(serde_json::to_string_pretty(&index)?.as_bytes())?;
        zip.finish()?;
        fs::rename(&partial, dest)?;
        Ok(())
    })();
    if let Err(e) = finished {
        fs::remove_file(&partial).unwrap_or(());
        return Err(e).context("Couldn't write the modpack");
    }
    log::info!("Shared {} mods from {profile} as {}", mods.len(), dest.display());
    Ok(mods.len())
}

fn add_mod(zip: &mut zip::ZipWriter<fs::File>, name: &str, path: &Path) -> Result<()> {
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    if path.is_file() {
        zip.start_file(name, stored)?;
        std::io::copy(&mut fs::File::open(path)?, zip)?;
        return Ok(());
    }
    if !path.is_dir() {
        anyhow_ext::bail!("Its files are missing from {}. Reinstall it, then share the profile again.", path.display());
    }
    let mut inner = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for file in crate::import::files_in(path) {
        let rel = file.strip_prefix(path)?.to_string_lossy().replace('\\', "/");
        inner.start_file(rel, stored)?;
        std::io::copy(&mut fs::File::open(&file)?, &mut inner)?;
    }
    let bytes = inner.finish()?.into_inner();
    zip.start_file(name, stored)?;
    zip.write_all(&bytes)?;
    Ok(())
}

fn known_options(mod_: &crate::mods::Mod, wanted: &[ModOption]) -> Vec<ModOption> {
    let offered: Vec<&ModOption> = mod_
        .meta
        .options
        .iter()
        .flat_map(|group| match group {
            uk_mod::OptionGroup::Exclusive(g) => g.options.iter(),
            uk_mod::OptionGroup::Multiple(g) => g.options.iter(),
        })
        .collect();
    wanted
        .iter()
        .filter_map(|w| offered.iter().find(|o| o.path == w.path).map(|o| (*o).clone()))
        .collect()
}

pub fn read_index(path: &Path) -> Result<PackIndex> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?).context("This file is not a SWMM modpack")?;
    let mut text = String::new();
    zip.by_name(INDEX)
        .context("This file is not a SWMM modpack")?
        .read_to_string(&mut text)?;
    let index: PackIndex = serde_json::from_str(&text).context("The modpack's list of mods can't be read")?;
    if index.format > FORMAT {
        anyhow_ext::bail!("This modpack was made by a newer SWMM. Update SWMM to open it.");
    }
    Ok(index)
}

pub fn free_profile_name(core: &Manager, wanted: &str) -> String {
    let wanted = wanted.trim();
    let wanted = if wanted.is_empty() { "Modpack" } else { wanted };
    let taken: Vec<String> = core.settings().profiles().map(|p| p.to_string()).collect();
    let mut name = wanted.to_string();
    let mut k = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{wanted} ({k})");
        k += 1;
    }
    name
}

pub fn import(core: &Manager, path: &Path) -> Result<String> {
    let index = read_index(path)?;
    if index.mods.is_empty() {
        anyhow_ext::bail!("This modpack has no mods in it");
    }
    let profile = free_profile_name(core, &index.name);
    let previous = core.mod_manager().current_profile().to_string();
    let temp = crate::util::new_temp_dir()?;
    core.change_profile(&profile)?;
    let result = install_all(core, path, &index, &temp);
    crate::util::release_temp(&temp);
    match result {
        Ok(()) => {
            log::info!("Imported the modpack {} as profile {profile}", index.name);
            Ok(profile)
        }
        Err(e) => {
            core.change_profile(&previous).unwrap_or(());
            crate::util::remove_dir_all(core.settings().profiles_dir().join(&profile)).unwrap_or(());
            core.mod_manager().forget_profile(&profile);
            Err(e)
        }
    }
}

fn install_all(core: &Manager, path: &Path, index: &PackIndex, temp: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut order = Vec::with_capacity(index.mods.len());
    for (i, entry) in index.mods.iter().enumerate() {
        log::info!("Installing {} ({} of {})", entry.name, i + 1, index.mods.len());
        let package = temp.join(format!("{i:03}.zip"));
        {
            let mut src = zip
                .by_name(&entry.file)
                .with_context(|| format!("The modpack is missing {}", entry.name))?;
            let mut out = fs::File::create(&package)?;
            std::io::copy(&mut src, &mut out)?;
        }
        let manager = core.mod_manager();
        if manager.all_mods().any(|m| m.meta.name.as_str() == entry.name) {
            log::warn!("The modpack has {} twice, keeping the first one", entry.name);
            fs::remove_file(&package).unwrap_or(());
            continue;
        }
        let mod_ = manager
            .add(&package, None)
            .with_context(|| format!("Failed to install {}", entry.name))?;
        fs::remove_file(&package).unwrap_or(());
        let options = known_options(&mod_, &entry.options);
        if options.len() != entry.options.len() {
            log::warn!("Some options saved for {} don't exist in this version, they were left off", entry.name);
        }
        if !options.is_empty() {
            manager
                .set_enabled_options(mod_.hash(), options)
                .with_context(|| format!("Failed to select the options of {}", entry.name))?;
        }
        manager.set_enabled(mod_.hash(), entry.enabled, None)?;
        order.push(mod_.hash());
    }
    let manager = core.mod_manager();
    if order.is_empty() {
        anyhow_ext::bail!("None of the modpack's mods could be installed");
    }
    manager.set_order(order);
    manager.save()?;
    Ok(())
}
