use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow_ext::{Context, Result, bail};
use fs_err as fs;
use swmm_formats::{platform_prefixes, prelude::Endian};
use swmm_modpack::{
    pack::ModPacker, unpack::ModReader, ExclusiveOptionGroup, Meta, ModCategory, ModOption,
    ModPlatform, MultipleOptionGroup, OptionGroup,
};

use crate::util;

const MAX_DEPTH: usize = 4;
const ARCHIVES: &[&str] = ARCHIVE_EXTENSIONS;
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lzma", "cab", "iso",
];
pub const MOD_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lzma", "cab", "iso",
    "bnp", "txt", "json", "pack", "sarc", "ssarc", "bactorpack", "sbactorpack", "bfres", "sbfres",
    "blarc", "sblarc", "byml", "sbyml", "bgdata", "bgsvdata", "bxml", "sbxml", "baslist", "bmodellist",
    "bphysics", "bgparamlist", "bdrop", "bshop", "brecipe", "bitemico", "sbitemico", "bntx",
    "bfstm", "bfwav", "bars", "bfsar", "msbt", "smubin", "mubin", "hkrb", "hkcl", "hksc", "hkrg",
    "bfevfl", "sbfevfl", "bfsha", "sbfsha", "bfarc", "sbfarc", "bflim", "bflyt", "bflan", "evfl",
    "bcamanim", "beventpack", "sbeventpack", "bstftex", "sstats", "stats", "bmscdef", "bmscinfo",
    "sbquestpack", "bquestpack", "sbmapopen", "sbreviewtex", "bfotf", "bfttf", "bffnt", "mp4",
];
const NOT_ROMFS_EXTS: &[&str] = &["nsp", "xci", "nca", "nsz", "xcz", "ips", "pchtxt", "tkcl", "nro"];
const SKIP_EXTS: &[&str] = &[
    "txt", "md", "pdf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "url", "html", "htm", "ini",
    "json", "yml", "yaml", "exe", "dll", "db",
];

pub fn convert_any(core: &crate::core::Manager, path: &Path, meta: Option<Meta>) -> Result<PathBuf> {
    log::info!("Looking for a mod in {}", path.display());
    convert_inner(core, path, meta, 0)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default()
}

fn is_ukmm_dir(dir: &Path) -> bool {
    dir.join("meta.yml").is_file() && dir.join("manifest.yml").is_file()
}

fn is_bnp_dir(dir: &Path) -> bool {
    dir.join("info.json").is_file()
        && ["content", "aoc", "01007EF00011E000", "01007EF00011F001", "logs"]
            .iter()
            .any(|root| dir.join(root).exists())
}

