use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow_ext::{Context, Result};
use fs_err as fs;
use uk_content::{prelude::Endian, resource::{MergeableResource, ResourceData}};
use uk_mod::{Manifest, Meta, ModPlatform};

use crate::wiiu::Report;

pub fn is_wiiu(dir: &Path) -> bool {
    fs::read_to_string(dir.join("meta.yml"))
        .ok()
        .and_then(|m| serde_yaml::from_str::<Meta>(&m).ok())
        .is_some_and(|m| m.platform == ModPlatform::Specific(Endian::Big))
}

const MIPS: u32 = 1000;

fn texture_part(name: &str) -> Option<(String, u32)> {
    let rest = name.strip_suffix(".sbfres").or_else(|| name.strip_suffix(".bfres"))?;
    for (kind, offset) in [(".Tex1", 0), (".Tex2", MIPS)] {
        if let Some(stem) = rest.strip_suffix(kind) {
            return Some((stem.to_string(), offset));
        }
        if let Some((stem, n)) = rest.rsplit_once(&format!("{kind}."))
            && let Some(n) = n.parse::<u32>().ok().filter(|&n| n > 0 && n < MIPS) {
                return Some((stem.to_string(), offset + n));
            }
    }
    None
}

fn switch_name_in(name: &str, firsts: &std::collections::HashMap<String, u32>) -> Option<String> {
    match texture_part(name) {
        Some((stem, n)) if firsts.get(&stem) == Some(&n) => {
            let ext = if name.ends_with(".sbfres") { "sbfres" } else { "bfres" };
            Some(format!("{stem}.Tex.{ext}"))
        }
        Some(_) => None,
        None => Some(name.to_string()),
    }
}

fn first_parts<'a>(names: impl Iterator<Item = &'a str>) -> std::collections::HashMap<String, u32> {
    let mut m: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for (stem, n) in names.filter_map(texture_part) {
        let e = m.entry(stem).or_insert(n);
        *e = (*e).min(n);
    }
    m
}

fn rename_list(entries: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let firsts = first_parts(entries.iter().map(|(f, _)| f.as_str()));
    entries.into_iter().filter_map(|(f, d)| switch_name_in(&f, &firsts).map(|n| (n, d))).collect()
}

