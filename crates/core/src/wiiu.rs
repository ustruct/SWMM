use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow_ext::{Context, Result, bail};
use fs_err as fs;
use roead::{
    Endian,
    byml::Byml,
    sarc::{Sarc, SarcWriter},
    yaz0,
};

#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub converted: usize,
    pub textures: usize,
    pub stock: usize,
    pub not_needed: usize,
    pub skipped: Vec<(String, String)>,
}

thread_local! {
    static SUMMARY: std::cell::RefCell<Summary> = std::cell::RefCell::new(Summary::default());
}

pub fn record(report: &Report) {
    SUMMARY.with(|s| {
        let mut s = s.borrow_mut();
        s.converted += report.converted;
        s.textures += report.textures;
        s.stock += report.stock;
        s.not_needed += report.not_needed;
        s.skipped.extend(report.skipped.iter().cloned());
    });
}

pub fn take_summary() -> Summary {
    SUMMARY.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub converted: usize,
    pub copied: usize,
    pub skipped: Vec<(String, String)>,
    pub textures: usize,
    pub notes: Vec<String>,
    pub stock: usize,
    pub game: crate::stock::Game,
    pub neutral: Vec<(String, Vec<&'static str>)>,
    pub tex_stems: Option<std::collections::HashSet<String>>,
    pub not_needed: usize,
}

fn own_tex(name: &str, model_dir: Option<&Path>, tex_stems: Option<&std::collections::HashSet<String>>) -> Option<String> {
    let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
    let stem = file.strip_suffix(".sbfres").or_else(|| file.strip_suffix(".bfres"))?;
    let base = stem.rsplit_once('-').filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(b, _)| b);
    std::iter::once(stem).chain(base).find_map(|s| {
        let candidate = if dir.is_empty() { s.to_string() } else { format!("{dir}/{s}") };
        let found = match (tex_stems, model_dir.and_then(Path::parent)) {
            (Some(set), _) => set.contains(&candidate),
            (None, Some(src)) => src.join(format!("{candidate}.Tex1.sbfres")).is_file(),
            (None, None) => false,
        };
        found.then_some(candidate)
    })
}

pub const NOT_NEEDED: &str = "not needed on Switch";

impl Report {
    pub fn leave_out(&mut self, name: String, why: &str) {
        if why == NOT_NEEDED {
            log::info!("{name}: a file the Switch game doesn't use (left out)");
            self.not_needed += 1;
        } else {
            self.skipped.push((name, why.to_string()));
        }
    }
}

fn stock_or(raw: &[u8], compressed: bool, report: &mut Report, why: &'static str) -> std::result::Result<Vec<u8>, &'static str> {
    let Some(path) = crate::stock::switch_twin(raw) else {
        return Err(if crate::stock::wiiu_only(raw) { NOT_NEEDED } else { why });
    };
    match report.game.read(path) {
        Some(d) => {
            report.stock += 1;
            Ok(if compressed { yaz0::compress(&d) } else { d })
        }
        None => Err("the game's own file, unchanged: left out (in the game's archives and folders the Switch game's is used)"),
    }
}

const SAME_ON_BOTH: &[&str] = &[
    "hght", "mate", "extm",
    "bxml", "bphysics", "bas", "baslist", "baiprog", "bgparamlist", "bdrop", "bshop", "brecipe", "blod", "bchemical",
    "bdmgparam", "bmodellist", "bumii", "bawareness", "bslink", "bgapkginfo", "bgapkglist", "batcllist", "batcl",
    "baischedule", "bfevfl_unused", "brgconfiglist", "brgconfig", "brgbw", "bgsdw", "bgapkg", "bphyssb", "bmscdef",
    "blifecondition", "bxml", "bactcapt", "bdgnenv", "bcamera", "txt", "json", "yml", "yaml",
    "bfevfl", "bfevtm", "agstats", "blwp", "tscb", "fxparam", "jpg", "png",
    "kybin",
];

const NOT_YET: &[(&str, &str)] = &[
    ("bfres", "model"),
    ("bntx", "texture"),
    ("bflim", "texture"),
    ("hkx", "Havok physics"),
    ("hksc", "Havok physics"),
    ("hktmrb", "Havok physics"),
    ("hkcl", "Havok cloth"),
    ("hkrb", "Havok physics"),
    ("hkrg", "Havok physics"),
    ("bfwav", "sound"),
    ("bfsar", "sound"),
    ("bfstp", "sound"),
    ("ptcl", "effect"),
    ("esetlist", "effect"),
    ("bnvib", "vibration"),
    ("bfsha", "shader"),
    ("sharcb", "shader"),
    ("sharc", "shader"),
    ("bfotf", "font"),
    ("bfttf", "font"),
    ("bfcpx", "font"),
    ("mp4", "movie"),
];

fn ext_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    
    base.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn plain_ext(e: &str) -> &str {
    match e {
        "ssarc" => "sarc",
        "sbyml" => "byml",
        "smubin" => "mubin",
        _ if e.len() > 1 && e.starts_with('s') && !matches!(e, "sarc" | "sharc" | "sharcb" | "srsizetable") => &e[1..],
        _ => e,
    }
}

fn is_byml(d: &[u8]) -> bool {
    d.len() > 4 && (&d[..2] == b"BY" || &d[..2] == b"YB")
}

pub type Skipped = Vec<(String, String)>;

pub fn convert_file_lenient(name: &str, data: &[u8]) -> std::result::Result<(Vec<u8>, Skipped), String> {
    let mut r = Report::default();
    let out = convert(name, data, None, &mut r).map_err(str::to_string)?;
    Ok((out, r.skipped))
}

pub fn convert_with(name: &str, data: &[u8], report: &mut Report) -> std::result::Result<Vec<u8>, &'static str> {
    convert(name, data, None, report)
}

pub fn convert_file(name: &str, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut r = Report::default();
    let out = convert(name, data, None, &mut r).map_err(str::to_string)?;
    if let Some((f, why)) = r.skipped.first() {
        return Err(format!("{f}: {why}"));
    }
    Ok(out)
}

