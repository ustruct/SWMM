use anyhow_ext::{Context, Result, bail};

use super::{dict, format_info, tegra};
use crate::bfres::model::{UserData, UserValue};

#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub name: String,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub channels: [u8; 4],
    pub levels: Vec<Vec<u8>>,
    pub layers: u32,
    pub array: bool,
    pub user_data: Vec<UserData>,
}

const DIM_2D_ARRAY: u8 = 5;

fn layer_size(levels: &[tegra::Level], bh0: u32) -> u32 {
    let chain: u32 = levels.iter().map(|l| l.size).sum();
    chain.div_ceil(512 * bh0) * 512 * bh0
}

fn rd<const N: usize>(b: &[u8], o: usize) -> Result<[u8; N]> {
    b.get(o..o + N).and_then(|s| s.try_into().ok()).context("BNTX is cut short")
}
fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(rd(b, o)?))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(rd(b, o)?))
}
fn u64_at(b: &[u8], o: usize) -> Result<usize> {
    Ok(u64::from_le_bytes(rd(b, o)?) as usize)
}

fn string_at(b: &[u8], o: usize) -> Result<String> {
    let n = u16_at(b, o)? as usize;
    let s = b.get(o + 2..o + 2 + n).context("BNTX string is cut short")?;
    Ok(String::from_utf8_lossy(s).into_owned())
}

pub fn parse(b: &[u8]) -> Result<Vec<Texture>> {
    if b.get(..4) != Some(b"BNTX") || b.get(0x20..0x24) != Some(b"NX  ") {
        bail!("Not a Switch texture file (BNTX)");
    }
    let count = u32_at(b, 0x24)? as usize;
    let array = u64_at(b, 0x28)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let t = u64_at(b, array + 8 * i)?;
        if b.get(t..t + 4) != Some(b"BRTI") {
            bail!("BNTX texture {i} is not where its pointer says");
        }
        let mips = u16_at(b, t + 0x16)? as u32;
        let format = u32_at(b, t + 0x1c)?;
        let width = u32_at(b, t + 0x24)?;
        let height = u32_at(b, t + 0x28)?;
        let bh0 = 1 << u32_at(b, t + 0x34)?;
        let channels: [u8; 4] = rd(b, t + 0x58)?;
        let name = string_at(b, u64_at(b, t + 0x60)?)?;
        let mip_array = u64_at(b, t + 0x70)?;
        let layers = u32_at(b, t + 0x30)?.max(1);
        let array = b.get(t + 0x5c) == Some(&DIM_2D_ARRAY);
        let Some(info) = format_info(format) else { bail!("{name}: texture format {format:#x} isn't supported") };
        let chain = tegra::levels(width, height, mips, info.block, info.bytes, bh0);
        let stride = if array { layer_size(&chain, bh0) as usize } else { 0 };
        let mut levels = Vec::new();
        for (m, l) in chain.iter().enumerate() {
            let at = u64_at(b, mip_array + 8 * m)?;
            let mut level = Vec::new();
            for k in 0..layers as usize {
                let from = at + k * stride;
                let data = b.get(from..from + l.size as usize).context("BNTX texture data is cut short")?;
                level.extend(tegra::deswizzle(data, l, info.bytes));
            }
            levels.push(level);
        }
        let user_data = match u64_at(b, t + 0x98)? {
            0 => vec![],
            d => read_user_data(b, u64_at(b, t + 0x78)?, u32_at(b, d + 4)? as usize)?,
        };
        out.push(Texture { name, format, width, height, channels, levels, layers, array, user_data });
    }
    Ok(out)
}

fn read_user_data(b: &[u8], arr: usize, n: usize) -> Result<Vec<UserData>> {
    (0..n)
        .map(|i| {
            let e = arr + 0x40 * i;
            let name = string_at(b, u64_at(b, e)?)?;
            let data = u64_at(b, e + 8)?;
            let count = u32_at(b, e + 0x10)? as usize;
            let value = match b.get(e + 0x14).copied() {
                Some(0) => UserValue::Int((0..count).map(|k| Ok(u32_at(b, data + 4 * k)? as i32)).collect::<Result<_>>()?),
                Some(1) => UserValue::Float(
                    (0..count).map(|k| Ok(f32::from_bits(u32_at(b, data + 4 * k)?))).collect::<Result<_>>()?,
                ),
                Some(2) => UserValue::Str(
                    (0..count).map(|k| string_at(b, u64_at(b, data + 8 * k)?)).collect::<Result<_>>()?,
                    false,
                ),
                Some(4) => UserValue::Bytes(b.get(data..data + count).context("user data cut short")?.to_vec()),
                k => bail!("texture user data {name}: unknown type {k:?}"),
            };
            Ok(UserData { name, value })
        })
        .collect()
}