pub fn convert_dir(dir: &Path, romfs: Option<&Path>) -> Result<Report> {
    let decomp = uk_mod::unpack::init_decompressor();
    let mut report = Report { game: crate::stock::Game::new(romfs), ..Default::default() };
    let read = |p: &Path| -> Option<ResourceData> {
        let data = fs::read(p).ok()?;
        let raw = {
            let mut d = decomp.lock();
            let size = uk_mod::zstd::bulk::Decompressor::upper_bound(&data).unwrap_or(data.len() * 1024);
            d.decompress(&data, size).ok().or_else(|| uk_mod::zstd::decode_all(data.as_slice()).ok())?
        };
        minicbor_ser::from_slice::<ResourceData>(&raw).ok()
    };
    let plain = |d: Vec<u8>| -> Result<Vec<u8>> {
        Ok(if d.starts_with(b"Yaz0") { roead::yaz0::decompress(&d).context("Broken Yaz0 data")? } else { d })
    };
    let write = |p: &Path, r: &ResourceData| -> Result<()> {
        let bytes = minicbor_ser::to_vec(r).map_err(|e| anyhow_ext::anyhow!("Failed to encode a converted file: {e:?}"))?;
        fs::write(p, uk_mod::zstd::encode_all(bytes.as_slice(), 8)?)?;
        Ok(())
    };
    let mut roots = vec![dir.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(dir.join("options")) {
        roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    const AOC: &str = "Aoc/0010/";
    let mut textures: BTreeMap<(usize, String), BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
    let mut places: BTreeMap<(usize, String), BTreeSet<String>> = BTreeMap::new();
    let mut removed: Vec<BTreeSet<String>> = vec![BTreeSet::new(); roots.len()];
    let mut neutral: BTreeMap<(usize, String), Vec<&'static str>> = BTreeMap::new();
    for (ri, root) in roots.iter().enumerate() {
        let mut files = Vec::new();
        walk(root, root, dir, &mut files);
        report.tex_stems = Some(
            files.iter().filter_map(|(n, _)| texture_part(n)).filter(|(_, part)| *part < MIPS).map(|(stem, _)| stem).collect(),
        );
        for (name, path) in files {
            let Some(res) = read(&path) else { continue };
            match res {
                ResourceData::Binary(data) => {
                    if let Some((stem, part)) = texture_part(&name) {
                        let (prefix, key) = match stem.strip_prefix(AOC) {
                            Some(k) => (AOC, k.to_string()),
                            None => ("", stem.clone()),
                        };
                        textures.entry((ri, key.clone())).or_default().insert(part, data);
                        places.entry((ri, key)).or_default().insert(format!("{prefix}|{name}"));
                        fs::remove_file(&path)?;
                        continue;
                    }
                    if name.starts_with("timg/") && name.ends_with(".bflim") {
                        fs::remove_file(&path)?;
                        removed[ri].insert(name);
                        continue;
                    }
                    match crate::wiiu::convert_with(&name, &data, &mut report) {
                        Ok(out) => write(&path, &ResourceData::Binary(plain(out)?))?,
                        Err(why) => {
                            report.leave_out(name.clone(), why);
                            fs::remove_file(&path)?;
                            removed[ri].insert(name);
                        }
                    }
                }
                ResourceData::Sarc(mut map) => {
                    if map.files.iter().any(|f| texture_part(f).is_some()) {
                        let entries: Vec<(String, bool)> =
                            map.files.iter_full().map(|(f, d)| (f.to_string(), *d)).collect();
                        map.files = rename_list(entries).into_iter().map(|(f, d)| (f.into(), d)).collect();
                        write(&path, &ResourceData::Sarc(map))?;
                    }
                }
                ResourceData::Mergeable(MergeableResource::LayoutArchive(mut a)) => {
                    let mut images = Vec::new();
                    let mut files = uk_content::util::IndexMap::default();
                    for (n, bytes) in std::mem::take(&mut a.0) {
                        if let Some(stem) = n.strip_prefix("timg/").and_then(|s| s.strip_suffix(".bflim")) {
                            images.push((stem.to_string(), bytes));
                            continue;
                        }
                        match crate::wiiu::convert_with(&n, &bytes, &mut report) {
                            Ok(out) => {
                                files.insert(n, out);
                            }
                            Err(why) => report.leave_out(format!("{name}//{n}"), why),
                        }
                    }
                    if !images.is_empty() {
                        match crate::wiiu::layout_textures(&name, &images, &mut report) {
                            Ok(bntx) => {
                                report.converted += 1;
                                report.textures += images.len();
                                files.insert("timg/__Combined.bntx".into(), bntx);
                            }
                            Err(e) => report.skipped.push((format!("{name}//timg"), format!("layout textures: {e:#}"))),
                        }
                    }
                    a.0 = files;
                    write(&path, &ResourceData::Mergeable(MergeableResource::LayoutArchive(a)))?;
                }
                ResourceData::Mergeable(MergeableResource::FontArchive(mut a)) => {
                    let mut files = uk_content::util::IndexMap::default();
                    for (n, bytes) in std::mem::take(&mut a.0) {
                        match crate::wiiu::convert_with(&n, &bytes, &mut report) {
                            Ok(out) => {
                                files.insert(n, out);
                            }
                            Err(why) => report.leave_out(format!("{name}//{n}"), why),
                        }
                    }
                    a.0 = files;
                    write(&path, &ResourceData::Mergeable(MergeableResource::FontArchive(a)))?;
                }
                ResourceData::Mergeable(MergeableResource::BinaryOverride(b)) => {
                    let (bytes, why) = *b;
                    match crate::wiiu::convert_with(&name, &bytes, &mut report) {
                        Ok(out) => write(
                            &path,
                            &ResourceData::Mergeable(MergeableResource::BinaryOverride(Box::new((plain(out)?, why)))),
                        )?,
                        Err(why) => {
                            report.leave_out(name.clone(), why);
                            fs::remove_file(&path)?;
                            removed[ri].insert(name);
                        }
                    }
                }
                ResourceData::Mergeable(MergeableResource::ActorInfo(mut a)) => {
                    match report.game.actor_sizes() {
                        Some(sizes) => {
                            for (hash, entry) in a.0.iter_mut() {
                                sizes.convert_entry(*hash, entry);
                            }
                            write(&path, &ResourceData::Mergeable(MergeableResource::ActorInfo(a)))?;
                        }
                        None => report.notes.push(
                            "actor sizes left as the Wii U's: the Switch game's files weren't found (actors the mod                              adds may crash the game)"
                                .into(),
                        ),
                    }
                }
                ResourceData::Mergeable(_) => {}
            }
        }
        for (stem, names) in std::mem::take(&mut report.neutral) {
            let key = stem.strip_prefix(AOC).unwrap_or(&stem).to_string();
            let list = neutral.entry((ri, key)).or_default();
            list.extend(names);
            list.sort_unstable();
            list.dedup();
        }
    }
    report.tex_stems = None;
    let keys: Vec<(usize, String)> = textures.keys().cloned().collect();
    for (ri, key) in keys {
        let mut parts = textures[&(ri, key.clone())].clone();
        if ri != 0 && !parts.keys().any(|k| *k < MIPS)
            && let Some(main) = textures.get(&(0, key.clone())) {
                parts.extend(main.iter().filter(|(k, _)| **k < MIPS).map(|(k, v)| (*k, v.clone())));
            }
        let file = key.rsplit('/').next().unwrap_or(&key).to_string();
        let names: Vec<(String, String)> = places[&(ri, key.clone())]
            .iter()
            .filter_map(|p| p.split_once('|').map(|(a, b)| (a.to_string(), b.to_string())))
            .collect();
        let tex2: Vec<&[u8]> = parts.iter().filter(|(k, _)| **k >= MIPS).map(|(_, v)| v.as_slice()).collect();
        let firsts: Vec<&[u8]> = parts.iter().filter(|(k, _)| **k < MIPS).map(|(_, v)| v.as_slice()).collect();
        let unchanged = firsts.is_empty()
            && !tex2.is_empty()
            && tex2.iter().all(|d| {
                let d = if d.starts_with(b"Yaz0") { roead::yaz0::decompress(d).map(|d| d.to_vec()).unwrap_or_default() } else { d.to_vec() };
                crate::stock::wiiu_only(&d)
            });
        let result = if firsts.is_empty() {
            Err(anyhow_ext::anyhow!("mip levels without their texture file (Tex1)"))
        } else {
            crate::tex::convert_wiiu_split(&file, &firsts, &tex2)
        };
        match result {
            Ok((data, r)) => {
                let data = plain(data)?;
                let data = match neutral.get(&(ri, key.clone())) {
                    Some(names) => crate::tex::add_textures_to_tex(&data, &format!("{file}.Tex"), names)?,
                    None => data,
                };
                let prefixes: BTreeSet<&str> = names.iter().map(|(p, _)| p.as_str()).collect();
                for prefix in prefixes {
                    let out = roots[ri].join(format!("{prefix}{key}.Tex.bfres"));
                    if let Some(p) = out.parent() {
                        fs::create_dir_all(p)?;
                    }
                    write(&out, &ResourceData::Binary(data.clone()))?;
                }
                report.converted += 1;
                report.textures += r.textures;
                report.notes.extend(r.notes.into_iter().map(|n| format!("{file}: {n}")));
            }
            Err(e) => {
                for (_, name) in &names {
                    if unchanged {
                        report.leave_out(name.clone(), crate::wiiu::NOT_NEEDED);
                    } else {
                        report.skipped.push((name.clone(), format!("texture: {e:#}")));
                    }
                    removed[ri].insert(name.clone());
                }
            }
        }
    }
    for (ri, root) in roots.iter().enumerate() {
        let removed = &removed[ri];
        let manifest_path = root.join("manifest.yml");
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            let mut m: Manifest = serde_yaml::from_str(&text).context("Broken manifest in the mod")?;
            let fix = |set: &BTreeSet<smartstring::alias::String>| -> BTreeSet<smartstring::alias::String> {
                let kept = set
                    .iter()
                    .filter(|f| !removed.contains(f.as_str()) && !removed.contains(uk_content::canonicalize(f.as_str()).as_str()))
                    .map(|f| (f.to_string(), false))
                    .collect();
                rename_list(kept).into_iter().map(|(f, _)| f.into()).collect()
            };
            m.content_files = fix(&m.content_files);
            m.aoc_files = fix(&m.aoc_files);
            fs::write(&manifest_path, serde_yaml::to_string(&m)?)?;
        }
    }
    let meta_path = dir.join("meta.yml");
    let mut meta: Meta = serde_yaml::from_str(&fs::read_to_string(&meta_path)?).context("Broken meta.yml in the mod")?;
    meta.platform = ModPlatform::Specific(Endian::Little);
    fs::write(&meta_path, serde_yaml::to_string(&meta)?)?;
    Ok(report)
}

fn walk(d: &Path, root: &Path, package: &Path, out: &mut Vec<(String, PathBuf)>) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            if root == package && p == package.join("options") {
                continue;
            }
            walk(&p, root, package, out);
        } else {
            let name = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push((name, p));
        }
    }
}

