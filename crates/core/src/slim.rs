use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::Path,
};

use anyhow_ext::{Context, Result};
use uk_content::{
    canonicalize,
    layout::{LayoutArchive, bntx::Bntx},
    resource::{MergeableResource, ResourceData},
};
use uk_mod::Manifest;
use uk_reader::ResourceReader;

const MARK: &str = "swmm-slim-1";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Slimmed {
    pub files: usize,
    pub layout_files: usize,
    pub textures: usize,
}

impl Slimmed {
    pub fn total(&self) -> usize {
        self.files + self.layout_files + self.textures
    }
}

pub fn is_slim(path: &Path) -> bool {
    fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|z| z.comment() == MARK.as_bytes())
}

fn stock_bytes(dump: &ResourceReader, canon: &str) -> Option<Vec<u8>> {
    dump.get_bytes_uncached(canon)
        .ok()
        .map(|d| roead::yaz0::decompress_if(&d).into_owned())
}

pub fn slim_layout(la: &mut LayoutArchive, stock: &LayoutArchive, s: &mut Slimmed) {
    la.0.retain(|name, data| {
        let Some(base) = stock.0.get(name) else { return true };
        if base == data {
            s.layout_files += 1;
            return false;
        }
        if !name.ends_with(".bntx") {
            return true;
        }
        let (Some(mut ours), Some(theirs)) = (Bntx::parse(data), Bntx::parse(base)) else { return true };
        let before = ours.textures.len();
        ours.textures.retain(|t| !theirs.textures.contains(t));
        let gone = before - ours.textures.len();
        s.textures += gone;
        if ours.textures.is_empty() {
            s.layout_files += 1;
            return false;
        }
        if gone > 0 {
            *data = ours.to_binary();
        }
        true
    });
}

fn split_option(name: &str) -> (String, String) {
    match name.strip_prefix("options/").and_then(|r| r.split_once('/')) {
        Some((opt, rest)) => (format!("options/{opt}/"), rest.to_string()),
        None => (String::new(), name.to_string()),
    }
}

fn encode(res: &ResourceData) -> Result<Vec<u8>> {
    let bytes = minicbor_ser::to_vec(res).map_err(|e| anyhow_ext::anyhow!("Failed to encode a mod file: {e:?}"))?;
    Ok(uk_mod::zstd::encode_all(bytes.as_slice(), 8)?)
}

pub fn slim_mod(path: &Path, dump: &ResourceReader) -> Result<Slimmed> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?).context("Mod package isn't a zip")?;
    if zip.comment() == MARK.as_bytes() {
        return Ok(Slimmed::default());
    }
    let decomp = uk_mod::unpack::init_decompressor();
    let mut s = Slimmed::default();
    let mut replaced: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    let mut dropped: HashMap<String, HashSet<String>> = HashMap::new();
    let in_root: HashSet<String> = zip.file_names().filter(|n| !n.starts_with("options/")).map(String::from).collect();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        if f.is_dir() || name.ends_with(".yml") || name.ends_with("thumb.jpg") {
            continue;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        let raw = {
            let mut d = decomp.lock();
            let size = uk_mod::zstd::bulk::Decompressor::upper_bound(&buf).unwrap_or(buf.len() * 1024);
            match d.decompress(&buf, size).ok().or_else(|| uk_mod::zstd::decode_all(buf.as_slice()).ok()) {
                Some(raw) => raw,
                None => continue,
            }
        };
        let Ok(res) = minicbor_ser::from_slice::<ResourceData>(&raw) else { continue };
        let (prefix, canon) = split_option(&name);
        if !prefix.is_empty() && in_root.contains(&canon) {
            continue;
        }
        match res {
            ResourceData::Binary(data) => {
                if stock_bytes(dump, &canon).is_some_and(|stock| stock == data) {
                    s.files += 1;
                    replaced.insert(name, None);
                    dropped.entry(prefix).or_default().insert(canon);
                }
            }
            ResourceData::Mergeable(MergeableResource::LayoutArchive(mut la)) => {
                let Ok(stock) = dump.get_data(&canon) else { continue };
                let Some(MergeableResource::LayoutArchive(stock)) = stock.as_mergeable() else { continue };
                let before = s;
                slim_layout(&mut la, stock, &mut s);
                if la.0.is_empty() {
                    replaced.insert(name, None);
                    dropped.entry(prefix).or_default().insert(canon);
                } else if s != before {
                    let bytes = encode(&ResourceData::Mergeable(MergeableResource::LayoutArchive(la)))?;
                    replaced.insert(name, Some(bytes));
                }
            }
            _ => {}
        }
    }
    for (prefix, canons) in &dropped {
        let manifest_name = format!("{prefix}manifest.yml");
        let Ok(mut f) = zip.by_name(&manifest_name) else { continue };
        let mut text = String::new();
        f.read_to_string(&mut text)?;
        drop(f);
        let Ok(mut manifest) = serde_yaml::from_str::<Manifest>(&text) else { continue };
        manifest.content_files.retain(|f| !canons.contains(canonicalize(f.as_str()).as_str()));
        manifest.aoc_files.retain(|f| {
            !canons.contains(canonicalize(f.as_str()).as_str())
                && !canons.contains(canonicalize(format!("Aoc/0010/{f}")).as_str())
        });
        replaced.insert(manifest_name, Some(serde_yaml::to_string(&manifest)?.into_bytes()));
    }
    let tmp = path.with_extension("zip.slim");
    {
        let mut out = zip::ZipWriter::new(fs::File::create(&tmp)?);
        for i in 0..zip.len() {
            let name = zip.by_index_raw(i)?.name().to_string();
            match replaced.get(&name) {
                Some(None) => {}
                Some(Some(bytes)) => {
                    let method = zip.by_index_raw(i)?.compression();
                    out.start_file(name, zip::write::SimpleFileOptions::default().compression_method(method))?;
                    out.write_all(bytes)?;
                }
                None => out.raw_copy_file(zip.by_index_raw(i)?)?,
            }
        }
        out.set_comment(MARK);
        out.finish()?;
    }
    drop(zip);
    fs::rename(&tmp, path).context("Failed to replace the mod package")?;
    Ok(s)
}