fn user_value_size(v: &UserValue) -> usize {
    let n = match v {
        UserValue::Int(x) => 4 * x.len(),
        UserValue::Float(x) => 4 * x.len(),
        UserValue::Str(x, _) => 8 * x.len(),
        UserValue::Bytes(x) => x.len(),
    };
    align(n, 8)
}

fn user_data_size(u: &[UserData]) -> usize {
    if u.is_empty() {
        return 0;
    }
    0x40 * u.len() + u.iter().map(|d| user_value_size(&d.value)).sum::<usize>() + 8 + 16 * (u.len() + 1)
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

struct Out {
    b: Vec<u8>,
    ptrs: Vec<(usize, bool)>,
}

impl Out {
    fn at(&mut self, o: usize) -> &mut [u8] {
        if self.b.len() < o + 8 {
            self.b.resize(o + 8, 0);
        }
        &mut self.b[o..]
    }
    fn u16(&mut self, o: usize, v: u16) {
        self.at(o)[..2].copy_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, o: usize, v: u32) {
        self.at(o)[..4].copy_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, o: usize, v: u64) {
        self.at(o)[..8].copy_from_slice(&v.to_le_bytes());
    }
    fn ptr(&mut self, o: usize, target: usize, data: bool) {
        self.u64(o, target as u64);
        self.ptrs.push((o, data));
    }
    fn bytes(&mut self, o: usize, v: &[u8]) {
        if self.b.len() < o + v.len() {
            self.b.resize(o + v.len(), 0);
        }
        self.b[o..o + v.len()].copy_from_slice(v);
    }
}

fn rlt_entries(positions: &[usize]) -> Vec<(u32, u16, u8, u8)> {
    crate::bfres::relocation_entries(positions)
}

pub fn write(name: &str, textures: &[Texture]) -> Result<Vec<u8>> {
    let n = textures.len();
    let mut o = Out { b: vec![0; 0x198], ptrs: Vec::new() };
    let array = 0x198;
    let str_at = array + 8 * n;
    let mut pos = str_at + 0x14;
    let mut name_pos = Vec::with_capacity(n);
    let put = |o: &mut Out, s: &str, pos: &mut usize| -> usize {
        let at = *pos;
        o.u16(at, s.len() as u16);
        o.bytes(at + 2, s.as_bytes());
        o.bytes(at + 2 + s.len(), &[0]);
        *pos = align(at + 3 + s.len(), 2);
        at
    };
    let empty = put(&mut o, "", &mut pos);
    for t in textures {
        name_pos.push(put(&mut o, &t.name, &mut pos));
    }
    let mut user_str: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for t in textures {
        for u in &t.user_data {
            let mut strings = vec![u.name.clone()];
            if let UserValue::Str(v, _) = &u.value {
                strings.extend(v.iter().cloned());
            }
            for s in strings {
                if let std::collections::hash_map::Entry::Vacant(e) = user_str.entry(s) {
                    let at = put(&mut o, e.key(), &mut pos);
                    e.insert(at);
                }
            }
        }
    }
    let same = textures.iter().position(|t| t.name == name);
    let file_name = match same {
        Some(i) => name_pos[i],
        None => put(&mut o, name, &mut pos),
    };
    let dic = align(pos, 8);
    o.bytes(dic, b"_DIC");
    o.u32(dic + 4, n as u32);
    let keys: Vec<&str> = textures.iter().map(|t| t.name.as_str()).collect();
    let nodes = dict::build(&keys);
    for (k, node) in nodes.iter().enumerate() {
        let e = dic + 8 + 16 * k;
        o.u32(e, node.bit as u32);
        o.u16(e + 4, node.left);
        o.u16(e + 6, node.right);
        o.ptr(e + 8, if k == 0 { empty } else { name_pos[k - 1] }, false);
    }
    let mut brti = align(dic + 8 + 16 * (n + 1), 8);
    let first_brti = brti;
    let mut brtis = Vec::with_capacity(n);
    for t in textures {
        let info = format_info(t.format).with_context(|| format!("texture format {:#x}", t.format))?;
        if t.levels.is_empty() {
            bail!("{} has no image data", t.name);
        }
        let size = 0xa0 + 0x200 + 8 * t.levels.len() + user_data_size(&t.user_data);
        brtis.push(brti);
        brti += size;
        let _ = info;
    }
    let end_brti = brti;
    let brtd = align(end_brti + 0x10, 0x1000) - 0x10;
    let data_start = brtd + 0x10;
    let mut data_at = data_start;
    let mut layouts = Vec::with_capacity(n);
    for t in textures {
        let info = format_info(t.format).context("format")?;
        let block_w = info.block;
        let bh0 = tegra::block_height(t.height.div_ceil(block_w));
        let levels = tegra::levels(t.width, t.height, t.levels.len() as u32, block_w, info.bytes, bh0);
        data_at = align(data_at, 0x200);
        let layers = t.layers.max(1);
        let stride = if t.array { layer_size(&levels, bh0) } else { levels.iter().map(|l| l.size).sum() };
        let total = stride * layers;
        let mut buf = vec![0u8; total as usize];
        for (l, linear) in levels.iter().zip(&t.levels) {
            let need = (l.width_blocks * l.height_blocks * info.bytes) as usize;
            if linear.len() < need * layers as usize {
                bail!("{}: a mip level has {} bytes, {} expected", t.name, linear.len(), need * layers as usize);
            }
            for k in 0..layers as usize {
                let at = k * stride as usize + l.offset as usize;
                tegra::swizzle(&linear[k * need..(k + 1) * need], l, info.bytes, &mut buf[at..at + l.size as usize]);
            }
        }
        o.bytes(data_at, &buf);
        layouts.push((data_at, bh0, levels, total));
        data_at += total as usize;
    }
    let data_end = align(data_at, 0x1000);
    if o.b.len() < data_end {
        o.b.resize(data_end, 0);
    }
    for (i, t) in textures.iter().enumerate() {
        let b = brtis[i];
        let (at, bh0, levels, total) = &layouts[i];
        let size = 0xa0 + 0x200 + 8 * t.levels.len() + user_data_size(&t.user_data);
        let next = if i + 1 < n { size } else { brtd - b };
        o.bytes(b, b"BRTI");
        o.u32(b + 4, next as u32);
        o.u64(b + 8, next as u64);
        o.bytes(b + 0x10, &[1, 2]);
        o.u16(b + 0x12, 0);
        o.u16(b + 0x14, 0);
        o.u16(b + 0x16, t.levels.len() as u16);
        o.u32(b + 0x18, 1);
        o.u32(b + 0x1c, t.format);
        o.u32(b + 0x20, 0x20);
        o.u32(b + 0x24, t.width);
        o.u32(b + 0x28, t.height);
        o.u32(b + 0x2c, 1);
        o.u32(b + 0x30, t.layers.max(1));
        o.u32(b + 0x34, bh0.trailing_zeros());
        o.u32(b + 0x38, 0x10007);
        o.u32(b + 0x50, *total);
        o.u32(b + 0x54, 0x200);
        o.bytes(b + 0x58, &t.channels);
        o.u32(b + 0x5c, if t.array { DIM_2D_ARRAY as u32 } else { 1 });
        o.ptr(b + 0x60, name_pos[i], false);
        o.ptr(b + 0x68, 0x20, false);
        o.ptr(b + 0x70, b + 0x2a0, false);
        o.ptr(b + 0x80, b + 0xa0, false);
        o.ptr(b + 0x88, b + 0x1a0, false);
        for (m, l) in levels.iter().enumerate() {
            o.ptr(b + 0x2a0 + 8 * m, at + l.offset as usize, true);
        }
        if !t.user_data.is_empty() {
            let arr = b + 0x2a0 + 8 * t.levels.len();
            let mut v = arr + 0x40 * t.user_data.len();
            for (k, u) in t.user_data.iter().enumerate() {
                let e = arr + 0x40 * k;
                o.ptr(e, user_str[&u.name], false);
                o.ptr(e + 8, v, false);
                let (count, kind) = match &u.value {
                    UserValue::Int(x) => {
                        for (j, n) in x.iter().enumerate() {
                            o.u32(v + 4 * j, *n as u32);
                        }
                        (x.len(), 0u8)
                    }
                    UserValue::Float(x) => {
                        for (j, n) in x.iter().enumerate() {
                            o.u32(v + 4 * j, n.to_bits());
                        }
                        (x.len(), 1)
                    }
                    UserValue::Str(x, _) => {
                        for (j, s) in x.iter().enumerate() {
                            o.ptr(v + 8 * j, user_str[s], false);
                        }
                        (x.len(), 2)
                    }
                    UserValue::Bytes(x) => {
                        o.bytes(v, x);
                        (x.len(), 4)
                    }
                };
                o.u32(e + 0x10, count as u32);
                o.bytes(e + 0x14, &[kind]);
                v += user_value_size(&u.value);
            }
            let keys: Vec<&str> = t.user_data.iter().map(|u| u.name.as_str()).collect();
            let nodes = dict::build(&keys);
            o.bytes(v, b"_DIC");
            o.u32(v + 4, keys.len() as u32);
            for (k, node) in nodes.iter().enumerate() {
                let e = v + 8 + 16 * k;
                o.u32(e, node.bit as u32);
                o.u16(e + 4, node.left);
                o.u16(e + 6, node.right);
                o.ptr(e + 8, if k == 0 { empty } else { user_str[keys[k - 1]] }, false);
            }
            o.ptr(b + 0x78, arr, false);
            o.ptr(b + 0x98, v, false);
        }
    }
    for (i, b) in brtis.iter().enumerate() {
        o.ptr(array + 8 * i, *b, false);
    }
    o.bytes(str_at, b"_STR");
    o.u32(str_at + 4, (first_brti - str_at) as u32);
    o.u64(str_at + 8, (first_brti - str_at) as u64);
    o.u32(str_at + 0x10, n as u32 + u32::from(same.is_none()));
    o.bytes(0x20, b"NX  ");
    o.u32(0x24, n as u32);
    o.ptr(0x28, array, false);
    o.ptr(0x30, brtd, true);
    o.ptr(0x38, dic, false);
    o.ptr(0x40, 0x58, false);
    o.bytes(brtd, b"BRTD");
    o.u64(brtd + 8, (data_end - brtd) as u64);
    let rlt = data_end;
    let mut head: Vec<usize> = o.ptrs.iter().filter(|p| !p.1).map(|p| p.0).collect();
    let mut data: Vec<usize> = o.ptrs.iter().filter(|p| p.1).map(|p| p.0).collect();
    head.sort_unstable();
    data.sort_unstable();
    let (eh, ed) = (rlt_entries(&head), rlt_entries(&data));
    let mut r = Vec::new();
    r.extend_from_slice(b"_RLT");
    r.extend_from_slice(&(rlt as u32).to_le_bytes());
    r.extend_from_slice(&2u32.to_le_bytes());
    r.extend_from_slice(&0u32.to_le_bytes());
    for (pos, size, index, count) in
        [(0, end_brti, 0, eh.len()), (brtd, data_end - brtd, eh.len(), ed.len())]
    {
        r.extend_from_slice(&0u64.to_le_bytes());
        r.extend_from_slice(&(pos as u32).to_le_bytes());
        r.extend_from_slice(&(size as u32).to_le_bytes());
        r.extend_from_slice(&(index as u32).to_le_bytes());
        r.extend_from_slice(&(count as u32).to_le_bytes());
    }
    for (pos, s, k, p) in eh.iter().chain(&ed) {
        r.extend_from_slice(&pos.to_le_bytes());
        r.extend_from_slice(&s.to_le_bytes());
        r.extend_from_slice(&[*k, *p]);
    }
    o.b.truncate(rlt);
    o.b.extend_from_slice(&r);
    let file_size = o.b.len();
    o.bytes(0, b"BNTX\0\0\0\0");
    o.u32(8, 0x0004_0000);
    o.bytes(0xc, &[0xff, 0xfe, 0x0c, 0x40]);
    o.u32(0x10, (file_name + 2) as u32);
    o.u16(0x14, 0);
    o.u16(0x16, str_at as u16);
    o.u32(0x18, rlt as u32);
    o.u32(0x1c, file_size as u32);
    Ok(o.b)
}

pub fn relocations(b: &[u8]) -> Result<Vec<(usize, usize)>> {
    let rlt = u32_at(b, 0x18)? as usize;
    if b.get(rlt..rlt + 4) != Some(b"_RLT") {
        bail!("No relocation table");
    }
    let sections = u32_at(b, rlt + 8)? as usize;
    let entries = rlt + 0x10 + 0x18 * sections;
    let mut out = Vec::new();
    for s in 0..sections {
        let sec = rlt + 0x10 + 0x18 * s;
        let first = u32_at(b, sec + 0x10)? as usize;
        let count = u32_at(b, sec + 0x14)? as usize;
        for e in first..first + count {
            let at = entries + 8 * e;
            let pos = u32_at(b, at)? as usize;
            let structs = u16_at(b, at + 4)? as usize;
            let offsets = b[at + 6] as usize;
            let padding = b[at + 7] as usize;
            for st in 0..structs {
                for k in 0..offsets {
                    out.push((s, pos + st * 8 * (offsets + padding) + 8 * k));
                }
            }
        }
    }
    out.sort_unstable();
    Ok(out)
}