pub fn fix_lists(dir: &Path) -> Result<usize> {
    let decomp = uk_mod::unpack::init_decompressor();
    let mut changed = 0;
    let mut roots = vec![dir.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(dir.join("options")) {
        roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    for root in &roots {
        let mut files = Vec::new();
        walk(root, root, dir, &mut files);
        for (_, path) in files {
            let Ok(data) = fs::read(&path) else { continue };
            let raw = {
                let mut d = decomp.lock();
                let size = uk_mod::zstd::bulk::Decompressor::upper_bound(&data).unwrap_or(data.len() * 1024);
                match d.decompress(&data, size).ok().or_else(|| uk_mod::zstd::decode_all(data.as_slice()).ok()) {
                    Some(r) => r,
                    None => continue,
                }
            };
            let Ok(ResourceData::Sarc(mut map)) = minicbor_ser::from_slice::<ResourceData>(&raw) else { continue };
            if !map.files.iter().any(|f| texture_part(f).is_some()) {
                continue;
            }
            let entries: Vec<(String, bool)> = map.files.iter_full().map(|(f, d)| (f.to_string(), *d)).collect();
            map.files = rename_list(entries).into_iter().map(|(f, d)| (f.into(), d)).collect();
            let bytes = minicbor_ser::to_vec(&ResourceData::Sarc(map))
                .map_err(|e| anyhow_ext::anyhow!("Failed to encode a file list: {e:?}"))?;
            fs::write(&path, uk_mod::zstd::encode_all(bytes.as_slice(), 8)?)?;
            changed += 1;
        }
        let manifest_path = root.join("manifest.yml");
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            let mut m: Manifest = serde_yaml::from_str(&text).context("Broken manifest in the mod")?;
            if m.content_files.iter().chain(m.aoc_files.iter()).any(|f| texture_part(f).is_some()) {
                let fix = |set: &BTreeSet<smartstring::alias::String>| -> BTreeSet<smartstring::alias::String> {
                    rename_list(set.iter().map(|f| (f.to_string(), false)).collect()).into_iter().map(|(f, _)| f.into()).collect()
                };
                m.content_files = fix(&m.content_files);
                m.aoc_files = fix(&m.aoc_files);
                fs::write(&manifest_path, serde_yaml::to_string(&m)?)?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}
