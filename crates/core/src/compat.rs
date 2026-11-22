use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
};

use anyhow_ext::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Full,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    pub status: Status,
    #[serde(default)]
    pub left_out: usize,
    #[serde(default)]
    pub converted: usize,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub wiiu: bool,
}

impl Entry {
    pub fn date(&self) -> &str {
        if self.date.is_empty() { TESTED } else { &self.date }
    }

    pub fn url(&self) -> Option<String> {
        (self.id != 0).then(|| format!("https://gamebanana.com/mods/{}", self.id))
    }

    fn same(&self, other: &Entry) -> bool {
        self.id == other.id && self.file == other.file
    }
}

static LIST: LazyLock<Vec<Entry>> =
    LazyLock::new(|| serde_json::from_str::<Vec<Entry>>(include_str!("wiiu_compat.json")).unwrap_or_default());

static BY_ID: LazyLock<HashMap<u64, &'static Entry>> = LazyLock::new(|| LIST.iter().map(|e| (e.id, e)).collect());

pub const TESTED: &str = "2026-11-22";

pub fn bundled() -> &'static [Entry] {
    &LIST
}

pub fn wiiu(id: u64) -> Option<&'static Entry> {
    BY_ID.get(&id).copied()
}

pub fn lookup(id: u64) -> Option<Entry> {
    let found = MINE.read().ok().and_then(|list| list.iter().find(|e| e.id == id).cloned());
    found.or_else(|| wiiu(id).cloned())
}

fn user_file() -> PathBuf {
    crate::settings::Settings::config_dir().join("compat.json")
}