fn pack_textures(
    sarc: &Sarc,
    model_dir: Option<&Path>,
    w: &mut SarcWriter,
    report: &mut Report,
) -> std::collections::HashSet<String> {
    let mut used = std::collections::HashSet::new();
    let names: Vec<String> = sarc.files().filter_map(|f| f.name().map(str::to_string)).collect();
    let mut stems: Vec<String> = names
        .iter()
        .filter_map(|n| {
            n.strip_suffix(".Tex2.sbfres")
                .or_else(|| n.strip_suffix(".Tex1.sbfres"))
                .map(str::to_string)
                .or_else(|| tex1_part_stem(n).map(|s| format!("{}{s}", &n[..n.rfind('/').map_or(0, |i| i + 1)])))
        })
        .collect();
    stems.sort();
    stems.dedup();
    for stem in stems {
        let file = stem.rsplit('/').next().unwrap_or(&stem).to_string();
        let from_pack = |n: &str| sarc.get_data(n).map(|d| d.to_vec());
        let loose = |n: &str| model_dir.and_then(|d| fs::read(d.join(n)).ok());
        let Some(first) = from_pack(&format!("{stem}.Tex1.sbfres")).or_else(|| loose(&format!("{file}.Tex1.sbfres")))
        else {
            let unchanged = from_pack(&format!("{stem}.Tex2.sbfres"))
                .map(|d| if d.starts_with(b"Yaz0") { yaz0::decompress(&d).map(|d| d.to_vec()).unwrap_or_default() } else { d })
                .is_some_and(|d| crate::stock::wiiu_only(&d));
            report.leave_out(
                format!("{stem}.Tex2.sbfres"),
                if unchanged { NOT_NEEDED } else { "mip levels without their texture file (Tex1)" },
            );
            used.insert(format!("{stem}.Tex2.sbfres"));
            continue;
        };
        let mut parts = vec![first];
        for k in 1.. {
            match from_pack(&format!("{stem}.Tex1.{k}.sbfres")).or_else(|| loose(&format!("{file}.Tex1.{k}.sbfres"))) {
                Some(d) => {
                    used.insert(format!("{stem}.Tex1.{k}.sbfres"));
                    parts.push(d);
                }
                None => break,
            }
        }
        let tex2 = from_pack(&format!("{stem}.Tex2.sbfres")).or_else(|| loose(&format!("{file}.Tex2.sbfres")));
        used.insert(format!("{stem}.Tex1.sbfres"));
        used.insert(format!("{stem}.Tex2.sbfres"));
        let parts: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        match crate::tex::convert_wiiu_parts(&file, &parts, tex2.as_deref()) {
            Ok((data, r)) => {
                w.add_file(format!("{stem}.Tex.sbfres"), data);
                report.converted += 1;
                report.textures += r.textures;
                report.notes.extend(r.notes.into_iter().map(|n| format!("{file}: {n}")));
            }
            Err(e) => report.skipped.push((format!("{stem}.Tex1.sbfres"), format!("texture: {e:#}"))),
        }
    }
    used
}

fn switch_layout_textures(name: &str, report: &mut Report) -> Option<Vec<u8>> {
    let file = name.rsplit('/').next()?;
    let file = match file.strip_suffix(".blarc") {
        Some(stem) => format!("{stem}.sblarc"),
        None => file.to_string(),
    };
    let inner = format!("Layout/{file}");
    ["", "Pack/Bootup.pack//", "Pack/Title.pack//", "Pack/TitleBG.pack//"]
        .iter()
        .find_map(|pack| report.game.read(&format!("{pack}{inner}")))
        .and_then(|d| Sarc::new(d.as_slice()).ok()?.get_data("timg/__Combined.bntx").map(<[u8]>::to_vec))
}

pub fn layout_textures(archive: &str, images: &[(String, Vec<u8>)], report: &mut Report) -> anyhow_ext::Result<Vec<u8>> {
    match switch_layout_textures(archive, report) {
        Some(theirs) => merged_layout_textures(&theirs, images),
        None => crate::tex::flim::combined(images),
    }
}

fn merged_layout_textures(theirs: &[u8], images: &[(String, Vec<u8>)]) -> anyhow_ext::Result<Vec<u8>> {
    let mut textures = crate::tex::bntx::parse(theirs)?;
    for (n, d) in images {
        let plain = if d.starts_with(b"Yaz0") { yaz0::decompress(d)? } else { d.clone() };
        if crate::stock::is_game_file(&plain) && textures.iter().any(|t| &t.name == n) {
            continue;
        }
        let t = crate::tex::flim::to_texture(n, d).map_err(|e| anyhow_ext::anyhow!("{n}: {e:#}"))?;
        match textures.iter_mut().find(|x| x.name == t.name) {
            Some(x) => *x = t,
            None => textures.push(t),
        }
    }
    textures.sort_by(|a, b| a.name.cmp(&b.name));
    crate::tex::bntx::write("__Combined", &textures)
}

