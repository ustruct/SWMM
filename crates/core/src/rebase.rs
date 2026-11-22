
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow_ext::{Context, Result};
use botw_utils::hashes::{Platform, StockHashTable};
use fs_err as fs;
use roead::{
    byml::Byml,
    sarc::{Sarc, SarcWriter},
    yaz0::{compress, decompress_if},
};

static STOCK: LazyLock<StockHashTable> = LazyLock::new(|| StockHashTable::new(&Platform::Switch));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub actors_kept: usize,
    pub actor_sizes: usize,
    pub flags_kept: usize,
    pub flags_moved: usize,
    pub saved_flags_kept: usize,
    pub texts_kept: usize,
}

impl Report {
    pub fn changed(&self) -> bool {
        *self != Self::default()
    }
}

fn newer_than_stock(name: &str, data: &[u8]) -> bool {
    STOCK.is_file_modded(name, decompress_if(data), true)
}

fn endian_of(byml: &[u8]) -> roead::Endian {
    if byml.starts_with(b"BY") {
        roead::Endian::Big
    } else {
        roead::Endian::Little
    }
}

fn repacked(original: &[u8], data: Vec<u8>) -> Vec<u8> {
    if original.starts_with(b"Yaz0") {
        compress(data)
    } else {
        data
    }
}

fn actor_name(actor: &Byml) -> Option<&str> {
    actor.as_map().ok()?.get("name")?.as_string().ok().map(|s| s.as_str())
}

fn actor_size(actor: &Byml) -> Option<i64> {
    actor.as_map().ok()?.get("instSize")?.as_int::<i64>().ok()
}

pub fn actor_info(theirs: &[u8], game: &[u8], report: &mut Report) -> Result<Option<Vec<u8>>> {
    let (mod_data, game_data) = (decompress_if(theirs), decompress_if(game));
    let mut table = Byml::from_binary(&mod_data).context("The mod's actor table can't be read")?;
    let game_table = Byml::from_binary(&game_data).context("The game's actor table can't be read")?;
    let game_actors = game_table.as_map()?.get("Actors").context("No actors")?.as_array()?;
    let game_by_name: BTreeMap<&str, &Byml> =
        game_actors.iter().filter_map(|actor| Some((actor_name(actor)?, actor))).collect();
    let actors = table.as_mut_map()?.get_mut("Actors").context("No actors")?.as_mut_array()?;
    let (mut kept, mut sizes) = (0, 0);
    let mut have = HashSet::new();
    for actor in actors.iter_mut() {
        let Some(name) = actor_name(actor).map(String::from) else { continue };
        if let Some(game_actor) = game_by_name.get(name.as_str()) {
            if let (Some(mine), Some(theirs)) = (actor_size(actor), actor_size(game_actor)) {
                if mine < theirs {
                    let size = game_actor.as_map()?.get("instSize").cloned().context("No size")?;
                    actor.as_mut_map()?.insert("instSize".into(), size);
                    sizes += 1;
                }
            }
        }
        have.insert(name);
    }
    for (name, actor) in &game_by_name {
        if !have.contains(*name) {
            actors.push((*actor).clone());
            kept += 1;
        }
    }
    if kept == 0 && sizes == 0 {
        return Ok(None);
    }
    let hash = |actor: &Byml| actor_name(actor).map(roead::aamp::hash_name).unwrap_or(0);
    actors.sort_by_key(hash);
    let hashes: Vec<Byml> = actors
        .iter()
        .map(|actor| match hash(actor) {
            h if h > i32::MAX as u32 => Byml::U32(h),
            h => Byml::I32(h as i32),
        })
        .collect();
    table.as_mut_map()?.insert("Hashes".into(), Byml::Array(hashes));
    report.actors_kept += kept;
    report.actor_sizes += sizes;
    Ok(Some(repacked(theirs, table.to_binary(endian_of(&mod_data)))))
}

fn flag_name(flag: &Byml) -> Option<&str> {
    flag.as_map().ok()?.get("DataName")?.as_string().ok().map(|s| s.as_str())
}