fn read_mine() -> Vec<Entry> {
    std::fs::read_to_string(user_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

static MINE: LazyLock<RwLock<Vec<Entry>>> = LazyLock::new(|| RwLock::new(read_mine()));

pub fn mine() -> Vec<Entry> {
    MINE.read().map(|list| list.clone()).unwrap_or_default()
}

fn update(change: impl FnOnce(&mut Vec<Entry>)) -> Result<()> {
    let mut list = read_mine();
    change(&mut list);
    save(&list)?;
    if let Ok(mut cache) = MINE.write() {
        *cache = list;
    }
    Ok(())
}

fn save(list: &[Entry]) -> Result<()> {
    let path = user_file();
    if let Some(dir) = path.parent() {
        fs_err::create_dir_all(dir)?;
    }
    fs_err::write(&path, serde_json::to_string_pretty(list)?).context("Couldn't save the compatibility results")?;
    Ok(())
}

pub fn remember(entry: &Entry) -> Result<()> {
    update(|list| {
        list.retain(|e| !e.same(entry));
        list.insert(0, entry.clone());
    })
}

pub fn forget(entry: &Entry) -> Result<()> {
    update(|list| list.retain(|e| !e.same(entry)))
}

pub struct Test {
    pub name: String,
    pub status: Status,
    pub converted: usize,
    pub textures: usize,
    pub stock: usize,
    pub not_needed: usize,
    pub left_out: Vec<(String, String)>,
    pub kinds: BTreeMap<String, usize>,
    pub wiiu: bool,
    pub rebased: bool,
    pub kind: Option<&'static str>,
    pub options: usize,
    pub error: Option<String>,
    pub error_chain: Vec<String>,
}

fn short(error: &anyhow_ext::Error) -> String {
    let text = format!("{error:?}");
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().to_string();
    let deepest = error.chain().last().map(|e| e.to_string()).unwrap_or_default();
    if deepest.is_empty() || first.contains(&deepest) { first } else { format!("{first}: {deepest}") }
}

pub fn reason_kind(reason: &str) -> String {
    let lower = reason.to_lowercase();
    if lower.contains("effect") {
        "effects".into()
    } else if lower.contains("shader") || lower.contains("can't convert from wii u yet") {
        "shaders and other formats".into()
    } else {
        reason.split('(').next().unwrap_or(reason).trim().to_string()
    }
}

fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(" at `") {
        out.push_str(&rest[..at]);
        match rest[at + 5..].find('`') {
            Some(end) => rest = &rest[at + 5 + end + 1..],
            None => {
                rest = &rest[at..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn kind_name(kind: &str, count: usize) -> String {
    let kind = clean(kind);
    let names = match kind.to_lowercase().as_str() {
        "effects" => Some(("effect file", "effect files")),
        "shaders and other formats" => Some(("file in a format SWMM can't convert", "files in formats SWMM can't convert")),
        "model swmm can't convert yet" => Some(("model SWMM can't convert yet", "models SWMM can't convert yet")),
        _ => None,
    };
    match names {
        Some((one, many)) => (if count == 1 { one } else { many }).to_string(),
        None => kind.replace("swmm", "SWMM"),
    }
}

impl Test {
    pub fn reason(&self) -> String {
        match self.status {
            Status::Full => String::new(),
            Status::Failed => {
                let chain: Vec<String> = if self.error_chain.is_empty() {
                    vec![clean(self.error.as_deref().unwrap_or_default())]
                } else {
                    self.error_chain.iter().map(|c| clean(c)).collect()
                };
                let last = chain.last().cloned().unwrap_or_default();
                let parsed = chain
                    .iter()
                    .filter(|c| c.starts_with("Failed to parse resource ") && !c.contains(" in SARC "))
                    .last();
                match parsed {
                    Some(p) => format!("{} can't be read: {last}", p.trim_start_matches("Failed to parse resource ")),
                    None => last,
                }
            }
            Status::Partial => {
                let mut kinds: Vec<_> = self.kinds.iter().collect();
                kinds.sort_by(|a, b| b.1.cmp(a.1));
                kinds.iter().map(|(k, n)| format!("{n} {}", kind_name(k, **n))).collect::<Vec<_>>().join(", ")
            }
        }
    }

    pub fn entry(&self, id: u64, name: &str, file: &str) -> Entry {
        let name = if id == 0 && !self.name.trim().is_empty() { self.name.trim() } else { name };
        Entry {
            id,
            name: name.to_string(),
            status: self.status,
            left_out: self.left_out.len(),
            converted: self.converted,
            file: file.to_string(),
            reason: self.reason(),
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            wiiu: self.wiiu,
        }
    }
}

pub fn test_file(core: &crate::core::Manager, path: &Path) -> Test {
    crate::util::begin_temp_scope();
    crate::origin::reset();
    let _ = crate::wiiu::take_summary();
    let outcome = crate::import::convert_any(core, path, None)
        .and_then(|p| uk_mod::unpack::ModReader::open_peek(&p, vec![]).map(|r| (r.meta.name.to_string(), r.meta.options.len())));
    let notes = crate::origin::take();
    let summary = crate::wiiu::take_summary();
    crate::util::end_temp_scope(None);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, reason) in &summary.skipped {
        *kinds.entry(reason_kind(reason)).or_default() += 1;
    }
    let (name, status, options, error, error_chain) = match outcome {
        Ok((name, options)) => {
            let status = if summary.skipped.is_empty() { Status::Full } else { Status::Partial };
            (name, status, options, None, vec![])
        }
        Err(e) => (String::new(), Status::Failed, 0, Some(short(&e)), e.chain().map(|c| c.to_string()).collect()),
    };
    Test {
        name,
        status,
        converted: summary.converted,
        textures: summary.textures,
        stock: summary.stock,
        not_needed: summary.not_needed,
        left_out: summary.skipped,
        kinds,
        wiiu: notes.wiiu,
        rebased: notes.rebased,
        kind: notes.kind.map(|k| k.label()),
        options,
        error,
        error_chain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_parses() {
        let text = include_str!("wiiu_compat.json");
        let entries: Vec<Entry> = serde_json::from_str(text).unwrap();
        assert_eq!(entries.len(), LIST.len());
        assert!(LIST.iter().all(|e| !e.name.is_empty()));
    }

    #[test]
    fn reasons_read_well() {
        assert_eq!(clean("Bad file at `C:\\x\\y.rs:4` here"), "Bad file here");
        let mut test = Test {
            name: String::new(),
            status: Status::Partial,
            converted: 3,
            textures: 0,
            stock: 0,
            not_needed: 0,
            left_out: vec![],
            kinds: BTreeMap::from([("effects".to_string(), 1), ("shaders and other formats".to_string(), 2)]),
            wiiu: true,
            rebased: false,
            kind: None,
            options: 0,
            error: None,
            error_chain: vec![],
        };
        assert_eq!(test.reason(), "2 files in formats SWMM can't convert, 1 effect file");
        test.status = Status::Failed;
        test.error_chain = vec!["Failed to parse resource Actor/Pack/A.sbactorpack".into(), "Bad magic".into()];
        assert_eq!(test.reason(), "Actor/Pack/A.sbactorpack can't be read: Bad magic");
    }
}
