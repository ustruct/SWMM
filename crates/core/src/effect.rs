use std::collections::HashMap;
use std::sync::OnceLock;

static TABLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/wiiu/effects.txt"));

const NONE: u32 = 0xffff_ffff;
const RESOURCES: &[&[u8; 4]] = &[b"TEXA", b"PRMA", b"SHDA", b"GRTF", b"G3PR", b"GRSN", b"GRSC"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Swap,
    Raw,
    Wide,
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub magic: [u8; 4],
    pub data: usize,
    pub end: usize,
}

#[derive(Debug, Default, Clone)]
pub struct Tree {
    pub blocks: Vec<Block>,
    pub textures: Option<(usize, usize)>,
    pub shapes: Option<(usize, usize)>,
    pub prims: Option<(usize, usize)>,
}

fn word(d: &[u8], o: usize, be: bool) -> Option<u32> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    Some(if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
}

fn walk(d: &[u8], be: bool, mut at: usize, tree: &mut Tree, depth: usize, follow: bool) -> Option<()> {
    if depth > 16 {
        return None;
    }
    loop {
        if tree.blocks.len() > 200_000 {
            return None;
        }
        let magic: [u8; 4] = d.get(at..at + 4)?.try_into().ok()?;
        let size = word(d, at + 4, be)? as usize;
        let child = word(d, at + 8, be)?;
        let next = word(d, at + 0xc, be)?;
        let attr = word(d, at + 0x10, be)?;
        let start = word(d, at + 0x14, be)? as usize;
        if RESOURCES.contains(&&magic) {
            let extent = if next != NONE && next != 0 { next as usize } else { d.len().checked_sub(at)? };
            let range = Some((at, at + extent.min(d.len() - at)));
            match &magic {
                b"TEXA" | b"GRTF" => tree.textures = range,
                b"PRMA" => {
                    tree.prims = range;
                    if tree.shapes.is_none() {
                        tree.shapes = range;
                    }
                }
                b"G3PR" => tree.shapes = range,
                _ => {}
            }
        } else {
            let has_child = child != NONE && child != 0;
            let end = match &magic {
                b"EMTR" if attr != NONE => attr as usize,
                b"ESTA" | b"ESET" if has_child => child as usize,
                _ => size,
            };
            if start > end || at + end > d.len() {
                return None;
            }
            tree.blocks.push(Block { magic, data: at + start, end: at + end });
            if has_child && matches!(&magic, b"ESTA" | b"ESET" | b"EMTR") {
                walk(d, be, at + child as usize, tree, depth + 1, true)?;
            }
            if &magic == b"EMTR" && attr != NONE {
                walk(d, be, at + attr as usize, tree, depth + 1, true)?;
            }
        }
        if !follow || next == NONE || next == 0 {
            return Some(());
        }
        at = at.checked_add(next as usize)?;
    }
}

pub fn tree(d: &[u8], be: bool) -> Option<Tree> {
    let start = d.get(..0x100)?.windows(4).position(|w| w == b"ESTA")?;
    let mut t = Tree::default();
    walk(d, be, start, &mut t, 0, true)?;
    Some(t)
}

fn subtree(d: &[u8], be: bool, at: usize) -> Option<Tree> {
    let mut t = Tree::default();
    walk(d, be, at, &mut t, 0, false)?;
    Some(t)
}

fn chain(d: &[u8], be: bool, first: usize) -> Option<Vec<usize>> {
    let mut out = vec![];
    let mut at = first;
    loop {
        out.push(at);
        if out.len() > 100_000 {
            return None;
        }
        let next = word(d, at + 0xc, be)?;
        if next == NONE || next == 0 {
            return Some(out);
        }
        at = at.checked_add(next as usize)?;
    }
}

fn first_child(d: &[u8], be: bool, at: usize) -> Option<usize> {
    let child = word(d, at + 8, be)?;
    (child != NONE && child != 0).then_some(at + child as usize)
}

fn has_name(magic: &[u8; 4]) -> bool {
    matches!(magic, b"EMTR" | b"ESET")
}