fn convert_inner(
    core: &crate::core::Manager,
    path: &Path,
    meta: Option<Meta>,
    depth: usize,
) -> Result<PathBuf> {
    if depth > MAX_DEPTH {
        bail!("{} has too many archives inside archives", path.display());
    }
    let ext = extension(path);
    let file_name = name_of(path).to_lowercase();
    if path.is_file() && (file_name == "rules.txt" || file_name == "info.json") {
        if let Some(parent) = path.parent() {
            return convert_inner(core, parent, meta, depth);
        }
    }
    if path.is_file() {
        if ext == "bnp" {
            return crate::bnp::convert_bnp(core, path);
        }
        if ext == "zip" && ModReader::open_peek(path, vec![]).is_ok() {
            return Ok(path.to_path_buf());
        }
    } else if is_ukmm_dir(path) {
        return zip_folder(path);
    }

    let dir = if path.is_dir() {
        path.to_path_buf()
    } else if ARCHIVES.contains(&ext.as_str()) {
        extract_archive(path, &ext)?
    } else if path.is_file() {
        let temp = util::get_temp_folder().to_path_buf();
        let name = path.file_name().context("File has no name")?;
        fs::copy(path, temp.join(name))?;
        temp
    } else {
        bail!("{} does not exist", path.display());
    };

    let dirs = dirs_breadth_first(&dir);
    if let Some(found) = dirs.iter().find(|d| is_ukmm_dir(d)) {
        log::info!("Found a packaged mod at {}", found.display());
        return zip_folder(found);
    }
    if let Some(found) = dirs.iter().find(|d| is_bnp_dir(d)) {
        log::info!("Found a BNP folder at {}", found.display());
        return crate::bnp::convert_bnp(core, found);
    }

    let files = files_in(&dir);
    if let Some(bnp) = files.iter().find(|f| extension(f) == "bnp") {
        log::info!("Found a BNP at {}", bnp.display());
        return crate::bnp::convert_bnp(core, bnp);
    }
    let mut nested_error = None;
    for archive in files.iter().filter(|f| ARCHIVES.contains(&extension(f).as_str())) {
        log::info!("Trying archive {}", archive.display());
        match convert_inner(core, archive, meta.clone(), depth + 1) {
            Ok(result) => return Ok(result),
            Err(e) => {
                log::warn!("No mod in {}: {e:?}", archive.display());
                nested_error = Some(e);
            }
        }
    }

    let staged = util::get_temp_folder().to_path_buf();
    let result = stage_romfs(core, &dir, &dirs, &files, &staged)?;
    let placed = result.count;
    if placed == 0 {
        if let Some(reason) = unsupported_reason(&dirs, &files) {
            bail!(
                "{} can't be used: {reason}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
            );
        }
        if let Some(e) = nested_error {
            return Err(e);
        }
        bail!(
            "No game files were found in {}. A mod needs files laid out like the game's romfs \
             (for example Actor/Pack/...), a romfs folder, a BNP or a packaged mod.",
            path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
        );
    }
    log::info!("Staged {placed} game files from {}", dir.display());

    let meta = match meta {
        Some(meta) => meta,
        None => find_meta(&dirs).unwrap_or_else(|| auto_meta(path, &dir)),
    };
    let options = if meta.options.is_empty() { result.groups } else { meta.options.clone() };
    let meta = Meta {
        platform: ModPlatform::Specific(Endian::Little),
        options,
        ..meta
    };
    let dump = core
        .settings()
        .dump()
        .context("No game dump available. Choose savagewind.exe in Settings first.")?;
    let out = util::get_temp_folder().to_path_buf();
    let name = meta.name.clone();
    ModPacker::new(&staged, &out, Some(meta), vec![dump])
        .with_context(|| format!("Failed to package {name}"))?
        .pack()
}

fn find_meta(dirs: &[PathBuf]) -> Option<Meta> {
    dirs.iter().find_map(|d| {
        d.join("rules.txt")
            .is_file()
            .then(|| ModPacker::parse_rules(d.join("rules.txt")).ok())
            .flatten()
            .or_else(|| {
                d.join("info.json")
                    .is_file()
                    .then(|| ModPacker::parse_info(d.join("info.json")).ok())
                    .flatten()
            })
    })
}

fn auto_meta(path: &Path, dir: &Path) -> Meta {
    let single_folder = fs::read_dir(dir)
        .ok()
        .map(|entries| entries.filter_map(|e| e.ok()).collect::<Vec<_>>())
        .filter(|entries| entries.len() == 1 && entries[0].path().is_dir())
        .and_then(|entries| entries[0].file_name().to_str().map(|s| s.to_string()))
        .filter(|name| {
            let lower = name.to_lowercase();
            lower != "romfs" && !lower.starts_with("01007ef00011") && lower != "atmosphere"
        });
    let raw = single_folder.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported mod")
            .to_string()
    });
    let name = raw
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Meta {
        api: env!("CARGO_PKG_VERSION").into(),
        name: if name.is_empty() { "Imported mod".into() } else { name.as_str().into() },
        version: "1.0.0".into(),
        author: Default::default(),
        category: ModCategory::Other,
        description: Default::default(),
        platform: ModPlatform::Specific(Endian::Little),
        url: None,
        options: vec![],
        masters: Default::default(),
    }
}

fn dirs_breadth_first(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        out.push(dir.clone());
        if depth >= 8 {
            continue;
        }
        let mut children: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(_) => continue,
        };
        children.sort();
        queue.extend(children.into_iter().map(|c| (c, depth + 1)));
    }
    out
}

fn files_in(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = jwalk::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path())
        .collect();
    files.sort_by_key(|f| (f.components().count(), f.clone()));
    files
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

fn top_names(dir: &Path) -> HashMap<String, String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(|n| (n.to_lowercase(), n.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn copy_tree(src: &Path, dst: &Path) -> Result<usize> {
    let mut count = 0;
    for file in files_in(src) {
        let rel = file.strip_prefix(src)?;
        let to = dst.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&file, &to)?;
        count += 1;
    }
    Ok(count)
}

fn copy_romfs(src: &Path, dst: &Path, stock: &HashMap<String, String>) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(src)?.filter_map(|e| e.ok()) {
        let from = entry.path();
        let name = name_of(&from);
        let fixed = stock.get(&name.to_lowercase()).cloned().unwrap_or(name);
        if from.is_dir() {
            count += copy_tree(&from, &dst.join(fixed))?;
        } else if !SKIP_EXTS.contains(&extension(&from).as_str()) {
            fs::create_dir_all(dst)?;
            fs::copy(&from, dst.join(fixed))?;
            count += 1;
        }
    }
    Ok(count)
}