fn convert(
    name: &str,
    data: &[u8],
    model_dir: Option<&Path>,
    report: &mut Report,
) -> std::result::Result<Vec<u8>, &'static str> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let compressed = data.len() > 4 && &data[..4] == b"Yaz0";
    if compressed && data.len() >= 8 && data[4..8] == [0, 0, 0, 0] {
        return Ok(data.to_vec());
    }
    let raw = if compressed { yaz0::decompress(data).map_err(|_| "broken Yaz0 data")? } else { data.to_vec() };
    if raw.starts_with(b"SARC") && crate::stock::wiiu_only(&raw) {
        return Err(NOT_NEEDED);
    }
    if !raw.starts_with(b"SARC")
        && let Some(d) = crate::stock::switch_twin(&raw).and_then(|path| report.game.read(path)) {
            report.stock += 1;
            return Ok(if compressed { yaz0::compress(&d) } else { d });
        }
    let e = ext_of(name);
    if matches!(e.as_str(), "dae" | "fbx" | "obj" | "blend" | "psd" | "xcf" | "tga" | "dds" | "ini" | "log" | "py" | "bat" | "md" | "db") {
        return Err(NOT_NEEDED);
    }
    let p = plain_ext(&e);
    let wiiu_fres = raw.len() > 0x70 && &raw[..4] == b"FRES" && raw[8..10] == [0xfe, 0xff];
    let groups = |i: usize| if wiiu_fres { u16::from_be_bytes([raw[0x50 + 2 * i], raw[0x51 + 2 * i]]) } else { 0 };
    if wiiu_fres && p != "bfres" && groups(1) > 0 && groups(0) == 0 && (2..11).all(|g| groups(g) == 0) {
        let out = crate::tex::convert_wiiu_container(&raw).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "texture file SWMM can't convert yet (see the log)"
        })?;
        report.converted += 1;
        report.textures += groups(1) as usize;
        return Ok(if compressed { yaz0::compress(&out) } else { out });
    }
    if wiiu_fres && (p == "bfres" || groups(1) == 0) {
        let mut file = crate::bfres::wiiu::read(&raw, false).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "model SWMM can't convert yet (material visibility animations, morphs or an unusual format: see the log)"
        })?;
        let tex = own_tex(name, model_dir, report.tex_stems.as_ref());
        let fixes = crate::bfres::fix_dangling_samplers(&mut file, tex.is_some());
        if !fixes.is_empty() {
            let mut needed: Vec<&'static str> = fixes.iter().filter_map(|f| f.texture).collect();
            needed.sort_unstable();
            needed.dedup();
            for f in &fixes {
                let how = match f.texture {
                    Some(t) => format!("given {t}"),
                    None => "pointed at the material's first texture".to_string(),
                };
                log::info!(
                    "{name}: material {} uses texture {} for {}, but has no such texture ({how}, so the game doesn't crash)",
                    f.material, f.missing, f.shader_sampler
                );
            }
            if let (Some(tex), false) = (tex, needed.is_empty()) {
                report.neutral.push((tex, needed));
            }
        }
        let out = crate::bfres::switch::write(&file).map_err(|_| "model that couldn't be written")?;
        report.converted += 1;
        return Ok(if compressed { yaz0::compress(&out) } else { out });
    }
    if raw.starts_with(&[0x57, 0xe0, 0xe0, 0x57, 0x10, 0xc0, 0xc0, 0x10]) {
        if let Ok(out) = stock_or(&raw, compressed, report, "") {
            return Ok(out);
        }
        return match crate::havok::to_switch(&raw) {
            Ok(out) => {
                report.converted += 1;
                Ok(if compressed { yaz0::compress(&out) } else { out })
            }
            Err(e) => {
                log::warn!("{name}: {e:#}");
                Err("Havok file SWMM can't convert (see the log)")
            }
        };
    }
    if matches!(p, "srsizetable" | "rsizetable") {
        return Err(NOT_NEEDED);
    }
    if p == "esetlist" && let Some(out) = crate::effect::to_switch(name, &raw, &mut report.game) {
        report.converted += 1;
        return Ok(if compressed { yaz0::compress(&out) } else { out });
    }
    if let Some((_, why)) = NOT_YET.iter().find(|(x, _)| *x == p) {
        return stock_or(&raw, compressed, report, why);
    }
    let out = if crate::tables::known(p) {
        crate::tables::to_switch(p, &raw).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "table SWMM can't convert (see the log)"
        })?
    } else if raw.len() > 4 && &raw[..4] == b"BARS" {
        crate::sound::bars_to_switch(&raw, name).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "sound archive SWMM can't convert (see the log)"
        })?
    } else if raw.len() > 4 && matches!(&raw[..4], b"FLYT" | b"FLAN") {
        crate::layout::to_switch(&raw).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "layout SWMM can't convert (see the log)"
        })?
    } else if raw.len() > 4 && &raw[..4] == b"XLNK" {
        let out = crate::xlink::to_switch(&raw).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "effect or sound link table SWMM can't convert (see the log)"
        })?;
        let sound = p == "bslnk";
        let game = if sound { "Pack/Bootup.pack//SLink2/SLink2DB.sbslnk" } else { "Pack/Bootup.pack//ELink2/ELink2DB.sbelnk" };
        match report.game.read(game) {
            Some(theirs) => match crate::xlink::merge(&theirs, &out, sound) {
                Ok((merged, n)) => {
                    report.notes.push(format!("{name}: the game's table with the mod's {n} new or changed users"));
                    merged
                }
                Err(e) => {
                    log::warn!("{name}: {e:#}");
                    report.notes.push(format!("{name}: the mod's changes couldn't be merged; the game's table kept"));
                    theirs
                }
            },
            None => out,
        }
    } else if raw.len() > 4 && &raw[..4] == b"FFNT" {
        crate::font::to_switch(&raw).map_err(|e| {
            log::warn!("{name}: {e:#}");
            "font SWMM can't convert (see the log)"
        })?
    } else if raw.len() > 4 && &raw[..4] == b"FWAV" {
        crate::sound::bfwav_to_switch(&raw).map_err(|_| "sound SWMM can't convert")?
    } else if raw.len() > 4 && &raw[..4] == b"FSTM" {
        crate::sound::bfstm_to_switch(&raw).map_err(|_| "music stream SWMM can't convert")?
    } else if matches!(p, "hght" | "mate" | "extm") || SAME_ON_BOTH.contains(&p) && !is_byml(&raw) {
        raw
    } else if name.ends_with("AnimalUnitSpeed.bin") {
        raw.chunks(4).flat_map(|w| w.iter().rev().copied().collect::<Vec<_>>()).collect()
    } else if raw.len() > 4 && &raw[..4] == b"SARC" {
        let sarc = Sarc::new(raw.as_slice()).map_err(|_| "broken archive")?;
        let mut w = SarcWriter::new(Endian::Little).with_min_alignment(sarc.guess_min_alignment());
        let mut used = pack_textures(&sarc, model_dir, &mut w, report);
        let images: Vec<(String, Vec<u8>)> = sarc
            .files()
            .filter_map(|f| {
                let n = f.name()?;
                let stem = n.strip_prefix("timg/")?.strip_suffix(".bflim")?;
                Some((stem.to_string(), f.data().to_vec()))
            })
            .collect();
        if !images.is_empty() {
            match layout_textures(name, &images, report) {
                Ok(bntx) => {
                    w.add_file("timg/__Combined.bntx", bntx);
                    report.converted += 1;
                    report.textures += images.len();
                    used.extend(images.iter().map(|(n, _)| format!("timg/{n}.bflim")));
                }
                Err(e) => {
                    log::warn!("{name}: {e:#}");
                    report.skipped.push((format!("{name}/timg"), "layout textures SWMM can't convert (see the log)".into()));
                }
            }
        }
        for f in sarc.files() {
            let Some(n) = f.name() else { continue };
            if used.contains(n) {
                continue;
            }
            match convert(n, f.data(), model_dir, report) {
                Ok(d) => w.add_file(n, d),
                Err(why) => report.leave_out(n.to_string(), why),
            }
        }
        w.to_binary()
    } else if is_byml(&raw) {
        let mut b = Byml::from_binary(&raw).map_err(|_| "broken BYML data")?;
        if name.contains("ActorInfo.product")
            && let Some(sizes) = report.game.actor_sizes() {
                sizes.convert_info(&mut b);
            }
        b.to_binary(Endian::Little)
    } else if raw.len() > 8 && &raw[..8] == b"MsgStdBn" {
        let raw = if raw.get(0xc) == Some(&0) { msbt_utf8_to_utf16(&raw).ok_or("broken text file")? } else { raw };
        let msyt = uk_content::message::Msyt::from_msbt_bytes(&raw).map_err(|_| "broken text file")?;
        let out =
            msyt.into_msbt_bytes(uk_content::message::Endianness::Little).map_err(|_| "text file can't be written")?;
        msbt_in_wiiu_order(&raw, &out)
            .filter(|o| uk_content::message::Msyt::from_msbt_bytes(o).is_ok())
            .unwrap_or(out)
    } else if raw.len() > 4 && &raw[..4] == b"AAMP" || SAME_ON_BOTH.contains(&p) {
        raw
    } else {
        return stock_or(&raw, compressed, report, "format SWMM can't convert from Wii U yet");
    };
    report.converted += 1;
    Ok(if compressed { yaz0::compress(&out) } else { out })
}