pub fn signature(d: &[u8], t: &Tree) -> u32 {
    let mut s = Vec::with_capacity(t.blocks.len() * 72);
    for b in &t.blocks {
        s.extend(b.magic);
        s.extend(((b.end - b.data) as u32).to_le_bytes());
        if has_name(&b.magic) {
            s.extend(d.get(b.data + 0x10..b.data + 0x50).unwrap_or_default());
        }
    }
    crate::stock::crc32(&s)
}

pub fn resource_crc(d: &[u8], range: Option<(usize, usize)>) -> u32 {
    range.and_then(|(a, b)| d.get(a..b)).map(crate::stock::crc32).unwrap_or(0)
}

fn in_ranges(i: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(a, b)| (a..=b).contains(&i))
}

const EMITTER_RAW: &[(usize, usize)] = &[
    (4, 19), (468, 471), (502, 505), (508, 508), (526, 527), (550, 551), (554, 557), (566, 567), (574, 578),
    (580, 580), (594, 596), (615, 617), (632, 632), (635, 635), (640, 640), (643, 644), (648, 648),
    (651, 652), (656, 656), (659, 660), (662, 664), (666, 668), (670, 672),
];
const EMITTER_WIDE: &[usize] = &[542, 562, 588, 638, 646, 654];
const EMITTER_KEEP: &[(usize, usize)] = &[(548, 548), (568, 568), (581, 585)];
const FLAGGED: &[&[u8; 4]] = &[
    b"EAA0", b"EAA1", b"EAC0", b"EAC1", b"EADV", b"EAER", b"EAES", b"EAET", b"EAOV", b"EAPL", b"EASL", b"EASS",
    b"EATR", b"FCLN", b"FCOV", b"FMAG", b"FPAD", b"FRND",
];

fn class(magic: &[u8; 4], len: usize, i: usize) -> Class {
    match magic {
        b"EMTR" => {
            if in_ranges(i, EMITTER_KEEP) {
                Class::Keep
            } else if EMITTER_WIDE.iter().any(|&w| i == w || i == w + 1) {
                Class::Wide
            } else if in_ranges(i, EMITTER_RAW) {
                Class::Raw
            } else {
                Class::Swap
            }
        }
        b"ESET" if (4..=19).contains(&i) => Class::Raw,
        b"CADP" if len == 144 && in_ranges(i, &[(4, 11), (20, 35)]) => Class::Raw,
        m if i == 0 && FLAGGED.contains(&m) => Class::Raw,
        _ => Class::Swap,
    }
}

pub fn same_shape(a: &[u8], ta: &Tree, b: &[u8], tb: &Tree) -> bool {
    ta.blocks.len() == tb.blocks.len()
        && ta.blocks.iter().zip(&tb.blocks).all(|(x, y)| {
            x.magic == y.magic
                && x.end - x.data == y.end - y.data
                && (!has_name(&x.magic) || a.get(x.data + 0x10..x.data + 0x50) == b.get(y.data + 0x10..y.data + 0x50))
        })
}

pub fn carry_over(ours: &[u8], ta: &Tree, out: &mut [u8], tb: &Tree) -> usize {
    let mut changed = 0;
    for (a, b) in ta.blocks.iter().zip(&tb.blocks) {
        let len = a.end - a.data;
        let mut i = 0;
        while 4 * i + 4 <= len {
            let (src, dst) = (a.data + 4 * i, b.data + 4 * i);
            let c = class(&a.magic, len, i);
            let n = if c == Class::Wide && 4 * i + 8 <= len { 8 } else { 4 };
            let mut v = ours[src..src + n].to_vec();
            match c {
                Class::Keep => {}
                Class::Raw => {}
                Class::Swap | Class::Wide => v.reverse(),
            }
            if c != Class::Keep && out[dst..dst + n] != v[..] {
                out[dst..dst + n].copy_from_slice(&v);
                changed += 1;
            }
            i += n / 4;
        }
    }
    changed
}

fn put(out: &mut [u8], at: usize, v: usize) {
    out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
}

pub fn shape_only(a: &Tree, b: &Tree) -> bool {
    a.blocks.len() == b.blocks.len()
        && a.blocks.iter().zip(&b.blocks).all(|(x, y)| x.magic == y.magic && x.end - x.data == y.end - y.data)
}

