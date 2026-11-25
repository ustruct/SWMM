use std::{
    cell::Cell,
    collections::HashMap,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use parking_lot::RwLock;
use uk_content::prelude::Endian;
use uk_mod::ModPlatform;

use crate::mods::Mod;

pub const FILE: &str = "swmm-origin.yml";
pub const LIST_NAME: &str = "savagewind-mods.txt";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    WiiU,
    Switch,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::WiiU => "Wii U",
            Origin::Switch => "Switch",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Origin::WiiU => "wiiu",
            Origin::Switch => "switch",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "wiiu" => Some(Origin::WiiU),
            "switch" => Some(Origin::Switch),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Bnp,
    GameFiles,
    Packaged,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Bnp => "BNP",
            Kind::GameFiles => "game files",
            Kind::Packaged => "packaged",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Kind::Bnp => "bnp",
            Kind::GameFiles => "files",
            Kind::Packaged => "packaged",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "bnp" => Some(Kind::Bnp),
            "files" => Some(Kind::GameFiles),
            "packaged" => Some(Kind::Packaged),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    pub origin: Origin,
    pub kind: Option<Kind>,
}

impl Marker {
    fn text(self) -> String {
        match self.kind {
            Some(kind) => format!("origin: {}\nkind: {}\n", self.origin.key(), kind.key()),
            None => format!("origin: {}\n", self.origin.key()),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        if let Some(origin) = Origin::parse(text) {
            return Some(Marker { origin, kind: None });
        }
        let mut origin = None;
        let mut kind = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else { continue };
            match key.trim() {
                "origin" => origin = Origin::parse(value),
                "kind" => kind = Kind::parse(value),
                _ => {}
            }
        }
        origin.map(|origin| Marker { origin, kind })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub wiiu: bool,
    pub own_info: bool,
    pub kind: Option<Kind>,
    pub rebased: bool,
    pub code_patches: bool,
}

impl Notes {
    pub fn marker(self) -> Marker {
        Marker {
            origin: if self.wiiu { Origin::WiiU } else { Origin::Switch },
            kind: self.kind,
        }
    }
}

thread_local! {
    static NOTES: Cell<Notes> = const { Cell::new(Notes { wiiu: false, own_info: false, kind: None, rebased: false, code_patches: false }) };
}

pub fn reset() {
    NOTES.with(|n| n.set(Notes::default()));
}

pub fn take() -> Notes {
    NOTES.with(|n| n.replace(Notes::default()))
}

pub fn snapshot() -> Notes {
    NOTES.with(Cell::get)
}

pub fn restore(notes: Notes) {
    NOTES.with(|n| n.set(notes));
}

pub fn note_wiiu() {
    NOTES.with(|n| n.set(Notes { wiiu: true, ..n.get() }));
}

pub fn note_own_info() {
    NOTES.with(|n| n.set(Notes { own_info: true, ..n.get() }));
}

pub fn note_rebased() {
    NOTES.with(|n| n.set(Notes { rebased: true, ..n.get() }));
}

pub fn note_code_patches() {
    NOTES.with(|n| n.set(Notes { code_patches: true, ..n.get() }));
}

pub fn note_kind(kind: Kind) {
    NOTES.with(|n| {
        let notes = n.get();
        if notes.kind.is_none() {
            n.set(Notes { kind: Some(kind), ..notes });
        }
    });
}

static CACHE: LazyLock<RwLock<HashMap<PathBuf, Option<Marker>>>> = LazyLock::new(Default::default);

pub fn forget(path: &Path) {
    CACHE.write().remove(path);
}

fn read_marker(path: &Path) -> Option<Marker> {
    if path.is_dir() {
        return fs::read_to_string(path.join(FILE)).ok().and_then(|t| Marker::parse(&t));
    }
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).ok()?;
    let mut entry = zip.by_name(FILE).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Marker::parse(&text)
}

fn marker_of(mod_: &Mod) -> Option<Marker> {
    if let Some(found) = CACHE.read().get(&mod_.path) {
        return *found;
    }
    let found = read_marker(&mod_.path);
    CACHE.write().insert(mod_.path.clone(), found);
    found
}