fn msbt_sections(b: &[u8], big: bool) -> Option<Vec<([u8; 4], usize, usize)>> {
    let u16_at = |o: usize| -> Option<u16> {
        let v: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if big { u16::from_be_bytes(v) } else { u16::from_le_bytes(v) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let v: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if big { u32::from_be_bytes(v) } else { u32::from_le_bytes(v) })
    };
    let n = u16_at(0xe)?;
    let mut o = 0x20;
    let mut out = Vec::new();
    for _ in 0..n {
        let magic: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        let size = u32_at(o + 4)? as usize;
        b.get(o + 16..o + 16 + size)?;
        out.push((magic, o + 16, size));
        o = (o + 16 + size).div_ceil(16) * 16;
    }
    Some(out)
}

fn msbt_labels(b: &[u8], at: usize, big: bool) -> Option<HashMap<Vec<u8>, u32>> {
    let rd = |o: usize| -> Option<u32> {
        let v: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if big { u32::from_be_bytes(v) } else { u32::from_le_bytes(v) })
    };
    let mut out = HashMap::new();
    for i in 0..rd(at)? as usize {
        let (count, off) = (rd(at + 4 + 8 * i)?, rd(at + 8 + 8 * i)? as usize);
        let mut q = at + off;
        for _ in 0..count {
            let len = *b.get(q)? as usize;
            let name = b.get(q + 1..q + 1 + len)?.to_vec();
            out.insert(name, rd(q + 1 + len)?);
            q += 1 + len + 4;
        }
    }
    Some(out)
}

fn msbt_utf8_to_utf16(src: &[u8]) -> Option<Vec<u8>> {
    let be32 = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(src.get(o..o + 4)?.try_into().ok()?)) };
    let mut out = src.get(..0x20)?.to_vec();
    out[0xc] = 1;
    for (magic, at, size) in msbt_sections(src, true)? {
        let mut body = src.get(at..at + size)?.to_vec();
        if &magic == b"TXT2" {
            let n = be32(at)? as usize;
            let offs: Vec<usize> = (0..n).map(|i| be32(at + 4 + 4 * i).map(|v| v as usize)).collect::<Option<_>>()?;
            let utf16 = offs.iter().enumerate().all(|(i, &start)| {
                let end = offs.get(i + 1).copied().unwrap_or(size);
                end >= start + 2 && (end - start) % 2 == 0 && src.get(at + end - 2..at + end) == Some(&[0, 0][..])
            });
            if utf16 {
                out.extend_from_slice(&magic);
                out.extend((body.len() as u32).to_be_bytes());
                out.extend([0; 8]);
                out.extend(body);
                while out.len() % 16 != 0 {
                    out.push(0xab);
                }
                continue;
            }
            let mut texts = Vec::with_capacity(n);
            for (i, &start) in offs.iter().enumerate() {
                let end = offs.get(i + 1).copied().unwrap_or(size);
                let t = src.get(at + start..at + end)?;
                let mut w = Vec::with_capacity(t.len() * 2);
                let mut q = 0;
                while q < t.len() {
                    match t[q] {
                        0x0e => {
                            let psize = u16::from_be_bytes(t.get(q + 5..q + 7)?.try_into().ok()?) as usize;
                            w.extend([0, 0x0e]);
                            w.extend_from_slice(t.get(q + 1..q + 7 + psize)?);
                            q += 7 + psize;
                        }
                        0x0f => {
                            w.extend([0, 0x0f]);
                            w.extend_from_slice(t.get(q + 1..q + 5)?);
                            q += 5;
                        }
                        b => {
                            let len = match b {
                                0..0x80 => 1,
                                0xc0..0xe0 => 2,
                                0xe0..0xf0 => 3,
                                0xf0..0xf8 => 4,
                                _ => return None,
                            };
                            let c = std::str::from_utf8(t.get(q..q + len)?).ok()?.chars().next()?;
                            let mut units = [0u16; 2];
                            for u in c.encode_utf16(&mut units) {
                                w.extend(u.to_be_bytes());
                            }
                            q += len;
                        }
                    }
                }
                texts.push(w);
            }
            body = (n as u32).to_be_bytes().to_vec();
            let mut off = 4 + 4 * n;
            for t in &texts {
                body.extend((off as u32).to_be_bytes());
                off += t.len();
            }
            for t in texts {
                body.extend(t);
            }
        }
        out.extend_from_slice(&magic);
        out.extend((body.len() as u32).to_be_bytes());
        out.extend([0; 8]);
        out.extend(body);
        while out.len() % 16 != 0 {
            out.push(0xab);
        }
    }
    let size = out.len() as u32;
    out.get_mut(0x12..0x16)?.copy_from_slice(&size.to_be_bytes());
    Some(out)
}