fn name_at<'a>(d: &'a [u8], t: &Tree) -> &'a [u8] {
    t.blocks.first().and_then(|b| d.get(b.data + 0x10..b.data + 0x50)).unwrap_or_default()
}

fn alike(ours: &[u8], ta: &Tree, theirs: &[u8], tb: &Tree) -> usize {
    let mut n = 0;
    for (a, b) in ta.blocks.iter().zip(&tb.blocks) {
        let len = a.end - a.data;
        for i in 0..len / 4 {
            let (x, y) = (a.data + 4 * i, b.data + 4 * i);
            if class(&a.magic, len, i) == Class::Raw && ours[x..x + 4] == theirs[y..y + 4] {
                n += 1;
            }
        }
    }
    n
}

struct Unit {
    at: usize,
    tree: Tree,
}

fn units(d: &[u8], be: bool, parent: usize) -> Option<Vec<Unit>> {
    let Some(first) = first_child(d, be, parent) else { return Some(vec![]) };
    chain(d, be, first)?
        .into_iter()
        .map(|at| Some(Unit { at, tree: subtree(d, be, at)? }))
        .collect()
}

fn esta(d: &[u8]) -> Option<usize> {
    d.get(..0x100)?.windows(4).position(|w| w == b"ESTA")
}

fn same_emitter_names(ours: &[u8], a: &Tree, theirs: &[u8], b: &Tree) -> bool {
    a.blocks.iter().zip(&b.blocks).skip(1).all(|(x, y)| {
        !has_name(&x.magic) || ours.get(x.data + 0x10..x.data + 0x50) == theirs.get(y.data + 0x10..y.data + 0x50)
    })
}

pub fn rebuild(ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    rebuild_with(ours, theirs, true)
}

pub fn rebuild_with(ours: &[u8], theirs: &[u8], whole_sets: bool) -> Option<Vec<u8>> {
    let (m_esta, t_esta) = (esta(ours)?, esta(theirs)?);
    let t_child = word(theirs, t_esta + 8, false)? as usize;
    let t_next = word(theirs, t_esta + 0xc, false)?;
    let res_start = if t_next != NONE && t_next != 0 { t_esta + t_next as usize } else { theirs.len() };
    let t_sets = units(theirs, false, t_esta)?;
    let mut t_emitters = vec![];
    for set in &t_sets {
        let set_name = name_at(theirs, &set.tree);
        t_emitters.extend(units(theirs, false, set.at)?.into_iter().map(|u| (u, set_name)));
    }
    let m_sets = units(ours, true, m_esta)?;
    let mut out = theirs.get(..t_esta + t_child)?.to_vec();
    let mut last_set: Option<usize> = None;
    for (k, ms) in m_sets.iter().enumerate() {
        let name = name_at(ours, &ms.tree);
        let whole = t_sets
            .iter()
            .enumerate()
            .filter(|(_, u)| whole_sets && shape_only(&ms.tree, &u.tree) && same_emitter_names(ours, &ms.tree, theirs, &u.tree))
            .max_by_key(|(i, u)| (name_at(theirs, &u.tree) == name, *i == k))
            .map(|(_, u)| u);
        let (head, picks): (&Unit, Vec<usize>) = match whole {
            Some(u) => (u, first_child(theirs, false, u.at).map(|c| chain(theirs, false, c)).unwrap_or(Some(vec![]))?),
            None => {
                let head = t_sets.iter().find(|u| name_at(theirs, &u.tree) == name).or(t_sets.first())?;
                let m_emitters = units(ours, true, ms.at)?;
                let mut picks = vec![];
                for me in &m_emitters {
                    let (pick, _) = t_emitters.iter().filter(|(u, _)| shape_only(&me.tree, &u.tree)).max_by_key(|(u, set)| {
                        let same = name_at(theirs, &u.tree) == name_at(ours, &me.tree);
                        (same, same && *set == name, alike(ours, &me.tree, theirs, &u.tree))
                    })?;
                    picks.push(pick.at);
                }
                (head, picks)
            }
        };
        let start = out.len();
        if let Some(prev) = last_set {
            put(&mut out, prev + 0xc, start - prev);
        }
        let head_child = word(theirs, head.at + 8, false)?;
        let head_len = if head_child != NONE && head_child != 0 { head_child as usize } else { word(theirs, head.at + 4, false)? as usize };
        out.extend_from_slice(theirs.get(head.at..head.at + head_len)?);
        write_list(theirs, &picks, &mut out, 0)?;
        let size = out.len() - start;
        put(&mut out, start + 4, size);
        put(&mut out, start + 8, if picks.is_empty() { NONE as usize } else { head_len });
        put(&mut out, start + 0xc, NONE as usize);
        last_set = Some(start);
        let _ = k;
    }
    let size = out.len() - t_esta;
    put(&mut out, t_esta + 4, size);
    if res_start < theirs.len() {
        let mut at = out.len().div_ceil(16) * 16;
        at += (res_start as isize - at as isize).rem_euclid(0x1000) as usize;
        out.resize(at, 0);
        put(&mut out, t_esta + 0xc, at - t_esta);
        out.extend_from_slice(&theirs[res_start..]);
    } else {
        put(&mut out, t_esta + 0xc, NONE as usize);
    }
    if word(theirs, 0x1c, false) == Some(theirs.len() as u32) {
        let len = out.len();
        put(&mut out, 0x1c, len);
    }
    Some(out)
}

