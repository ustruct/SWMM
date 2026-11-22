use std::{collections::BTreeMap, path::Path};

use crate::mods::Mod;

const MERGED_EXTENSIONS: &[&str] = &[
    "sbactorpack",
    "pack",
    "sblarc",
    "ssarc",
    "sarc",
    "sbeventpack",
    "sbquestpack",
    "sbgamedata",
    "sbyml",
    "byml",
    "smubin",
    "bgdata",
    "srsizetable",
    "sbactorinfo",
    "sbtcl",
    "sbmsg",
    "bxml",
    "baglblm",
    "baglenv",
    "bgparamlist",
    "bshop",
    "bdrop",
    "brecipe",
    "bxlink",
    "bars",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFile {
    pub path: String,
    pub merged: bool,
}

#[derive(Debug, Clone)]
pub struct Overlap {
    pub winner: usize,
    pub loser: usize,
    pub files: Vec<SharedFile>,
}

impl Overlap {
    pub fn replaced(&self) -> usize {
        self.files.iter().filter(|f| !f.merged).count()
    }
}

pub fn is_merged(path: &str) -> bool {
    let name = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
    name.rsplit('.')
        .next()
        .map(|ext| MERGED_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

pub fn overlaps(mods: &[Mod]) -> Vec<Overlap> {
    let mut owners: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, mod_) in mods.iter().enumerate().filter(|(_, m)| m.enabled) {
        let Ok(manifest) = mod_.manifest() else {
            continue;
        };
        for file in manifest.content_files.iter() {
            owners.entry(format!("content/{file}")).or_default().push(index);
        }
        for file in manifest.aoc_files.iter() {
            owners.entry(format!("aoc/{file}")).or_default().push(index);
        }
    }
    let mut pairs: BTreeMap<(usize, usize), Vec<SharedFile>> = BTreeMap::new();
    for (path, indices) in owners.into_iter().filter(|(_, v)| v.len() > 1) {
        let merged = is_merged(&path);
        for (i, &lower) in indices.iter().enumerate() {
            for &higher in &indices[i + 1..] {
                pairs.entry((higher, lower)).or_default().push(SharedFile {
                    path: path.clone(),
                    merged,
                });
            }
        }
    }
    pairs
        .into_iter()
        .map(|((winner, loser), files)| Overlap {
            winner: mods[winner].hash(),
            loser: mods[loser].hash(),
            files,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_files() {
        assert!(is_merged("content/Actor/Pack/Link.sbactorpack"));
        assert!(is_merged("content/Layout/Common.sblarc"));
        assert!(!is_merged("content/Model/Link.sbfres"));
        assert!(!is_merged("content/Movie/Demo.mp4"));
    }
}