struct Staged {
    count: usize,
    groups: Vec<OptionGroup>,
}

enum Source {
    Romfs { path: PathBuf, dlc: bool },
    Loose(PathBuf),
}

const LAYOUT_NAMES: &[&str] = &["atmosphere", "contents", "load", "mods", "sdmc", "switch", "romfs"];
const BASE_WORDS: &[&str] = &["base", "main", "core", "required", "common", "shared", "always"];

fn stage_romfs(
    core: &crate::core::Manager,
    root: &Path,
    dirs: &[PathBuf],
    files: &[PathBuf],
    staged: &Path,
) -> Result<Staged> {
    let (content, aoc) = platform_prefixes(core.settings().current_mode.into());
    let game = {
        let settings = core.settings();
        crate::savage_wind::current_game_files(&settings).ok()
    };
    let result = stage_layout(content, aoc, game.as_ref(), root, dirs, files, staged)?;
    if let Some(game) = game.as_ref() {
        let mut roots = vec![staged.to_path_buf()];
        if let Ok(entries) = fs::read_dir(staged.join("options")) {
            roots.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
        }
        for opt_root in roots {
            route_dlc_files(game, &opt_root.join(content), &opt_root.join(aoc))?;
        }
    }
    Ok(result)
}

fn route_dlc_files(game: &crate::savage_wind::GameFiles, content_dst: &Path, aoc_dst: &Path) -> Result<()> {
    if !content_dst.is_dir() {
        return Ok(());
    }
    for file in files_in(content_dst) {
        let rel = file.strip_prefix(content_dst)?.to_path_buf();
        let in_base = game.romfs.join(&rel).is_file();
        let in_dlc = game.all_aoc.iter().any(|a| a.join(&rel).is_file());
        if in_dlc && !in_base {
            let to = aoc_dst.join(&rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            log::info!("{} belongs to the DLC", rel.display());
            fs::rename(&file, &to)?;
        }
    }
    Ok(())
}

fn stock_top_names(game: Option<&crate::savage_wind::GameFiles>) -> HashMap<String, String> {
    let mut stock_tops = game.map(|g| top_names(&g.romfs)).unwrap_or_default();
    if let Some(game) = game {
        for aoc_dir in &game.all_aoc {
            for (k, v) in top_names(aoc_dir) {
                stock_tops.entry(k).or_insert(v);
            }
        }
    }
    for name in [
        "Actor", "Pack", "Model", "Layout", "Map", "Physics", "UI", "Sound", "Effect", "Event",
        "EventFlow", "Font", "Movie", "System", "Terrain", "Shader", "Voice", "Message", "Cooking",
        "GameData", "Quest", "Ecosystem", "NavMesh", "Camera", "Tips", "Awareness", "Demo",
        "DamageParam", "ELink2", "SLink2", "Env", "Local", "Particle", "Chemical", "AudioRes",
    ] {
        stock_tops.entry(name.to_lowercase()).or_insert_with(|| name.to_string());
    }
    stock_tops.remove("romfs");
    stock_tops.remove("aoc");
    stock_tops
}

fn variant_key(root: &Path, dir: &Path, skip: &[&str]) -> (String, bool, Vec<String>) {
    let rel: Vec<String> = dir
        .strip_prefix(root)
        .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    let dlc = rel.iter().any(|c| c.eq_ignore_ascii_case(skip[1]));
    let parts: Vec<String> = rel
        .into_iter()
        .filter(|c| {
            let lower = c.to_lowercase();
            !skip.contains(&lower.as_str()) && !LAYOUT_NAMES.contains(&lower.as_str())
        })
        .collect();
    (parts.join("/"), dlc, parts)
}

fn stage_layout(
    content: &str,
    aoc: &str,
    game: Option<&crate::savage_wind::GameFiles>,
    root: &Path,
    dirs: &[PathBuf],
    files: &[PathBuf],
    staged: &Path,
) -> Result<Staged> {
    let content_id = content.split('/').next().unwrap_or(content).to_lowercase();
    let aoc_id = aoc.split('/').next().unwrap_or(aoc).to_lowercase();
    let skip = [content_id.as_str(), aoc_id.as_str()];
    let stock_tops = stock_top_names(game);
    let has_game_dirs = |d: &Path| top_names(d).keys().any(|k| stock_tops.contains_key(k));

    let mut romfs_roots: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if romfs_roots.iter().any(|r| d.starts_with(r)) {
            continue;
        }
        let name = name_of(d).to_lowercase();
        let is_romfs = name == "romfs";
        let is_bare_title = (name == content_id || name == aoc_id)
            && !top_names(d).contains_key("romfs")
            && has_game_dirs(d.as_path());
        if is_romfs || is_bare_title {
            romfs_roots.push(d.clone());
        }
    }

    let mut variants: BTreeMap<String, (Vec<String>, Vec<Source>)> = BTreeMap::new();
    if !romfs_roots.is_empty() {
        for r in romfs_roots {
            let (key, dlc, parts) = variant_key(root, &r, &skip);
            let dlc = dlc || name_of(&r).eq_ignore_ascii_case(&aoc_id);
            variants
                .entry(key)
                .or_insert_with(|| (parts, Vec::new()))
                .1
                .push(Source::Romfs { path: r, dlc });
        }
    } else {
        let mut loose_roots: Vec<PathBuf> = Vec::new();
        for d in dirs {
            if loose_roots.iter().any(|r| d.starts_with(r)) {
                continue;
            }
            if has_game_dirs(d.as_path()) {
                loose_roots.push(d.clone());
            }
        }
        for r in loose_roots {
            let (key, _, parts) = variant_key(root, &r, &skip);
            variants
                .entry(key)
                .or_insert_with(|| (parts, Vec::new()))
                .1
                .push(Source::Loose(r));
        }
    }

    if !variants.is_empty() {
        let staged_result = stage_variants(variants, content, aoc, &stock_tops, staged)?;
        if staged_result.count > 0 {
            return Ok(staged_result);
        }
    }

    let count = stage_loose_files(game, files, &staged.join(content), &staged.join(aoc))?;
    Ok(Staged { count, groups: vec![] })
}