fn write_list(theirs: &[u8], list: &[usize], out: &mut Vec<u8>, depth: usize) -> Option<()> {
    let mut prev: Option<usize> = None;
    for &at in list {
        let start = write_emitter(theirs, at, out, depth)?;
        if let Some(p) = prev {
            put(out, p + 0xc, start - p);
        }
        prev = Some(start);
    }
    Some(())
}

fn write_emitter(theirs: &[u8], at: usize, out: &mut Vec<u8>, depth: usize) -> Option<usize> {
    if depth > 16 {
        return None;
    }
    let size = word(theirs, at + 4, false)? as usize;
    let child = word(theirs, at + 8, false)?;
    let attr = word(theirs, at + 0x10, false)?;
    let data = word(theirs, at + 0x14, false)? as usize;
    let data_end = if attr != NONE { attr as usize } else { size };
    if data < 0x20 || data > data_end || data_end > size {
        return None;
    }
    let start = out.len();
    out.extend_from_slice(theirs.get(at..at + 0x20)?);
    let data_at = (start + 0x20).div_ceil(0x100) * 0x100;
    out.resize(data_at, 0);
    out.extend_from_slice(theirs.get(at + data..at + data_end)?);
    let new_attr = if attr != NONE {
        let a = out.len() - start;
        out.extend_from_slice(theirs.get(at + data_end..at + size)?);
        a
    } else {
        NONE as usize
    };
    let new_size = out.len() - start;
    put(out, start + 4, new_size);
    put(out, start + 0xc, NONE as usize);
    put(out, start + 0x10, new_attr);
    put(out, start + 0x14, data_at - start);
    if child != NONE && child != 0 {
        put(out, start + 8, new_size);
        let kids = chain(theirs, false, at + child as usize)?;
        write_list(theirs, &kids, out, depth + 1)?;
    }
    Some(start)
}

pub struct Twin {
    pub path: String,
    pub textures: u32,
    pub shapes: u32,
}

pub struct Table {
    pub by_shape: HashMap<u32, Vec<usize>>,
    pub by_name: HashMap<String, Vec<usize>>,
    pub twins: Vec<Twin>,
}

pub fn parse_table(text: &str) -> Table {
    let mut t = Table { by_shape: HashMap::new(), by_name: HashMap::new(), twins: Vec::new() };
    for l in text.lines().skip(1) {
        let mut it = l.splitn(4, ' ');
        let (Some(sig), Some(tex), Some(shp), Some(path)) = (it.next(), it.next(), it.next(), it.next()) else {
            continue;
        };
        let (Ok(sig), Ok(tex), Ok(shp)) =
            (u32::from_str_radix(sig, 16), u32::from_str_radix(tex, 16), u32::from_str_radix(shp, 16))
        else {
            continue;
        };
        let i = t.twins.len();
        t.twins.push(Twin { path: path.to_string(), textures: tex, shapes: shp });
        t.by_shape.entry(sig).or_default().push(i);
        t.by_name.entry(file_name(path).to_string()).or_default().push(i);
    }
    t
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| parse_table(TABLE))
}

