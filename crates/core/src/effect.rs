use std::collections::HashMap;
use std::sync::OnceLock;

static TABLE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/wiiu/effects.txt"));

pub struct Emitter {
    pub name: Vec<u8>,
    pub data: usize,
    pub size: usize,
}

fn word(d: &[u8], o: usize, be: bool) -> Option<usize> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    Some(if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) } as usize)
}

pub fn emitters(d: &[u8], be: bool) -> Option<Vec<Emitter>> {
    let start = d.get(..0x100)?.windows(4).position(|w| w == b"ESTA")?;
    let mut out = Vec::new();
    let mut stack = vec![start];
    let mut seen = 0;
    while let Some(at) = stack.pop() {
        seen += 1;
        if seen > 100_000 {
            return None;
        }
        let magic = d.get(at..at + 4)?;
        let size = word(d, at + 4, be)?;
        let child = word(d, at + 8, be)?;
        let next = word(d, at + 0xc, be)?;
        if at + size > d.len() {
            return None;
        }
        if magic == b"EMTR" {
            let (attr, data) = (word(d, at + 0x10, be)?, word(d, at + 0x14, be)?);
            let size = attr.min(size).checked_sub(data)?;
            let name = d.get(at + data + 0x10..at + data + 0x50)?;
            let name = name.split(|b| *b == 0).next().unwrap_or_default().to_vec();
            out.push(Emitter { name, data: at + data, size });
        }
        if next != 0xffff_ffff && next != 0 {
            stack.push(at + next);
        }
        if child != 0xffff_ffff && child != 0 && matches!(magic, b"ESTA" | b"ESET" | b"EMTR") {
            stack.push(at + child);
        }
    }
    Some(out)
}

pub struct Table {
    pub words: Vec<usize>,
    pub twins: HashMap<u32, String>,
}

pub fn parse_table(text: &str) -> Table {
    let mut lines = text.lines();
    let words = lines
        .next()
        .and_then(|l| l.strip_prefix("words "))
        .map(|l| {
            l.split(' ')
                .filter_map(|r| {
                    let (a, b) = r.split_once('-').unwrap_or((r, r));
                    Some(a.parse::<usize>().ok()?..=b.parse::<usize>().ok()?)
                })
                .flatten()
                .collect()
        })
        .unwrap_or_default();
    let twins = lines
        .filter_map(|l| {
            let (crc, path) = l.split_once(' ')?;
            Some((u32::from_str_radix(crc, 16).ok()?, path.to_string()))
        })
        .collect();
    Table { words, twins }
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| parse_table(TABLE))
}

pub fn skeleton(d: &[u8], list: &[Emitter], words: &[usize]) -> u32 {
    let mut b = d.to_vec();
    for e in list {
        for &w in words {
            if 4 * w + 4 <= e.size {
                b[e.data + 4 * w..e.data + 4 * w + 4].fill(0);
            }
        }
    }
    crate::stock::crc32(&b)
}

pub fn to_switch(name: &str, raw: &[u8], game: &mut crate::stock::Game) -> Option<Vec<u8>> {
    if raw.get(..4) != Some(b"EFTB") {
        return None;
    }
    let t = table();
    let ours = emitters(raw, true)?;
    let paths = t.twins.get(&skeleton(raw, &ours, &t.words))?;
    let file = name.rsplit("//").next().unwrap_or(name);
    let file = file.rsplit('/').next().unwrap_or(file);
    let path = paths.split('|').find(|p| p.rsplit('/').next() == Some(file)).or_else(|| (!paths.contains('|')).then_some(paths.as_str()))?;
    let mut out = game.read(path)?;
    let theirs = emitters(&out, false)?;
    if ours.len() != theirs.len() || ours.iter().zip(&theirs).any(|(a, b)| a.name != b.name) {
        return None;
    }
    let mut changed = 0;
    for (a, b) in ours.iter().zip(&theirs) {
        for &w in &t.words {
            if 4 * w + 4 > a.size || 4 * w + 4 > b.size {
                continue;
            }
            let mut v: [u8; 4] = raw[a.data + 4 * w..a.data + 4 * w + 4].try_into().ok()?;
            v.reverse();
            let slot = &mut out[b.data + 4 * w..b.data + 4 * w + 4];
            if slot != v {
                slot.copy_from_slice(&v);
                changed += 1;
            }
        }
    }
    log::info!("{name}: the mod's {changed} emitter settings carried over to the Switch effect");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_parses() {
        let t = parse_table("words 13-15 20\n0000abcd Effect/A.sesetlist\n");
        assert_eq!(t.words, vec![13, 14, 15, 20]);
        assert_eq!(t.twins.get(&0xabcd).map(String::as_str), Some("Effect/A.sesetlist"));
        assert!(!table().words.is_empty());
    }
}