fn msbt_in_wiiu_order(wiiu: &[u8], conv: &[u8]) -> Option<Vec<u8>> {
    let ws = msbt_sections(wiiu, true)?;
    let cs = msbt_sections(conv, false)?;
    let find = |s: &[([u8; 4], usize, usize)], m: &[u8; 4]| s.iter().find(|x| &x.0 == m).map(|x| (x.1, x.2));
    let (wl, _) = find(&ws, b"LBL1")?;
    let (cl, _) = find(&cs, b"LBL1")?;
    let (ct, ct_size) = find(&cs, b"TXT2")?;
    let wiiu_labels = msbt_labels(wiiu, wl, true)?;
    let conv_labels = msbt_labels(conv, cl, false)?;
    let by_index: HashMap<u32, &Vec<u8>> = wiiu_labels.iter().map(|(k, v)| (*v, k)).collect();
    let le = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(conv.get(o..o + 4)?.try_into().ok()?)) };
    let n_conv = le(ct)? as usize;
    let mut conv_texts = Vec::with_capacity(n_conv);
    for i in 0..n_conv {
        let start = le(ct + 4 + 4 * i)? as usize;
        let end = if i + 1 < n_conv { le(ct + 8 + 4 * i)? as usize } else { ct_size };
        conv_texts.push(conv.get(ct + start..ct + end)?);
    }
    let be = |o: usize| -> Option<u32> { Some(u32::from_be_bytes(wiiu.get(o..o + 4)?.try_into().ok()?)) };
    let mut out = conv.get(..0x20)?.to_vec();
    for (magic, at, size) in &ws {
        let mut body = wiiu.get(*at..*at + *size)?.to_vec();
        match magic {
            b"LBL1" => {
                let n = be(*at)? as usize;
                body[..4].copy_from_slice(&(n as u32).to_le_bytes());
                for i in 0..n {
                    let (count, off) = (be(at + 4 + 8 * i)?, be(at + 8 + 8 * i)? as usize);
                    body[4 + 8 * i..8 + 8 * i].copy_from_slice(&count.to_le_bytes());
                    body[8 + 8 * i..12 + 8 * i].copy_from_slice(&(off as u32).to_le_bytes());
                    let mut q = off;
                    for _ in 0..count {
                        q += 1 + *body.get(q)? as usize;
                        let v = be(at + q)?;
                        body.get_mut(q..q + 4)?.copy_from_slice(&v.to_le_bytes());
                        q += 4;
                    }
                }
            }
            b"ATR1" => {
                let (count, each) = (be(*at)?, be(at + 4)?);
                body[..4].copy_from_slice(&count.to_le_bytes());
                body[4..8].copy_from_slice(&each.to_le_bytes());
                let table_first = 8 + 4 * count as usize;
                let strings = each == 4
                    || each == 0 && body.len() > 8 && be(at + 8).map(|v| v as usize) == Some(table_first);
                match each {
                    0 if !strings => {}
                    0 | 4 => {
                        let table = 8 + 4 * count as usize;
                        for k in (8..table).step_by(4) {
                            body.get_mut(k..k + 4)?.reverse();
                        }
                        for k in (table..body.len() - (body.len() - table) % 2).step_by(2) {
                            body.swap(k, k + 1);
                        }
                    }
                    _ => return None,
                }
            }
            b"TXT2" => {
                let n = be(*at)? as usize;
                let mut parts = Vec::with_capacity(n);
                for i in 0..n as u32 {
                    parts.push(*conv_texts.get(*conv_labels.get(*by_index.get(&i)?)? as usize)?);
                }
                body = (n as u32).to_le_bytes().to_vec();
                let mut off = 4 + 4 * n;
                for p in &parts {
                    body.extend_from_slice(&(off as u32).to_le_bytes());
                    off += p.len();
                }
                for p in &parts {
                    body.extend_from_slice(p);
                }
            }
            _ => return None,
        }
        out.extend_from_slice(magic);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&body);
        while out.len() % 16 != 0 {
            out.push(0xab);
        }
    }
    let size = out.len() as u32;
    out.get_mut(0x12..0x16)?.copy_from_slice(&size.to_le_bytes());
    Some(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, out) } else { out.push(p) }
        }
    }
}

fn big_endian(raw: &[u8]) -> Option<bool> {
    let data = if raw.starts_with(b"Yaz0") { std::borrow::Cow::Owned(yaz0::decompress(raw).ok()?) } else { std::borrow::Cow::Borrowed(raw) };
    let d = data.as_ref();
    if d.len() > 8 && &d[..4] == b"SARC" {
        Some(d[6..8] == [0xfe, 0xff])
    } else if is_byml(d) {
        Some(&d[..2] == b"BY")
    } else if d.len() > 0x10 && &d[..4] == b"FRES" {
        Some(d[8..10] == [0xfe, 0xff])
    } else {
        None
    }
}