fn file_name(name: &str) -> &str {
    let file = name.rsplit("//").next().unwrap_or(name);
    file.rsplit('/').next().unwrap_or(file)
}

fn candidates(name: &str, sig: u32) -> Vec<&'static Twin> {
    let t = table();
    let file = file_name(name);
    let named: Vec<usize> = t.by_name.get(file).cloned().unwrap_or_default();
    let shaped: Vec<usize> = t.by_shape.get(&sig).cloned().unwrap_or_default();
    let mut order: Vec<usize> = shaped.iter().copied().filter(|i| named.contains(i)).collect();
    order.extend(named.iter().copied());
    order.extend(shaped.iter().copied());
    let mut out: Vec<&Twin> = vec![];
    for i in order {
        if !out.iter().any(|o| std::ptr::eq(*o, &t.twins[i])) {
            out.push(&t.twins[i]);
        }
    }
    out
}

pub fn replace_textures(raw: &[u8], ours: &Tree, mut out: Vec<u8>) -> anyhow_ext::Result<(Vec<u8>, usize)> {
    use anyhow_ext::Context;
    use crate::tex::{bntx, eft};
    let (texa, _) = ours.textures.context("the mod's effect has no texture list")?;
    let theirs = tree(&out, false).context("the Switch effect can't be read")?;
    let (at, end) = theirs.textures.context("the Switch effect has no texture list")?;
    let mods = eft::wiiu_textures(raw, texa)?;
    let stock = eft::switch_textures(&out, at)?;
    let mut names: Vec<(u64, String)> = vec![];
    let mut textures: Vec<bntx::Texture> = vec![];
    let mut changed = 0;
    for m in mods {
        let name = stock.names.iter().find(|(id, _)| *id == m.id).map(|(_, n)| n.clone()).unwrap_or_else(|| eft::texture_name(m.id));
        let same = stock.textures.iter().find(|t| t.name == name).filter(|t| {
            t.levels == m.texture.levels && (t.width, t.height, t.format) == (m.texture.width, m.texture.height, m.texture.format)
        });
        let texture = match same {
            Some(t) => t.clone(),
            None => {
                changed += 1;
                bntx::Texture { name: name.clone(), ..m.texture }
            }
        };
        if !names.iter().any(|(_, n)| *n == name) {
            names.push((m.id, name));
            textures.push(texture);
        }
    }
    for (id, name) in &stock.names {
        if !names.iter().any(|(_, n)| n == name)
            && let Some(t) = stock.textures.iter().find(|t| &t.name == name)
        {
            names.push((*id, name.clone()));
            textures.push(t.clone());
        }
    }
    if changed == 0 && names.len() == stock.names.len() {
        return Ok((out, 0));
    }
    let next = word(&out, at + 0xc, false).context("cut short")?;
    let follows = next != NONE && next != 0 && at + (next as usize) < out.len();
    let rest = if follows { at + next as usize } else { end.max(at) };
    let built = if textures.is_empty() {
        out[at..rest].to_vec()
    } else {
        let blob = bntx::write("output", &textures)?;
        let mut g = eft::build_grtf(at, &out[at..at + 0x20], &names, &blob, follows.then_some(rest));
        if !follows {
            put(&mut g, 0xc, NONE as usize);
        }
        g
    };
    let tail = out.split_off(rest);
    out.truncate(at);
    out.extend(built);
    out.extend(tail);
    if word(&out, 0x1c, false).is_some() {
        let len = out.len();
        put(&mut out, 0x1c, len);
    }
    Ok((out, changed))
}