fn flag_list(file: &Byml) -> Option<(&str, &[Byml])> {
    let (key, flags) = file.as_map().ok()?.iter().next()?;
    Some((key.as_str(), flags.as_array().ok()?))
}

fn flag_kind(file: &str) -> &str {
    let stem = file.trim_start_matches('/').trim_end_matches(".bgdata");
    stem.rsplit_once('_').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).map_or(stem, |(kind, _)| kind)
}

pub fn game_data(theirs: &[u8], game: &[u8], report: &mut Report) -> Result<Option<Vec<u8>>> {
    let (mod_data, game_data) = (decompress_if(theirs), decompress_if(game));
    let (mod_sarc, game_sarc) = (Sarc::new(&*mod_data)?, Sarc::new(&*game_data)?);
    let mut game_kind: BTreeMap<String, String> = BTreeMap::new();
    let mut game_flags: Vec<(String, Byml)> = Vec::new();
    let mut kinds: BTreeMap<String, (String, String)> = BTreeMap::new();
    for file in game_sarc.files() {
        let Some(name) = file.name() else { continue };
        let parsed = Byml::from_binary(file.data()).with_context(|| format!("The game's {name} can't be read"))?;
        let Some((key, flags)) = flag_list(&parsed) else { continue };
        let kind = flag_kind(name).to_string();
        kinds.entry(kind.clone()).or_insert_with(|| (name.to_string(), key.to_string()));
        for flag in flags {
            if let Some(flag_name) = flag_name(flag) {
                game_kind.insert(flag_name.to_string(), kind.clone());
                game_flags.push((kind.clone(), flag.clone()));
            }
        }
    }
    let mut files: BTreeMap<String, (String, Vec<Byml>)> = BTreeMap::new();
    let mut have: HashSet<String> = HashSet::new();
    let mut to_add: BTreeMap<String, Vec<Byml>> = BTreeMap::new();
    let mut changed: HashSet<String> = HashSet::new();
    let (mut kept, mut moved) = (0, 0);
    for file in mod_sarc.files() {
        let Some(name) = file.name() else { continue };
        let parsed = Byml::from_binary(file.data()).with_context(|| format!("The mod's {name} can't be read"))?;
        let Some((key, flags)) = flag_list(&parsed) else { continue };
        let kind = flag_kind(name);
        let mut stay = Vec::with_capacity(flags.len());
        for flag in flags {
            let flag_name = flag_name(flag).unwrap_or_default();
            have.insert(flag_name.to_string());
            match game_kind.get(flag_name) {
                Some(theirs) if theirs != kind => {
                    to_add.entry(theirs.clone()).or_default().push(flag.clone());
                    changed.insert(name.to_string());
                    moved += 1;
                }
                _ => stay.push(flag.clone()),
            }
        }
        files.insert(name.to_string(), (key.to_string(), stay));
    }
    for (kind, flag) in game_flags {
        if !flag_name(&flag).is_some_and(|n| have.contains(n)) {
            to_add.entry(kind).or_default().push(flag);
            kept += 1;
        }
    }
    if kept == 0 && moved == 0 {
        return Ok(None);
    }
    for (kind, flags) in to_add {
        let target = files.keys().rev().find(|name| flag_kind(name) == kind).cloned();
        let target = match target {
            Some(target) => target,
            None => {
                let (_, key) = kinds.get(&kind).cloned().context("A kind of flag without a file")?;
                let name = format!("/{kind}_0.bgdata");
                files.insert(name.clone(), (key, Vec::new()));
                name
            }
        };
        files.get_mut(&target).context("A flag file is gone")?.1.extend(flags);
        changed.insert(target);
    }
    let mut writer = SarcWriter::from_sarc(&mod_sarc);
    let endian = mod_sarc.endian();
    for name in changed {
        let (key, flags) = &files[&name];
        let mut map = roead::byml::Map::default();
        map.insert(key.as_str().into(), Byml::Array(flags.clone()));
        writer.files.insert(name.as_str().into(), Byml::Map(map).to_binary(endian));
    }
    report.flags_kept += kept;
    report.flags_moved += moved;
    Ok(Some(repacked(theirs, writer.to_binary())))
}