pub fn has_wiiu_files(dir: &Path) -> bool {
    let mut files = Vec::new();
    walk(dir, &mut files);
    let mut checked = 0;
    for f in files {
        if checked >= 40 {
            break;
        }
        if fs::metadata(&f).map(|m| m.len() > 256 << 20).unwrap_or(true) {
            continue;
        }
        let Ok(raw) = fs::read(&f) else { continue };
        if let Some(big) = big_endian(&raw) {
            if big {
                return true;
            }
            checked += 1;
        }
    }
    false
}

pub fn find(dir: &Path) -> Option<(Option<PathBuf>, Option<PathBuf>)> {
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let content = d.join("content");
        let aoc = d.join("aoc").join("0010");
        let aoc_flat = d.join("aoc");
        let has_content = content.is_dir();
        let aoc = if aoc.is_dir() { Some(aoc) } else if aoc_flat.join("Map").is_dir() { Some(aoc_flat) } else { None };
        if has_content || aoc.is_some() {
            return Some((has_content.then_some(content), aoc));
        }
        if depth < 4
            && let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.path().is_dir() {
                        stack.push((e.path(), depth + 1));
                    }
                }
            }
    }
    None
}

fn game_case(rel: &Path, romfs: Option<&Path>, cache: &mut HashMap<PathBuf, Vec<String>>) -> PathBuf {
    let Some(romfs) = romfs else { return rel.to_path_buf() };
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let mut game_dir = romfs.to_path_buf();
    let mut out = PathBuf::new();
    for (i, part) in parts.iter().enumerate() {
        if i + 1 == parts.len() {
            out.push(part);
            break;
        }
        let names = cache.entry(game_dir.clone()).or_insert_with(|| {
            std::fs::read_dir(&game_dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().is_dir())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        });
        let fixed = names.iter().find(|n| n.eq_ignore_ascii_case(part)).cloned().unwrap_or_else(|| part.clone());
        game_dir.push(&fixed);
        out.push(&fixed);
    }
    out
}

fn aamp_paths(list: &roead::aamp::ParameterList, fix: &dyn Fn(&str) -> String, out: &mut HashMap<u32, u32>) {
    for (_, obj) in list.objects.0.iter() {
        for (_, p) in obj.0.iter() {
            if let Ok(s) = p.as_str() {
                let new = fix(s);
                if new != s {
                    out.insert(roead::aamp::hash_name(s), roead::aamp::hash_name(&new));
                }
            }
        }
    }
    for (_, l) in list.lists.0.iter() {
        aamp_paths(l, fix, out);
    }
}

fn aamp_rename(list: &mut roead::aamp::ParameterList, fix: &dyn Fn(&str) -> String, renames: &HashMap<u32, u32>) {
    use roead::aamp::{Name, Parameter};
    let name = |n: &Name| -> Name { renames.get(&n.hash()).map(|&h| Name::from(h)).unwrap_or(*n) };
    let objects = std::mem::take(&mut list.objects.0);
    for (k, mut obj) in objects {
        let params = std::mem::take(&mut obj.0);
        for (pk, mut p) in params {
            if let Ok(s) = p.as_str() {
                let new = fix(s);
                if new != s {
                    p = match p {
                        Parameter::String32(_) => Parameter::String32(new.as_str().into()),
                        Parameter::String64(_) => Parameter::String64(Box::new(new.as_str().into())),
                        Parameter::String256(_) => Parameter::String256(Box::new(new.as_str().into())),
                        _ => Parameter::StringRef(new.into()),
                    };
                }
            }
            obj.0.insert(name(&pk), p);
        }
        list.objects.0.insert(name(&k), obj);
    }
    let lists = std::mem::take(&mut list.lists.0);
    for (k, mut l) in lists {
        aamp_rename(&mut l, fix, renames);
        list.lists.0.insert(name(&k), l);
    }
}

fn loose_texture_part(file: &str) -> Option<(String, bool, u32)> {
    let rest = file.strip_suffix(".sbfres")?;
    for (kind, mips) in [(".Tex1", false), (".Tex2", true)] {
        if let Some(stem) = rest.strip_suffix(kind) {
            return Some((stem.to_string(), mips, 0));
        }
        if let Some((stem, n)) = rest.rsplit_once(&format!("{kind}."))
            && let Some(n) = n.parse::<u32>().ok().filter(|&n| n > 0) {
                return Some((stem.to_string(), mips, n));
            }
    }
    None
}

pub fn tex1_part_stem(name: &str) -> Option<&str> {
    let base = name.strip_suffix(".sbfres")?;
    let (head, n) = base.rsplit_once('.')?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
    let file = head.strip_suffix(".Tex1")?;
    Some(file.rsplit('/').next().unwrap_or(file))
}

pub fn convert_mod(dir: &Path, content_out: &Path, aoc_out: &Path) -> Result<Report> {
    convert_mod_with_game(dir, content_out, aoc_out, None)
}

fn fill_empty_actor_packs(dst: &Path, prefix: &str, report: &mut Report) -> Result<()> {
    let Ok(entries) = fs::read_dir(dst.join("Actor/Pack")) else { return Ok(()) };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".sbactorpack") || fs::metadata(&path).map_or(true, |m| m.len() != 0) {
            continue;
        }
        let Some(stock) = report
            .game
            .read(&format!("{prefix}Actor/Pack/{name}"))
            .or_else(|| report.game.read(&format!("{prefix}Pack/TitleBG.pack//Actor/Pack/{name}")))
        else {
            log::warn!("{name}: an empty actor pack (the mod's files for it are loose), and the game's own pack wasn't found: kept empty");
            continue;
        };
        let sarc = Sarc::new(stock.as_slice()).context("the game's actor pack")?;
        let mut w = SarcWriter::from_sarc(&sarc);
        let mut used = 0;
        for f in sarc.files() {
            let Some(n) = f.name() else { continue };
            if let Ok(d) = fs::read(dst.join(n)) {
                w.add_file(n, if d.starts_with(b"Yaz0") { yaz0::decompress(&d).map(|d| d.to_vec()).unwrap_or(d) } else { d });
                used += 1;
            }
        }
        fs::write(&path, yaz0::compress(w.to_binary()))?;
        log::info!("{name}: built from the game's own pack and {used} of the mod's loose files");
    }
    Ok(())
}