fn replace_raw_shapes(out: &mut Vec<u8>, at: usize, entries: &[&crate::effect_shape::Prim]) -> anyhow_ext::Result<bool> {
    use anyhow_ext::Context;
    let mut block = crate::effect_shape::raw_list(entries, at);
    let old_next = word(out, at + 0xc, false).context("cut short")?;
    let follows = old_next != NONE && old_next != 0;
    let end = if follows { at + old_next as usize } else { out.len() };
    let mut next_at = end;
    if follows {
        while next_at < (at + block.len()).div_ceil(16) * 16 {
            next_at += 0x1000;
        }
        block.resize(next_at - at, 0);
        put(&mut block, 0xc, next_at - at);
    } else {
        put(&mut block, 0xc, NONE as usize);
    }
    if out.get(at..end) == Some(&block[..]) {
        return Ok(false);
    }
    let old_len = out.len();
    out.splice(at..end, block);
    if word(out, 0x1c, false) == Some(old_len as u32) {
        let len = out.len();
        put(out, 0x1c, len);
    }
    Ok(true)
}

pub fn replace_shapes(raw: &[u8], ours: &Tree, out: &mut Vec<u8>) -> anyhow_ext::Result<usize> {
    use anyhow_ext::{Context, bail};
    use crate::effect_shape as es;
    let (prma, _) = ours.shapes.context("the mod's effect has no shape list")?;
    let theirs = tree(out, false).context("the Switch effect can't be read")?;
    let models = match theirs.shapes {
        Some((g3pr, _)) if out.get(g3pr..g3pr + 4) == Some(b"G3PR") => es::switch_models(out, g3pr)?,
        _ => None,
    };
    let model_shapes: &[es::Shape] = models.as_ref().map(|m| &m.shapes[..]).unwrap_or_default();
    let raw_at = theirs.prims.map(|(a, _)| a);
    let raw_stock: Vec<es::Prim> = match raw_at {
        Some(a) => es::switch_prims(out, a)?,
        None => vec![],
    };
    let known = |id: u64| model_shapes.iter().any(|s| s.id == id) || raw_stock.iter().any(|p| p.id == id);
    let used = |id: u64| raw[..prma].chunks_exact(4).any(|c| c == (id as u32).to_be_bytes());
    let prims: Vec<es::Prim> = es::wiiu_prims(raw, prma)?
        .into_iter()
        .filter(|p| {
            let keep = known(p.id) || used(p.id);
            if !keep {
                log::info!("the mod's particle shape {:#x} isn't used by any emitter, so it's left out", p.id);
            }
            keep
        })
        .collect();
    let taken = |id: u64| prims.iter().any(|p| p.id == id);
    let mut free: Vec<u64> = model_shapes.iter().map(|s| s.id).chain(raw_stock.iter().map(|p| p.id)).filter(|id| !taken(*id)).collect();
    free.sort_unstable();
    free.dedup();
    let mut renames: Vec<(u64, u64)> = vec![];
    let mut raw_ids: Vec<u64> = raw_stock.iter().map(|p| p.id).collect();
    for p in prims.iter().filter(|p| !known(p.id)) {
        let n = p.arrays[es::POSITION].len();
        let model = |id: u64| model_shapes.iter().find(|s| s.id == id);
        let size = |id: u64| {
            model(id).map(|s| s.vertices).or_else(|| raw_stock.iter().find(|r| r.id == id).map(|r| r.arrays[es::POSITION].len())).unwrap_or(0)
        };
        let Some(k) = (0..free.len()).filter(|&k| model(free[k]).is_none_or(|s| es::can_hold(s, p))).min_by_key(|&k| size(free[k]).abs_diff(n))
        else {
            bail!("the mod adds a new particle shape ({:#x}) and the game's effect has no unused shape that it can take the place of", p.id);
        };
        let old = free.remove(k);
        log::info!("new particle shape {:#x} takes the place of the game's unused shape {old:#x}", p.id);
        if model(old).is_some() {
            renames.push((old, p.id));
        }
        for r in raw_ids.iter_mut().filter(|r| **r == old) {
            *r = p.id;
        }
    }
    let mut changed = 0;
    if let Some(at) = raw_at
        && !raw_stock.is_empty()
    {
        let entries: Vec<&es::Prim> =
            raw_ids.iter().zip(&raw_stock).map(|(id, stock)| prims.iter().find(|p| p.id == *id).unwrap_or(stock)).collect();
        if replace_raw_shapes(out, at, &entries)? {
            changed += entries.iter().zip(&raw_stock).filter(|(a, b)| a.id != b.id || a.arrays != b.arrays || a.indices != b.indices).count();
        }
    }
    let in_models: Vec<&es::Prim> =
        prims.iter().filter(|p| model_shapes.iter().any(|s| s.id == p.id) || renames.iter().any(|r| r.1 == p.id)).collect();
    if in_models.is_empty() {
        return Ok(changed);
    }
    let theirs = tree(out, false).context("the Switch effect can't be read")?;
    let (g3pr, _) = theirs.shapes.context("the Switch effect has no shape list")?;
    let mut models = es::switch_models(out, g3pr)?.context("the Switch effect has no particle shape models")?;
    if !renames.is_empty() {
        for (old, new) in &renames {
            let shape = models.shapes.iter().find(|s| s.id == *old).context("shape")?;
            es::rename(out, shape, *new)?;
        }
        models = es::switch_models(out, g3pr)?.context("the Switch effect has no particle shape models")?;
    }
    let mut resized = !renames.is_empty();
    for p in &in_models {
        let shape = models.shapes.iter().find(|s| s.id == p.id).context("shape")?;
        resized |= shape.vertices != p.arrays[es::POSITION].len() || shape.index_count != p.indices.len();
    }
    if resized {
        return Ok(changed + es::relayout(out, &models, &in_models)?);
    }
    for p in &in_models {
        let shape = models.shapes.iter().find(|s| s.id == p.id).context("shape")?;
        if let es::Outcome::Changed = es::write_shape(out, shape, p)? {
            changed += 1;
        }
    }
    Ok(changed)
}