fn stage_source(
    source: &Source,
    content_dst: &Path,
    aoc_dst: &Path,
    stock_tops: &HashMap<String, String>,
) -> Result<usize> {
    match source {
        Source::Romfs { path, dlc } => {
            copy_romfs(path, if *dlc { aoc_dst } else { content_dst }, stock_tops)
        }
        Source::Loose(root) => {
            let mut count = 0;
            for (lower, name) in &top_names(root) {
                if lower == "aoc" {
                    let aoc_root = root.join(name);
                    let inner = top_names(&aoc_root);
                    let src = inner.get("0010").map(|n| aoc_root.join(n)).unwrap_or(aoc_root);
                    count += copy_romfs(&src, aoc_dst, stock_tops)?;
                } else if let Some(fixed) = stock_tops.get(lower) {
                    count += copy_tree(&root.join(name), &content_dst.join(fixed))?;
                }
            }
            Ok(count)
        }
    }
}

fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if slug.trim_matches('_').is_empty() { "option".to_string() } else { slug }
}

fn stage_variants(
    variants: BTreeMap<String, (Vec<String>, Vec<Source>)>,
    content: &str,
    aoc: &str,
    stock_tops: &HashMap<String, String>,
    staged: &Path,
) -> Result<Staged> {
    if variants.len() == 1 {
        let mut count = 0;
        for (_, (_, sources)) in variants {
            for source in &sources {
                count += stage_source(source, &staged.join(content), &staged.join(aoc), stock_tops)?;
            }
        }
        return Ok(Staged { count, groups: vec![] });
    }

    let all_parts: Vec<&Vec<String>> = variants.values().map(|(parts, _)| parts).collect();
    let common = (0..)
        .take_while(|i| {
            let first = all_parts[0].get(*i);
            first.is_some() && all_parts.iter().all(|p| p.get(*i) == first)
        })
        .count();
    let label = |parts: &Vec<String>| -> String {
        let rest = parts.iter().skip(common).cloned().collect::<Vec<_>>().join(" / ");
        if rest.is_empty() { "Main files".to_string() } else { rest.replace(['_', '-'], " ") }
    };

    let mut count = 0;
    let mut base_count = 0;
    let mut options: Vec<ModOption> = Vec::new();
    let mut used = std::collections::HashSet::new();
    for (_, (parts, sources)) in &variants {
        let name = label(parts);
        let lower = name.to_lowercase();
        let is_base = parts.len() <= common
            || BASE_WORDS.iter().any(|w| lower.split(|c: char| !c.is_alphanumeric()).any(|t| t == *w));
        if is_base {
            for source in sources {
                base_count += stage_source(source, &staged.join(content), &staged.join(aoc), stock_tops)?;
            }
            continue;
        }
        let mut folder = slug(&name);
        let mut n = 2;
        while !used.insert(folder.clone()) {
            folder = format!("{}_{n}", slug(&name));
            n += 1;
        }
        let opt_root = staged.join("options").join(&folder);
        let mut opt_count = 0;
        for source in sources {
            opt_count += stage_source(source, &opt_root.join(content), &opt_root.join(aoc), stock_tops)?;
        }
        if opt_count == 0 {
            continue;
        }
        count += opt_count;
        options.push(ModOption {
            name: name.as_str().into(),
            description: Default::default(),
            path: PathBuf::from(&folder),
            requires: vec![],
        });
    }
    count += base_count;
    let groups = match options.len() {
        0 => vec![],
        1 => vec![OptionGroup::Multiple(MultipleOptionGroup {
            name: "Extras".into(),
            description: "Optional parts of this mod".into(),
            required: false,
            defaults: options.iter().map(|o| o.path.clone()).collect(),
            options,
        })],
        _ => vec![OptionGroup::Exclusive(ExclusiveOptionGroup {
            name: "Version".into(),
            description: "This mod comes in several versions. Pick one.".into(),
            required: base_count == 0,
            default: options.first().map(|o| o.path.clone()),
            options,
        })],
    };
    if !groups.is_empty() && base_count == 0 {
        fs::create_dir_all(staged.join(content))?;
    }
    Ok(Staged { count, groups })
}