pub fn convert_mod_with_game(dir: &Path, content_out: &Path, aoc_out: &Path, romfs: Option<&Path>) -> Result<Report> {
    let Some((content, aoc)) = find(dir) else { bail!("No Wii U content or aoc folder in {}", dir.display()) };
    let mut report = Report { game: crate::stock::Game::new(romfs), ..Default::default() };
    let mut case_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for (src, dst, prefix) in [(content, content_out.to_path_buf(), ""), (aoc, aoc_out.to_path_buf(), "aoc/")] {
        let Some(src) = src else { continue };
        let mut files = Vec::new();
        walk(&src, &mut files);
        let mut in_packs = std::collections::HashSet::new();
        for f in files.iter().filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("pack"))) {
            let Ok(b) = fs::read(f) else { continue };
            let b = if b.starts_with(b"Yaz0") { yaz0::decompress(&b).unwrap_or_default() } else { b };
            if let Ok(s) = Sarc::new(b.as_slice()) {
                for n in s.files().filter_map(|x| x.name().map(str::to_string)) {
                    if let Some(stem) = n.strip_suffix(".Tex2.sbfres") {
                        in_packs.insert(stem.rsplit('/').next().unwrap_or(stem).to_string());
                    }
                }
            }
        }
        for f in files {
            let rel_raw = f.strip_prefix(&src).context("path")?;
            let rel_fixed = game_case(rel_raw, romfs, &mut case_cache);
            let rel = rel_fixed.as_path();
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            let lower = rel_s.to_ascii_lowercase();
            let file = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if let Some((stem, mips, n)) = loose_texture_part(&file) {
                let _ = lower;
                let part = |kind: &str, k: u32| {
                    f.with_file_name(if k == 0 { format!("{stem}.{kind}.sbfres") } else { format!("{stem}.{kind}.{k}.sbfres") })
                };
                let Some(lead) = (0..64).find(|&k| part("Tex1", k).is_file()) else {
                    let unchanged = fs::read(&f)
                        .map(|d| if d.starts_with(b"Yaz0") { yaz0::decompress(&d).map(|d| d.to_vec()).unwrap_or_default() } else { d })
                        .is_ok_and(|d| crate::stock::wiiu_only(&d));
                    report.leave_out(rel_s, if unchanged { NOT_NEEDED } else { "mip levels without their texture file (Tex1)" });
                    continue;
                };
                if mips || n != lead {
                    continue;
                }
                if lead == 0 && in_packs.contains(stem.as_str()) && !part("Tex2", 0).is_file() {
                    continue;
                }
                let mut parts = Vec::new();
                for k in lead.. {
                    match fs::read(part("Tex1", k)) {
                        Ok(d) => parts.push(d),
                        Err(_) if k == 0 => {}
                        Err(_) => break,
                    }
                }
                let mut tex2 = Vec::new();
                for k in 0.. {
                    match fs::read(part("Tex2", k)) {
                        Ok(d) => tex2.push(d),
                        Err(_) if k == 0 => {}
                        Err(_) => break,
                    }
                }
                let parts: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
                let tex2: Vec<&[u8]> = tex2.iter().map(|p| p.as_slice()).collect();
                match crate::tex::convert_wiiu_split(&stem, &parts, &tex2) {
                    Ok((data, r)) => {
                        let to = dst.join(rel).with_file_name(format!("{stem}.Tex.sbfres"));
                        if let Some(p) = to.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(to, data)?;
                        report.converted += 1;
                        report.textures += r.textures;
                        report.notes.extend(r.notes.into_iter().map(|n| format!("{stem}: {n}")));
                    }
                    Err(e) => report.skipped.push((rel_s, format!("texture: {e:#}"))),
                }
                continue;
            }
            let data = fs::read(&f)?;
            match convert(&rel_s, &data, Some(&src.join("Model")), &mut report) {
                Ok(d) => {
                    let to = dst.join(rel);
                    if let Some(p) = to.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::write(to, d)?;
                }
                Err(why) => report.leave_out(rel_s, why),
            }
        }
        for (stem, names) in std::mem::take(&mut report.neutral) {
            let tex = format!("{stem}.Tex.sbfres");
            let path = dst.join(&tex);
            let fres_name = format!("{}.Tex", stem.rsplit('/').next().unwrap_or(&stem));
            match fs::read(&path).map_err(anyhow_ext::Error::from).and_then(|d| crate::tex::add_textures_to_tex(&d, &fres_name, &names)) {
                Ok(d) => fs::write(&path, d)?,
                Err(e) => log::warn!("{tex}: couldn't add the neutral textures {names:?}: {e:#}"),
            }
        }
        fill_empty_actor_packs(&dst, prefix, &mut report)?;
    }
    if report.converted == 0 {
        bail!(
            "Nothing in this Wii U mod could be converted for Switch ({} files are models, sounds or other formats \
             SWMM can't convert yet)",
            report.skipped.len()
        );
    }
    report.copied = report.converted;
    Ok(report)
}