pub const CHANGED_TEXTURES: &str = "effect with textures SWMM can't convert yet (see the log)";
pub const CHANGED_SHAPES: &str = "effect with particle shapes that don't fit the Switch game's shape models (see the log)";
pub const NEW_LAYOUT: &str = "effect with new kinds of emitters SWMM can't build for Switch yet";

pub fn to_switch(name: &str, raw: &[u8], game: &mut crate::stock::Game) -> Option<Result<Vec<u8>, &'static str>> {
    if raw.get(..4) != Some(b"EFTB") {
        return None;
    }
    let Some(ours) = tree(raw, true) else {
        return Some(Err("effect SWMM can't read"));
    };
    let sig = signature(raw, &ours);
    let (tex, shp) = (resource_crc(raw, ours.textures), resource_crc(raw, ours.shapes));
    let mut reason = NEW_LAYOUT;
    for twin in candidates(name, sig) {
        let Some(stock) = game.read(&twin.path) else { continue };
        let Some(theirs) = tree(&stock, false) else { continue };
        let (mut out, theirs, rebuilt) = if same_shape(raw, &ours, &stock, &theirs) {
            (stock, theirs, false)
        } else {
            let Some(built) = rebuild(raw, &stock) else { continue };
            let Some(t) = tree(&built, false) else { continue };
            if !shape_only(&ours, &t) {
                continue;
            }
            (built, t, true)
        };
        let changed = carry_over(raw, &ours, &mut out, &theirs);
        if twin.textures != tex {
            match replace_textures(raw, &ours, out) {
                Ok((built, n)) => {
                    log::info!("{name}: {n} of the mod's effect textures converted for Switch");
                    out = built;
                }
                Err(e) => {
                    log::warn!("{name}: {e:#}");
                    reason = CHANGED_TEXTURES;
                    continue;
                }
            }
        }
        if twin.shapes != shp {
            match replace_shapes(raw, &ours, &mut out) {
                Ok(n) => log::info!("{name}: {n} of the mod's particle shapes converted for Switch"),
                Err(e) => {
                    log::warn!("{name}: {e:#}");
                    reason = CHANGED_SHAPES;
                    continue;
                }
            }
        }
        if rebuilt {
            log::info!(
                "{name}: rebuilt from the Switch game's {} with the mod's emitters ({changed} settings carried over)",
                twin.path
            );
        } else {
            log::info!("{name}: built from the Switch game's {} with the mod's {changed} changed settings", twin.path);
        }
        return Some(Ok(out));
    }
    Some(Err(reason))
}