fn stage_loose_files(
    game: Option<&crate::savage_wind::GameFiles>,
    files: &[PathBuf],
    content_dst: &Path,
    aoc_dst: &Path,
) -> Result<usize> {
    let Some(game) = game else {
        return Ok(0);
    };
    let loose: Vec<&PathBuf> = files
        .iter()
        .filter(|f| !SKIP_EXTS.contains(&extension(f).as_str()))
        .filter(|f| !ARCHIVES.contains(&extension(f).as_str()))
        .filter(|f| !NOT_ROMFS_EXTS.contains(&extension(f).as_str()))
        .collect();
    if loose.is_empty() {
        return Ok(0);
    }
    log::info!("Matching {} loose files to game paths", loose.len());
    let mut index: HashMap<String, (bool, PathBuf)> = HashMap::new();
    let add_index = |root: &Path, dlc: bool, index: &mut HashMap<String, (bool, PathBuf)>| {
        for entry in jwalk::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let path = entry.path();
                if let Ok(rel) = path.strip_prefix(root) {
                    index
                        .entry(name_of(&path).to_lowercase())
                        .or_insert((dlc, rel.to_path_buf()));
                }
            }
        }
    };
    add_index(game.romfs.as_path(), false, &mut index);
    for aoc_dir in &game.all_aoc {
        add_index(aoc_dir.as_path(), true, &mut index);
    }
    let mut count = 0;
    for file in loose {
        let lower = name_of(file).to_lowercase();
        let ext = extension(file);
        let stem = lower.trim_end_matches(&format!(".{ext}")).to_string();
        let candidates = [
            lower.clone(),
            format!("{stem}.s{ext}"),
            ext.strip_prefix('s')
                .map(|e| format!("{stem}.{e}"))
                .unwrap_or_default(),
        ];
        let Some((dlc, rel)) = candidates.iter().find_map(|c| index.get(c)) else {
            log::warn!("Could not find where {} goes in the game, leaving it out", file.display());
            continue;
        };
        let to = if *dlc { aoc_dst.join(rel) } else { content_dst.join(rel) };
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        log::info!("{} goes to {}", file.display(), rel.display());
        fs::copy(file, &to)?;
        count += 1;
    }
    Ok(count)
}

