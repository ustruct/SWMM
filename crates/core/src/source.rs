use std::{
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use uk_mod::{Meta, ModCategory};

use crate::util;

#[derive(Clone, Default)]
pub struct SourceInfo {
    pub name: String,
    pub author: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub url: String,
    pub thumbnail: Option<Vec<u8>>,
}

impl std::fmt::Debug for SourceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceInfo")
            .field("name", &self.name)
            .field("author", &self.author)
            .field("version", &self.version)
            .field("category", &self.category)
            .field("url", &self.url)
            .field("thumbnail", &self.thumbnail.as_ref().map(Vec::len))
            .finish()
    }
}

fn category(name: &str) -> Option<ModCategory> {
    let name = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    Some(if has(&["skin", "texture", "model", "outfit", "visual", "graphic", "shader"]) {
        ModCategory::Texture
    } else if has(&["overhaul"]) {
        ModCategory::Overhaul
    } else if has(&["quality", "qol", "utilit", "interface", "hud", "performance", "fps"]) {
        ModCategory::QoL
    } else if has(&["anim"]) {
        ModCategory::Animations
    } else if has(&["difficult", "challenge", "hard"]) {
        ModCategory::Difficulty
    } else if has(&["enem", "monster", "boss"]) {
        ModCategory::Enemies
    } else if has(&["weapon", "armor", "armour", "item", "bow", "shield"]) {
        ModCategory::Items
    } else if has(&["craft", "cook", "recipe"]) {
        ModCategory::Crafting
    } else if has(&["quest", "story"]) {
        ModCategory::Quest
    } else if has(&["shrine", "dungeon"]) {
        ModCategory::Shrine
    } else if has(&["meme", "joke", "gimmick", "fun"]) {
        ModCategory::Meme
    } else if has(&["player", "link", "character"]) {
        ModCategory::Player
    } else if has(&["world", "map", "level", "environment"]) {
        ModCategory::Overworld
    } else if has(&["balance", "gameplay", "tweak"]) {
        ModCategory::Balance
    } else if has(&["custom"]) {
        ModCategory::Customization
    } else if has(&["expansion", "content", "dlc"]) {
        ModCategory::Expansion
    } else if has(&["other", "misc"]) {
        ModCategory::Other
    } else {
        return None;
    })
}

fn set<T: for<'a> From<&'a str>>(field: &mut T, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        *field = value.into();
    }
}

fn merge(mut meta: Meta, info: &SourceInfo) -> Meta {
    set(&mut meta.name, &info.name);
    set(&mut meta.author, &info.author);
    set(&mut meta.version, &info.version);
    set(&mut meta.description, &info.description);
    if !info.url.trim().is_empty() {
        meta.url = Some(info.url.trim().into());
    }
    if let Some(category) = category(&info.category) {
        meta.category = category;
    }
    meta
}

pub fn apply(mod_path: &Path, info: &SourceInfo) -> Result<PathBuf> {
    let meta = merge(Meta::from_mod(mod_path).context("Could not read the mod's details")?, info);
    let mut zip = zip::ZipArchive::new(BufReader::new(fs::File::open(mod_path)?))?;
    let out = util::get_temp_folder().join("mod.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&out)?);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i)?;
        let name = file.name().to_string();
        if name == "meta.yml" || (info.thumbnail.is_some() && name.starts_with("thumb.")) {
            continue;
        }
        writer.raw_copy_file(file)?;
    }
    if let Some(thumbnail) = &info.thumbnail {
        writer.start_file("thumb.jpg", options)?;
        writer.write_all(thumbnail)?;
    }
    writer.start_file("meta.yml", options)?;
    writer.write_all(serde_yaml::to_string(&meta)?.as_bytes())?;
    writer.finish()?;
    log::info!("Using the GameBanana details for {}", meta.name);
    Ok(out)
}