pub fn convert_bnp_dir(dir: &Path, content: &str, aoc: &str, romfs: Option<&Path>) -> Result<Report> {
    let mut roots = vec![dir.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(dir.join("options")) {
        roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let mut total = Report::default();
    for root in roots {
        let has = root.join("content").is_dir() || root.join("aoc").is_dir();
        if !has {
            continue;
        }
        let out = crate::util::new_temp_dir()?;
        match convert_mod_with_game(&root, &out.join(content), &out.join(aoc), romfs) {
            Ok(r) => {
                total.converted += r.converted;
                total.stock += r.stock;
                total.not_needed += r.not_needed;
                total.skipped.extend(r.skipped);
            }
            Err(e) => log::warn!("{}: {e}", root.display()),
        }
        for d in ["content", "aoc"] {
            if root.join(d).is_dir() {
                fs::remove_dir_all(root.join(d))?;
            }
        }
        for prefix in [content, aoc] {
            let from = out.join(prefix);
            if from.is_dir() {
                let to = root.join(prefix);
                if to.exists() || fs::rename(&from, &to).is_err() {
                    fs::create_dir_all(&to)?;
                    crate::util::copy_dir(&from, &to)?;
                }
            }
        }
        let _ = fs::remove_dir_all(&out);
    }
    let mut log_dirs = vec![dir.join("logs")];
    if let Ok(rd) = std::fs::read_dir(dir.join("options")) {
        log_dirs.extend(rd.flatten().map(|e| e.path().join("logs")).filter(|p| p.is_dir()));
    }
    let json_sep = r"\\";
    let fix = |text: &str| -> String {
        text.replace("aoc/0010/", &format!("{aoc}/"))
            .replace(&format!("aoc{json_sep}0010{json_sep}"), &format!("{}{json_sep}", aoc.replace('/', json_sep)))
            .replace("content/", &format!("{content}/"))
            .replace(&format!("content{json_sep}"), &format!("{}{json_sep}", content.replace('/', json_sep)))
    };
    for logs in log_dirs {
        let mut files = Vec::new();
        walk(&logs, &mut files);
        for f in files {
            let e = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(e.as_str(), "json" | "yml" | "yaml") {
                let text = fs::read_to_string(&f)?;
                let new = fix(&text);
                if new != text {
                    fs::write(&f, new)?;
                }
            } else if e == "aamp" {
                let Ok(mut pio) = roead::aamp::ParameterIO::from_binary(fs::read(&f)?) else {
                    log::warn!("{}: a log that can't be read", f.display());
                    continue;
                };
                let mut renames = HashMap::new();
                aamp_paths(&pio.param_root, &fix, &mut renames);
                if !renames.is_empty() {
                    aamp_rename(&mut pio.param_root, &fix, &renames);
                    fs::write(&f, pio.to_binary())?;
                }
            }
        }
    }
    if !total.skipped.is_empty() {
        log::warn!("{} files of this Wii U BNP can't be converted for Switch yet and were left out", total.skipped.len());
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byml_and_sarc_to_little_endian() {
        let mut map = roead::byml::Map::default();
        map.insert("Hello".into(), Byml::I32(5));
        let byml = Byml::Map(map).to_binary(Endian::Big);
        assert_eq!(big_endian(&byml), Some(true));
        let sarc = SarcWriter::new(Endian::Big).with_file("Actor/Test.byml", byml).to_binary();
        let mut r = Report::default();
        let out = convert("Pack/Test.pack", &sarc, None, &mut r).unwrap();
        let s = Sarc::new(out.as_slice()).unwrap();
        assert_eq!(s.endian(), Endian::Little);
        let inner = s.get_data("Actor/Test.byml").unwrap();
        assert_eq!(&inner[..2], b"YB");
        assert_eq!(r.converted, 2);
        assert!(convert("Model/X.sbfres", b"FRES....", None, &mut r).is_err());
    }

    #[test]
    fn empty_actor_pack_is_built_from_the_game_and_loose_files() {
        let root = std::env::temp_dir().join(format!("swmm-actorpack-{}", std::process::id()));
        let (romfs, dst) = (root.join("romfs"), root.join("out"));
        fs::create_dir_all(romfs.join("Actor/Pack")).unwrap();
        fs::create_dir_all(dst.join("Actor/Pack")).unwrap();
        fs::create_dir_all(dst.join("Actor/ActorLink")).unwrap();
        let stock = SarcWriter::new(Endian::Little)
            .with_file("Actor/ActorLink/Foo.bxml", b"stock link".to_vec())
            .with_file("Actor/ModelList/Foo.bmodellist", b"stock models".to_vec())
            .to_binary();
        fs::write(romfs.join("Actor/Pack/Foo.sbactorpack"), yaz0::compress(&stock)).unwrap();
        fs::write(dst.join("Actor/Pack/Foo.sbactorpack"), b"").unwrap();
        fs::write(dst.join("Actor/ActorLink/Foo.bxml"), b"mod link").unwrap();
        let mut r = Report { game: crate::stock::Game::new(Some(&romfs)), ..Default::default() };
        fill_empty_actor_packs(&dst, "", &mut r).unwrap();
        let built = yaz0::decompress(&fs::read(dst.join("Actor/Pack/Foo.sbactorpack")).unwrap()).unwrap();
        let s = Sarc::new(built.as_slice()).unwrap();
        assert_eq!(s.get_data("Actor/ActorLink/Foo.bxml"), Some(&b"mod link"[..]));
        assert_eq!(s.get_data("Actor/ModelList/Foo.bmodellist"), Some(&b"stock models"[..]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_yaz0_stub_is_kept() {
        let mut stub = b"Yaz0".to_vec();
        stub.extend([0u8; 12]);
        stub.push(0xff);
        let sarc = SarcWriter::new(Endian::Big)
            .with_file("Armor_022_Head_B.sbactorpack", stub.clone())
            .to_binary();
        let mut r = Report::default();
        let out = convert("Actor/Pack/Armor_022_Head_B.sbactorpack", &yaz0::compress(&sarc), None, &mut r).unwrap();
        let s = Sarc::new(yaz0::decompress(&out).unwrap()).unwrap();
        assert_eq!(s.endian(), Endian::Little);
        assert_eq!(s.get_data("Armor_022_Head_B.sbactorpack"), Some(stub.as_slice()));
        assert!(r.skipped.is_empty());
        assert_eq!(convert("Pack/AocMainField.pack", &[], None, &mut r), Ok(Vec::new()));
    }
}