pub fn of(mod_: &Mod) -> Origin {
    if mod_.meta.platform == ModPlatform::Specific(Endian::Big) {
        return Origin::WiiU;
    }
    marker_of(mod_).map_or(Origin::Switch, |m| m.origin)
}

pub fn describe(mod_: &Mod) -> String {
    let kind = marker_of(mod_).and_then(|m| m.kind).unwrap_or(Kind::Packaged);
    format!("{}, {}", kind.label(), of(mod_).label())
}

pub fn mark(path: &Path, marker: Marker) -> Result<()> {
    if path.is_dir() {
        fs::write(path.join(FILE), marker.text())?;
        forget(path);
        return Ok(());
    }
    let temp = path.with_extension("origin.partial");
    {
        let mut zip = zip::ZipArchive::new(BufReader::new(fs::File::open(path)?))
            .context("The mod isn't a valid zip")?;
        let mut writer = zip::ZipWriter::new(fs::File::create(&temp)?);
        let comment = zip.comment().to_vec();
        for i in 0..zip.len() {
            let file = zip.by_index_raw(i)?;
            if file.name() == FILE {
                continue;
            }
            writer.raw_copy_file(file)?;
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file(FILE, options)?;
        writer.write_all(marker.text().as_bytes())?;
        if !comment.is_empty() {
            writer.set_raw_comment(comment.into_boxed_slice());
        }
        writer.finish()?;
    }
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })?;
    forget(path);
    Ok(())
}

fn clean(text: &str) -> String {
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    text.trim().trim_start_matches('#').trim().to_string()
}

pub fn list(profile: &str, mods: &[Mod]) -> String {
    let mut out = format!("# SWMM {}, profile {}\n", env!("CARGO_PKG_VERSION"), clean(profile));
    for mod_ in mods.iter().rev() {
        let meta = &mod_.meta;
        let mut line = format!("{} {} ({})", clean(&meta.name), clean(&meta.version), describe(mod_));
        if !mod_.enabled_options.is_empty() {
            let options: Vec<String> = mod_.enabled_options.iter().map(|o| clean(&o.name)).collect();
            line.push_str(&format!(" [options: {}]", options.join(", ")));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub fn write_list(content: &Path, profile: &str, mods: &[Mod]) {
    let path = content.join(LIST_NAME);
    let result = if mods.is_empty() {
        if path.is_file() { fs::remove_file(&path) } else { Ok(()) }
    } else {
        fs::create_dir_all(content).and_then(|_| fs::write(&path, list(profile, mods)))
    };
    if let Err(e) = result {
        log::warn!("Could not write the list of mods for Savage Wind's log: {e}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_reads() {
        let dir = std::env::temp_dir().join(format!("swmm-origin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mod.zip");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            writer.start_file("meta.yml", zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(b"name: test").unwrap();
            writer.set_raw_comment(b"keep".to_vec().into_boxed_slice());
            writer.finish().unwrap();
        }
        assert_eq!(read_marker(&path), None);
        let wiiu_bnp = Marker { origin: Origin::WiiU, kind: Some(Kind::Bnp) };
        mark(&path, wiiu_bnp).unwrap();
        assert_eq!(read_marker(&path), Some(wiiu_bnp));
        let switch = Marker { origin: Origin::Switch, kind: None };
        mark(&path, switch).unwrap();
        assert_eq!(read_marker(&path), Some(switch));
        let zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.comment(), b"keep");
        assert_eq!(zip.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notes() {
        reset();
        note_own_info();
        assert_eq!(take(), Notes { own_info: true, ..Notes::default() });
        note_wiiu();
        note_kind(Kind::Bnp);
        note_kind(Kind::Packaged);
        assert_eq!(take(), Notes { wiiu: true, kind: Some(Kind::Bnp), ..Notes::default() });
        assert_eq!(take(), Notes::default());
    }

    #[test]
    fn old_markers_and_clean_text() {
        assert_eq!(Marker::parse("wiiu"), Some(Marker { origin: Origin::WiiU, kind: None }));
        assert_eq!(
            Marker::parse("origin: switch\nkind: files\n"),
            Some(Marker { origin: Origin::Switch, kind: Some(Kind::GameFiles) })
        );
        assert_eq!(Marker::parse("nonsense"), None);
        assert_eq!(clean("  # Linkle\r\nsecond line "), "Linkle  second line");
    }
}