fn save_file(file: &Byml) -> Option<(&str, &[Byml])> {
    let list = file.as_map().ok()?.get("file_list")?.as_array().ok()?;
    let name = list.first()?.as_map().ok()?.get("file_name")?.as_string().ok()?;
    Some((name.as_str(), list.get(1)?.as_array().ok()?))
}

pub fn save_data(theirs: &[u8], game: &[u8], report: &mut Report) -> Result<Option<Vec<u8>>> {
    let (mod_data, game_data) = (decompress_if(theirs), decompress_if(game));
    let (mod_sarc, game_sarc) = (Sarc::new(&*mod_data)?, Sarc::new(&*game_data)?);
    let mut files: BTreeMap<String, Byml> = BTreeMap::new();
    let mut have: HashSet<(String, String)> = HashSet::new();
    for file in mod_sarc.files() {
        let Some(name) = file.name() else { continue };
        let parsed = Byml::from_binary(file.data()).with_context(|| format!("The mod's {name} can't be read"))?;
        if let Some((save, flags)) = save_file(&parsed) {
            have.extend(flags.iter().filter_map(flag_name).map(|flag| (save.to_string(), flag.to_string())));
        }
        files.insert(name.to_string(), parsed);
    }
    let mut kept = 0;
    let mut changed: HashSet<String> = HashSet::new();
    for file in game_sarc.files() {
        let Some(name) = file.name() else { continue };
        let parsed = Byml::from_binary(file.data()).with_context(|| format!("The game's {name} can't be read"))?;
        let Some((save, flags)) = save_file(&parsed) else { continue };
        let missing: Vec<Byml> = flags
            .iter()
            .filter(|flag| flag_name(flag).is_some_and(|n| !have.contains(&(save.to_string(), n.to_string()))))
            .cloned()
            .collect();
        if missing.is_empty() {
            continue;
        }
        let target = files
            .iter()
            .find(|(_, parsed)| save_file(parsed).is_some_and(|(s, _)| s == save))
            .map(|(name, _)| name.clone());
        match target {
            Some(target) => {
                let list = files
                    .get_mut(&target)
                    .and_then(|f| f.as_mut_map().ok()?.get_mut("file_list")?.as_mut_array().ok()?.get_mut(1));
                list.context("A save format file lost its flags")?.as_mut_array()?.extend(missing.iter().cloned());
                changed.insert(target);
            }
            None => {
                files.insert(name.to_string(), parsed.clone());
                changed.insert(name.to_string());
            }
        }
        kept += missing.len();
    }
    if kept == 0 {
        return Ok(None);
    }
    let mut writer = SarcWriter::from_sarc(&mod_sarc);
    let endian = mod_sarc.endian();
    for name in changed {
        writer.files.insert(name.as_str().into(), files[&name].to_binary(endian));
    }
    report.saved_flags_kept += kept;
    Ok(Some(repacked(theirs, writer.to_binary())))
}

const GAME_DATA: &str = "GameData/gamedata.ssarc";
const SAVE_DATA: &str = "GameData/savedataformat.ssarc";

fn bootup(theirs: &[u8], game: &[u8], report: &mut Report) -> Result<Option<Vec<u8>>> {
    let (mod_bytes, game_bytes) = (decompress_if(theirs), decompress_if(game));
    let (mod_sarc, game_sarc) = (Sarc::new(&*mod_bytes)?, Sarc::new(&*game_bytes)?);
    let mut writer = SarcWriter::from_sarc(&mod_sarc);
    let mut changed = false;
    type Step = fn(&[u8], &[u8], &mut Report) -> Result<Option<Vec<u8>>>;
    let steps: [(&str, Step); 2] = [(GAME_DATA, game_data), (SAVE_DATA, save_data)];
    for (member, step) in steps {
        let (Some(mine), Some(theirs)) = (mod_sarc.get_data(member), game_sarc.get_data(member)) else { continue };
        if !newer_than_stock(member, theirs) {
            continue;
        }
        if let Some(new) = step(mine, theirs, report).with_context(|| format!("Failed on {member}"))? {
            writer.files.insert(member.into(), new);
            changed = true;
        }
    }
    Ok(changed.then(|| repacked(theirs, writer.to_binary())))
}