fn unsupported_reason(dirs: &[PathBuf], files: &[PathBuf]) -> Option<&'static str> {
    let has_dir = |name: &str| dirs.iter().any(|d| name_of(d).eq_ignore_ascii_case(name));
    let has_ext = |exts: &[&str]| files.iter().any(|f| exts.contains(&extension(f).as_str()));
    if has_ext(&["nsp", "xci", "nca", "nsz", "xcz"]) {
        Some("It contains a game or update dump (NSP/XCI/NCA), not a mod. Mods are made of game files such as Actor/Pack/... .")
    } else if has_dir("exefs") || has_ext(&["ips", "pchtxt"]) {
        Some("It only patches the game's code (exefs, IPS or pchtxt). Savage Wind runs its own recompiled code, so these patches can't apply. Look for a Savage Wind version of this mod or a setting in Savage Wind itself.")
    } else if has_dir("cheats") {
        Some("It only contains cheat codes, which Savage Wind can't use.")
    } else if has_dir("content") || has_dir("aoc") {
        Some("It's a Wii U mod (content/aoc folders). Wii U files don't work on Switch; look for the Switch version.")
    } else if has_ext(&["tkcl"]) {
        Some("It's a Tears of the Kingdom mod (.tkcl), which doesn't work with Breath of the Wild.")
    } else {
        None
    }
}

fn extract_archive(path: &Path, ext: &str) -> Result<PathBuf> {
    let temp = util::get_temp_folder().to_path_buf();
    log::info!("Extracting {}", path.display());
    let mut magic = [0u8; 8];
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = std::io::Read::read(&mut file, &mut magic);
    }
    let kind = if magic.starts_with(b"PK") {
        "zip"
    } else if magic.starts_with(b"Rar!") {
        "rar"
    } else if magic.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        "7z"
    } else {
        ext
    };
    let native: Result<()> = match kind {
        "zip" => zip::ZipArchive::new(BufReader::new(fs::File::open(path)?))
            .context("Failed to open ZIP")
            .and_then(|mut zip| zip.extract(&temp).context("Failed to extract ZIP")),
        "7z" => sevenz_rust::decompress_file(path, &temp).context("Failed to extract 7Z file"),
        "rar" => (|| -> Result<()> {
            let mut archive = unrar::Archive::new(path)
                .open_for_processing()
                .context("Failed to open RAR for extraction")?;
            while let Some(header) = archive.read_header().context("Failed to read RAR header")? {
                archive = if header.entry().is_file() {
                    header
                        .extract_with_base::<&Path>(temp.as_path())
                        .context("Failed to extract RAR file")?
                } else {
                    header.skip().context("Failed to skip RAR folder")?
                };
            }
            Ok(())
        })(),
        other => Err(anyhow_ext::anyhow!("No built-in support for .{other} archives")),
    };
    match native {
        Ok(()) => Ok(temp),
        Err(e) => {
            log::warn!("Built-in extraction failed ({e}), trying installed tools");
            if external_extract(path, &temp) {
                Ok(temp)
            } else {
                Err(e.context(format!(
                    "Could not extract {}. Installing 7-Zip lets SWMM open more archive types.",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
                )))
            }
        }
    }
}

fn external_extract(path: &Path, dest: &Path) -> bool {
    let mut tools: Vec<(PathBuf, Vec<std::ffi::OsString>)> = Vec::new();
    let seven_args = vec![
        "x".into(),
        "-y".into(),
        path.as_os_str().to_os_string(),
        format!("-o{}", dest.display()).into(),
    ];
    for exe in [
        "7z",
        "7za",
        "C:\\Program Files\\7-Zip\\7z.exe",
        "C:\\Program Files (x86)\\7-Zip\\7z.exe",
    ] {
        tools.push((PathBuf::from(exe), seven_args.clone()));
    }
    tools.push((PathBuf::from("tar"), vec![
        "-xf".into(),
        path.as_os_str().to_os_string(),
        "-C".into(),
        dest.as_os_str().to_os_string(),
    ]));
    for (exe, args) in tools {
        let mut command = std::process::Command::new(&exe);
        command
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        if let Ok(status) = command.status() {
            if status.success() && fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(false) {
                log::info!("Extracted with {}", exe.display());
                return true;
            }
        }
    }
    false
}

fn zip_folder(dir: &Path) -> Result<PathBuf> {
    let out = util::get_temp_folder().to_path_buf().join(format!(
        "{}.zip",
        dir.file_name().and_then(|n| n.to_str()).unwrap_or("mod")
    ));
    let mut writer = zip::ZipWriter::new(fs::File::create(&out)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    for file in files_in(dir) {
        let rel = file
            .strip_prefix(dir)?
            .to_string_lossy()
            .replace('\\', "/");
        writer.start_file(rel, options)?;
        writer.write_all(&fs::read(&file)?)?;
    }
    writer.finish()?;
    Ok(out)
}