fn roots(staged: &Path) -> Vec<(PathBuf, bool)> {
    let mut mods = vec![staged.to_path_buf()];
    if let Ok(options) = std::fs::read_dir(staged.join("options")) {
        mods.extend(options.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let mut roots = Vec::new();
    for root in mods {
        for (folder, dlc) in [
            ("01007EF00011E000/romfs", false),
            ("content", false),
            ("01007EF00011F001/romfs", true),
            ("aoc/0010", true),
        ] {
            let dir = root.join(folder);
            if dir.is_dir() {
                roots.push((dir, dlc));
            }
        }
    }
    roots
}

pub fn rebase_tree(staged: &Path, game: &[PathBuf]) -> Result<Report> {
    let mut report = Report::default();
    type Step = fn(&[u8], &[u8], &mut Report) -> Result<Option<Vec<u8>>>;
    let files: [(&str, &str, Step); 2] = [
        ("Actor/ActorInfo.product.sbyml", "Actor/ActorInfo.product.byml", actor_info),
        ("Pack/Bootup.pack", "Pack/Bootup.pack", bootup),
    ];
    for (root, dlc) in roots(staged) {
        if dlc {
            continue;
        }
        for (file, name, step) in files {
            let path = root.join(file);
            let Some(theirs) = game.first().and_then(|dir| std::fs::read(dir.join(file)).ok()) else { continue };
            if !path.is_file() || !newer_than_stock(name, &theirs) {
                continue;
            }
            let mine = fs::read(&path)?;
            if let Some(new) = step(&mine, &theirs, &mut report).with_context(|| format!("Failed on {file}"))? {
                fs::write(&path, new)?;
            }
        }
        let packs = std::fs::read_dir(root.join("Pack")).into_iter().flatten().filter_map(|e| e.ok());
        for pack in packs {
            let file_name = pack.file_name().to_string_lossy().to_string();
            let Some(language) = crate::texts::language(&file_name) else { continue };
            let Some(theirs) = game.first().and_then(|dir| std::fs::read(dir.join("Pack").join(&file_name)).ok())
            else {
                continue;
            };
            let mine = fs::read(pack.path())?;
            let new = crate::texts::language_pack(&mine, &theirs, language)
                .with_context(|| format!("Failed on {file_name}"))?;
            if let Some((new, count)) = new {
                fs::write(pack.path(), new)?;
                report.texts_kept += count;
            }
        }
    }
    if report.changed() {
        crate::origin::note_rebased();
        log::info!(
            "The mod was made for another version of the game. Kept from this one: {} actors, {} actor sizes, {} \
             flags ({} more under the game's kind), {} saved flags, {} texts",
            report.actors_kept,
            report.actor_sizes,
            report.flags_kept,
            report.flags_moved,
            report.saved_flags_kept,
            report.texts_kept
        );
    }
    Ok(report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roead::byml::Map;

    fn map(pairs: &[(&str, Byml)]) -> Byml {
        let mut map = Map::default();
        for (key, value) in pairs {
            map.insert((*key).into(), value.clone());
        }
        Byml::Map(map)
    }

    fn actors(list: &[(&str, i32)]) -> Vec<u8> {
        let actors =
            list.iter().map(|(name, size)| map(&[("name", Byml::String((*name).into())), ("instSize", Byml::I32(*size))]));
        map(&[("Actors", Byml::Array(actors.collect())), ("Hashes", Byml::Array(vec![]))]).to_binary(roead::Endian::Little)
    }

    #[test]
    fn the_games_actors_and_sizes_are_kept() {
        let game = actors(&[("Old", 120), ("Grown", 300), ("New", 50)]);
        let theirs = compress(actors(&[("Old", 120), ("Grown", 200), ("Mod", 10), ("Bigger", 999)]));
        let game_with = actors(&[("Old", 120), ("Grown", 300), ("New", 50), ("Bigger", 400)]);
        let mut report = Report::default();
        let out = actor_info(&theirs, &game_with, &mut report).unwrap().unwrap();
        assert!(out.starts_with(b"Yaz0"));
        assert_eq!((report.actors_kept, report.actor_sizes), (1, 1));
        let table = Byml::from_binary(decompress_if(&out)).unwrap();
        let list = table.as_map().unwrap()["Actors"].as_array().unwrap();
        let size = |name: &str| list.iter().find(|a| actor_name(a) == Some(name)).and_then(actor_size);
        assert_eq!(
            [size("Old"), size("Grown"), size("New"), size("Mod"), size("Bigger")],
            [Some(120), Some(300), Some(50), Some(10), Some(999)]
        );
        let hashes = table.as_map().unwrap()["Hashes"].as_array().unwrap();
        assert_eq!(hashes.len(), 5);
        let order: Vec<u32> = list.iter().map(|a| roead::aamp::hash_name(actor_name(a).unwrap())).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
        assert!(actor_info(&out, &game_with, &mut report).unwrap().is_none());
        assert!(actor_info(&game, &game, &mut report).unwrap().is_none());
    }

    fn flag(name: &str) -> Byml {
        map(&[("DataName", Byml::String(name.into())), ("HashValue", Byml::I32(1))])
    }

    fn flags(files: &[(&str, &str, &[&str])]) -> Vec<u8> {
        let mut writer = SarcWriter::new(roead::Endian::Little);
        for (file, kind, names) in files {
            let list = Byml::Array(names.iter().map(|n| flag(n)).collect());
            writer.files.insert((*file).into(), map(&[(kind, list)]).to_binary(roead::Endian::Little));
        }
        writer.to_binary()
    }

    #[test]
    fn the_games_flags_are_kept() {
        let game = flags(&[
            ("/bool_data_0.bgdata", "bool_data", &["A", "B", "NewBool"]),
            ("/s32_data_0.bgdata", "s32_data", &["N", "NewNumber"]),
            ("/string256_data_0.bgdata", "string256_data", &["NewText"]),
            ("/revival_bool_data_3.bgdata", "bool_data", &["Back"]),
        ]);
        let theirs = flags(&[
            ("/bool_data_0.bgdata", "bool_data", &["A"]),
            ("/bool_data_1.bgdata", "bool_data", &["B", "Mod", "NewNumber", "Back"]),
            ("/s32_data_0.bgdata", "s32_data", &["N"]),
        ]);
        let mut report = Report::default();
        let out = game_data(&theirs, &game, &mut report).unwrap().unwrap();
        assert_eq!((report.flags_kept, report.flags_moved), (2, 2));
        let sarc = Sarc::new(&*out).unwrap();
        let names = |file: &str| -> Vec<String> {
            let parsed = Byml::from_binary(sarc.get_data(file).unwrap()).unwrap();
            flag_list(&parsed).unwrap().1.iter().map(|f| flag_name(f).unwrap().to_string()).collect()
        };
        assert_eq!(names("/bool_data_0.bgdata"), ["A"]);
        assert_eq!(names("/bool_data_1.bgdata"), ["B", "Mod", "NewBool"]);
        assert_eq!(names("/s32_data_0.bgdata"), ["N", "NewNumber"]);
        assert_eq!(names("/revival_bool_data_0.bgdata"), ["Back"]);
        assert_eq!(flag_kind("/revival_bool_data_12.bgdata"), "revival_bool_data");
        assert_eq!(names("/string256_data_0.bgdata"), ["NewText"]);
        assert!(game_data(&out, &game, &mut report).unwrap().is_none());
    }
}
