use anyhow_ext::{Context, Result, bail};

const NONE: u32 = 0xffff_ffff;

fn be32(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("effect shape data is cut short")?.try_into()?))
}
fn le16(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(d.get(o..o + 2).context("effect shape model is cut short")?.try_into()?))
}
fn le32(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).context("effect shape model is cut short")?.try_into()?))
}
fn le64(d: &[u8], o: usize) -> Result<usize> {
    Ok(u64::from_le_bytes(d.get(o..o + 8).context("effect shape model is cut short")?.try_into()?) as usize)
}

pub const POSITION: usize = 0;
pub const NORMAL: usize = 1;
pub const TANGENT: usize = 2;
const NEUTRAL: [[f32; 4]; 6] =
    [[0.0; 4], [0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 0.0, 1.0], [1.0; 4], [0.0; 4], [0.0; 4]];

#[derive(Clone)]
pub struct Prim {
    pub id: u64,
    pub arrays: [Vec<[f32; 4]>; 6],
    pub indices: Vec<u32>,
    pub components: [u32; 6],
    pub tail: u32,
}

fn read_prims(d: &[u8], prma: usize, be: bool) -> Result<Vec<(usize, Prim)>> {
    if d.get(prma..prma + 4) != Some(b"PRMA") {
        bail!("not an effect shape list");
    }
    let w = |o: usize| if be { be32(d, o) } else { le32(d, o) };
    let f = |c: &[u8]| {
        let b: [u8; 4] = c.try_into().unwrap();
        if be { f32::from_be_bytes(b) } else { f32::from_le_bytes(b) }
    };
    let mut out = vec![];
    let child = w(prma + 8)?;
    if child == NONE || child == 0 || w(prma + 4)? == 0 {
        return Ok(out);
    }
    let mut at = prma + child as usize;
    loop {
        if d.get(at..at + 4) != Some(b"PRIM") {
            bail!("an effect shape isn't where its list says");
        }
        let base = at + w(at + 0x14)? as usize;
        let v: Vec<u32> = (0..22).map(|i| w(base + 4 * i)).collect::<Result<_>>()?;
        let id = if be { v[1] as u64 | (v[0] as u64) << 32 } else { v[0] as u64 | (v[1] as u64) << 32 };
        let mut arrays: [Vec<[f32; 4]>; 6] = Default::default();
        for (k, a) in arrays.iter_mut().take(5).enumerate() {
            let count = v[2 + 2 * k] as usize;
            if count == 0 {
                continue;
            }
            let start = base + v[15 + k] as usize;
            let raw = d.get(start..start + 16 * count).context("effect shape vertices are cut short")?;
            *a = raw.chunks(16).map(|c| std::array::from_fn(|j| f(&c[4 * j..4 * j + 4]))).collect();
        }
        if v[12] > 0 {
            arrays[5] = arrays[4].iter().take(v[12] as usize).map(|a| [a[2], a[3], 0.0, 0.0]).collect();
        }
        let n = v[14] as usize;
        let start = base + v[20] as usize;
        let raw = d.get(start..start + 4 * n).context("effect shape indices are cut short")?;
        let indices = raw
            .chunks(4)
            .map(|c| {
                let b: [u8; 4] = c.try_into().unwrap();
                if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
            })
            .collect();
        let components = std::array::from_fn(|k| v[3 + 2 * k]);
        out.push((at, Prim { id, arrays, indices, components, tail: v[21] }));
        let next = w(at + 0xc)?;
        if next == NONE || next == 0 {
            break;
        }
        at += next as usize;
    }
    Ok(out)
}

pub fn wiiu_prims(d: &[u8], prma: usize) -> Result<Vec<Prim>> {
    Ok(read_prims(d, prma, true)?.into_iter().map(|(_, p)| p).collect())
}

pub fn switch_prims(d: &[u8], prma: usize) -> Result<Vec<Prim>> {
    Ok(read_prims(d, prma, false)?.into_iter().map(|(_, p)| p).collect())
}

fn raw_entry(p: &Prim, at: usize) -> Vec<u8> {
    let base = at + 0x20;
    let mut desc = vec![p.id as u32, (p.id >> 32) as u32];
    for k in 0..6 {
        let n = p.arrays[k].len() as u32;
        desc.extend([n, if n > 0 { p.components[k] } else { 0 }]);
    }
    desc.push(p.indices.len() as u32);
    let mut cur = base + 0x58;
    let mut chunks: Vec<(usize, Vec<u8>)> = vec![];
    let mut place = |bytes: Vec<u8>, cur: &mut usize| -> u32 {
        if bytes.is_empty() {
            return 0;
        }
        let p = cur.div_ceil(0x40) * 0x40;
        *cur = p + bytes.len();
        chunks.push((p, bytes));
        (p - base) as u32
    };
    for k in 0..5 {
        let bytes: Vec<u8> = p.arrays[k].iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
        desc.push(place(bytes, &mut cur));
    }
    let bytes: Vec<u8> = p.indices.iter().flat_map(|x| x.to_le_bytes()).collect();
    let index_at = cur.div_ceil(0x40) * 0x40;
    if bytes.is_empty() {
        cur = index_at;
    }
    desc.push(if bytes.is_empty() { (index_at - base) as u32 } else { place(bytes, &mut cur) });
    desc.push(p.tail);
    let size = cur - base;
    let mut out = vec![0xccu8; 0x20 + size];
    out[..0x20].fill(0);
    out[0..4].copy_from_slice(b"PRIM");
    for (i, v) in [size as u32, NONE, NONE, NONE, 0x20].iter().enumerate() {
        out[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in desc.iter().enumerate() {
        out[0x20 + 4 * i..0x24 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    for (p, b) in chunks {
        out[p - at..p - at + b.len()].copy_from_slice(&b);
    }
    out
}

pub fn raw_list(prims: &[&Prim], at: usize) -> Vec<u8> {
    let mut out = vec![0u8; 0x20];
    let mut last: Option<usize> = None;
    for p in prims {
        let start = out.len().div_ceil(8) * 8;
        out.resize(start, 0);
        if let Some(l) = last {
            out[l + 0xc..l + 0x10].copy_from_slice(&((start - l) as u32).to_le_bytes());
        }
        out.extend(raw_entry(p, at + start));
        last = Some(start);
    }
    out[0..4].copy_from_slice(b"PRMA");
    let empty = prims.is_empty();
    let size = if empty { 0 } else { out.len() - 0x20 };
    let child = if empty { NONE } else { 0x20 };
    let data = if empty { NONE } else { 0x20 };
    for (i, v) in [size as u32, child, 0, NONE, data, 0, prims.len() as u32].iter().enumerate() {
        out[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    out
}

pub struct Attribute {
    pub name: String,
    pub format: u32,
    pub offset: usize,
    pub buffer: usize,
}

pub struct Shape {
    pub id: u64,
    pub slots: [u8; 8],
    pub vertices: usize,
    pub attributes: Vec<Attribute>,
    pub buffers: Vec<(usize, usize, usize)>,
    pub index_at: usize,
    pub index_format: u32,
    pub index_count: usize,
    pub bounds_at: usize,
    pub radius_at: usize,
    pub sub_meshes: usize,
    pub fvtx: usize,
    pub mesh: usize,
    pub fmdl: usize,
    pub sizes: usize,
    pub parts_at: usize,
    pub name_at: usize,
}

pub struct Models {
    pub g3pr: usize,
    pub bf: usize,
    pub info: usize,
    pub gpu: usize,
    pub region: usize,
    pub rlt: usize,
    pub shapes: Vec<Shape>,
}

fn names(d: &[u8], g3pr: usize) -> Result<Vec<(u64, [u8; 8], usize)>> {
    let nt = g3pr + le32(d, g3pr + 8)? as usize;
    if d.get(nt..nt + 4) != Some(b"G3NT") {
        bail!("the Switch shape list has no name table");
    }
    let len = le32(d, nt + 4)? as usize;
    let mut k = nt + le32(d, nt + 0x14)? as usize;
    let end = k + len;
    let mut out = vec![];
    while k + 24 <= end {
        let id = le64(d, k)? as u64;
        let step = le32(d, k + 8)? as usize;
        let mut slots = [0xffu8; 8];
        let n = (le32(d, k + 12)? as usize).min(8);
        slots[..n].copy_from_slice(&d[k + 16..k + 16 + n]);
        out.push((id, slots, k));
        if step == 0 {
            break;
        }
        k += step;
    }
    Ok(out)
}

fn find_all(d: &[u8], from: usize, to: usize, magic: &[u8; 4]) -> Vec<usize> {
    d.get(from..to)
        .map(|s| s.windows(4).enumerate().filter(|(i, w)| i % 8 == 0 && w == magic).map(|(i, _)| from + i).collect())
        .unwrap_or_default()
}

fn string(d: &[u8], at: usize) -> Result<String> {
    let n = le16(d, at)? as usize;
    Ok(String::from_utf8_lossy(d.get(at + 2..at + 2 + n).context("cut short")?).into_owned())
}

pub fn switch_models(d: &[u8], g3pr: usize) -> Result<Option<Models>> {
    if d.get(g3pr..g3pr + 4) != Some(b"G3PR") || le32(d, g3pr + 4)? == 0 {
        return Ok(None);
    }
    let ids = names(d, g3pr)?;
    let bf = g3pr + le32(d, g3pr + 0x14)? as usize;
    if d.get(bf..bf + 4) != Some(b"FRES") {
        bail!("the Switch shape list has no model file");
    }
    let info = bf + le64(d, bf + 0x90)?;
    let gpu = bf + le64(d, info + 8)?;
    let region = le32(d, info + 4)? as usize;
    let rlt = bf + le32(d, bf + 0x18)? as usize;
    let models = le16(d, bf + 0xbc)? as usize;
    let model_array = bf + le64(d, bf + 0x28)?;
    let fvtx = find_all(d, bf, gpu, b"FVTX");
    let fshp = find_all(d, bf, gpu, b"FSHP");
    if fvtx.len() != models || fshp.len() != models || ids.len() != models {
        bail!("the Switch shape models don't line up with their names");
    }
    let mut out = vec![];
    for (i, (id, slots, name_at)) in ids.into_iter().enumerate() {
        let o = fvtx[i];
        let na = d[o + 0x54] as usize;
        let nb = d[o + 0x55] as usize;
        let vertices = le32(d, o + 0x58)? as usize;
        let attrs = bf + le64(d, o + 0x10)?;
        let mut attributes = vec![];
        for k in 0..na {
            let p = attrs + 0x10 * k;
            attributes.push(Attribute {
                name: string(d, bf + le64(d, p)?)?,
                format: le32(d, p + 8)?,
                offset: le16(d, p + 0xc)? as usize,
                buffer: le16(d, p + 0xe)? as usize,
            });
        }
        let sizes = bf + le64(d, o + 0x38)?;
        let strides = bf + le64(d, o + 0x40)?;
        let mut at = gpu + le32(d, o + 0x50)? as usize;
        let mut buffers = vec![];
        for k in 0..nb {
            at = at.div_ceil(8) * 8;
            let size = le32(d, sizes + 0x10 * k)? as usize;
            buffers.push((at, size, le32(d, strides + 0x10 * k)? as usize));
            at += size;
        }
        let s = fshp[i];
        if d[s + 0x67] != 1 {
            bail!("a Switch shape model with more than one mesh");
        }
        let mesh = bf + le64(d, s + 0x20)?;
        let sub_meshes = le16(d, mesh + 0x34)? as usize;
        out.push(Shape {
            id,
            slots,
            vertices,
            attributes,
            buffers,
            index_at: gpu + le32(d, mesh + 0x20)? as usize,
            index_format: le32(d, mesh + 0x28)?,
            index_count: le32(d, mesh + 0x2c)? as usize,
            bounds_at: bf + le64(d, s + 0x40)?,
            radius_at: bf + le64(d, s + 0x48)?,
            sub_meshes,
            fvtx: o,
            mesh,
            fmdl: model_array + 0x78 * i,
            sizes,
            parts_at: bf + le64(d, mesh)?,
            name_at,
        });
    }
    if out.iter().any(|s| d.get(s.fmdl..s.fmdl + 4) != Some(b"FMDL")) {
        bail!("the Switch shape models aren't where their list says");
    }
    Ok(Some(Models { g3pr, bf, info, gpu, region, rlt, shapes: out }))
}

pub fn half(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | if man != 0 { 0x7e00 } else { 0x7c00 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7bff;
    }
    if e <= 0 {
        return sign;
    }
    sign | ((e as u16) << 10) | (man >> 13) as u16
}

fn unorm(x: f32, max: f32) -> u32 {
    (x.clamp(0.0, 1.0) * max).round() as u32
}

fn snorm(x: f32, max: f32, bits: u32) -> u32 {
    let v = (x.clamp(-1.0, 1.0) * max).round() as i32;
    (v as u32) & ((1 << bits) - 1)
}

pub fn encode(format: u32, v: [f32; 4], out: &mut [u8]) -> Result<()> {
    let put = |out: &mut [u8], i: usize, b: &[u8]| out[i..i + b.len()].copy_from_slice(b);
    match format {
        0x1505 => (0..4).for_each(|i| put(out, 2 * i, &half(v[i]).to_le_bytes())),
        0x1205 => (0..2).for_each(|i| put(out, 2 * i, &half(v[i]).to_le_bytes())),
        0x1805 => (0..3).for_each(|i| put(out, 4 * i, &v[i].to_le_bytes())),
        0x1705 => (0..2).for_each(|i| put(out, 4 * i, &v[i].to_le_bytes())),
        0x1905 => (0..4).for_each(|i| put(out, 4 * i, &v[i].to_le_bytes())),
        0x0e02 => {
            let w = snorm(v[0], 511.0, 10) | (snorm(v[1], 511.0, 10) << 10) | (snorm(v[2], 511.0, 10) << 20) | (1 << 30);
            put(out, 0, &w.to_le_bytes());
        }
        0x0901 => (0..2).for_each(|i| out[i] = unorm(v[i], 255.0) as u8),
        0x0902 => (0..2).for_each(|i| out[i] = snorm(v[i], 127.0, 8) as u8),
        0x0b01 => (0..4).for_each(|i| out[i] = unorm(v[i], 255.0) as u8),
        0x0b02 => (0..4).for_each(|i| out[i] = snorm(v[i], 127.0, 8) as u8),
        0x1201 => (0..2).for_each(|i| put(out, 2 * i, &(unorm(v[i], 65535.0) as u16).to_le_bytes())),
        0x1202 => (0..2).for_each(|i| put(out, 2 * i, &(snorm(v[i], 32767.0, 16) as u16).to_le_bytes())),
        f => bail!("vertex format {f:#x} isn't supported for effect shapes"),
    }
    Ok(())
}

pub fn format_size(format: u32) -> usize {
    match format {
        0x1505 | 0x1705 => 8,
        0x1205 | 0x0e02 | 0x0b01 | 0x0b02 | 0x1201 | 0x1202 => 4,
        0x0901 | 0x0902 => 2,
        0x1805 => 12,
        0x1905 => 16,
        _ => 0,
    }
}

pub fn binormal(n: [f32; 4], t: [f32; 4]) -> [f32; 4] {
    let c = [n[1] * t[2] - n[2] * t[1], n[2] * t[0] - n[0] * t[2], n[0] * t[1] - n[1] * t[0]];
    [c[0] * t[3], c[1] * t[3], c[2] * t[3], t[3]]
}

pub enum Outcome {
    Same,
    Changed,
}

fn vertex_sources(shape: &Shape, prim: &Prim) -> Result<Vec<(usize, Vec<[f32; 4]>)>> {
    let mut out = vec![];
    for array in 0..6 {
        let wanted = ["_p0", "_n0", "_t0", "_c0", "_u0", "_u1"][array];
        let found = shape.attributes.iter().position(|a| a.name == wanted);
        match (found, prim.arrays[array].is_empty()) {
            (None, true) => continue,
            (None, false) => {
                log::info!("the mod's shape {:#x} has vertex data ({wanted}) the game's model has no room for, so it's left out", prim.id);
                continue;
            }
            (Some(_), true) if array == POSITION => bail!("the mod's shape {:#x} has no points", prim.id),
            (Some(i), true) => {
                log::info!("the mod's shape {:#x} has no {wanted} vertex data, so neutral values fill it", prim.id);
                out.push((i, vec![NEUTRAL[array]; prim.arrays[POSITION].len()]));
            }
            (Some(i), false) => {
                if prim.arrays[array].len() != prim.arrays[POSITION].len() {
                    bail!("the mod's shape {:#x} has vertex data of different lengths", prim.id);
                }
                out.push((i, prim.arrays[array].clone()));
            }
        }
    }
    if let Some(i) = shape.attributes.iter().position(|a| a.name == "_b0") {
        let n = prim.arrays[POSITION].len();
        let pick = |k: usize, j: usize| prim.arrays[k].get(j).copied().unwrap_or(NEUTRAL[k]);
        out.push((i, (0..n).map(|j| binormal(pick(NORMAL, j), pick(TANGENT, j))).collect()));
    }
    Ok(out)
}

fn fresh_buffers(shape: &Shape, prim: &Prim, existing: Option<Vec<Vec<u8>>>) -> Result<Vec<Vec<u8>>> {
    let n = prim.arrays[POSITION].len();
    let sources = vertex_sources(shape, prim)?;
    let mut out: Vec<Vec<u8>> = match existing {
        Some(e) if n == shape.vertices => e,
        _ => {
            if sources.len() != shape.attributes.len() {
                bail!("the game's shape model has vertex data SWMM can't fill from the mod's shape {:#x}", prim.id);
            }
            shape.buffers.iter().map(|(_, _, stride)| vec![0; stride * n]).collect()
        }
    };
    for (i, values) in sources {
        let a = &shape.attributes[i];
        let stride = shape.buffers.get(a.buffer).context("an attribute points at a missing buffer")?.2;
        let size = format_size(a.format);
        if size == 0 || a.offset + size > stride {
            bail!("vertex attribute {} doesn't fit its buffer", a.name);
        }
        for (k, v) in values.iter().enumerate() {
            let p = stride * k + a.offset;
            encode(a.format, *v, &mut out[a.buffer][p..p + size])?;
        }
    }
    Ok(out)
}

fn index_bytes(shape: &Shape, prim: &Prim) -> Result<Vec<u8>> {
    let mut out = vec![];
    for &i in &prim.indices {
        match shape.index_format {
            1 if i <= 0xffff => out.extend((i as u16).to_le_bytes()),
            1 => bail!("the mod's shape {:#x} has too many points for the game's model", prim.id),
            2 => out.extend(i.to_le_bytes()),
            f => bail!("index format {f} isn't supported for effect shapes"),
        }
    }
    Ok(out)
}

fn index_width(shape: &Shape) -> usize {
    if shape.index_format == 1 { 2 } else { 4 }
}

fn parts(d: &[u8], shape: &Shape) -> Result<Vec<(usize, usize)>> {
    let w = index_width(shape);
    (0..shape.sub_meshes)
        .map(|i| Ok((le32(d, shape.parts_at + 8 * i)? as usize / w, le32(d, shape.parts_at + 8 * i + 4)? as usize)))
        .collect()
}

fn split(count: usize, n: usize) -> Vec<(usize, usize)> {
    let triangles = count / 3;
    (0..n)
        .map(|i| {
            let start = 3 * (triangles * i / n);
            let end = if i + 1 == n { count } else { 3 * (triangles * (i + 1) / n) };
            (start, end - start)
        })
        .collect()
}

fn bbox(pos: &[[f32; 4]], picks: impl Iterator<Item = usize>) -> Option<[f32; 6]> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;
    for i in picks {
        let Some(p) = pos.get(i) else { continue };
        any = true;
        for j in 0..3 {
            lo[j] = lo[j].min(p[j]);
            hi[j] = hi[j].max(p[j]);
        }
    }
    any.then(|| std::array::from_fn(|j| if j < 3 { (lo[j] + hi[j]) * 0.5 } else { (hi[j - 3] - lo[j - 3]) * 0.5 }))
}

fn bounds(prim: &Prim, parts: &[(usize, usize)]) -> (Vec<u8>, [u8; 4]) {
    let pos = &prim.arrays[POSITION];
    let whole = bbox(pos, 0..pos.len()).unwrap_or([0.0; 6]);
    let mut b = vec![];
    for &(start, count) in parts {
        let part = bbox(pos, prim.indices.iter().skip(start).take(count).map(|&i| i as usize)).unwrap_or(whole);
        b.extend(part.iter().flat_map(|v| v.to_le_bytes()));
    }
    b.extend(whole.iter().flat_map(|v| v.to_le_bytes()));
    let radius = pos
        .iter()
        .map(|p| ((p[0] - whole[0]).powi(2) + (p[1] - whole[1]).powi(2) + (p[2] - whole[2]).powi(2)).sqrt())
        .fold(0f32, f32::max);
    (b, radius.to_le_bytes())
}

fn put_changed(d: &mut [u8], at: usize, b: &[u8], changed: &mut bool) -> Result<()> {
    let slot = d.get_mut(at..at + b.len()).context("effect shape model is cut short")?;
    if slot != b {
        slot.copy_from_slice(b);
        *changed = true;
    }
    Ok(())
}

pub fn can_hold(shape: &Shape, prim: &Prim) -> bool {
    vertex_sources(shape, prim).is_ok_and(|s| s.len() == shape.attributes.len()) && index_bytes(shape, prim).is_ok()
}

pub fn rename(d: &mut [u8], shape: &Shape, id: u64) -> Result<()> {
    let slot = d.get_mut(shape.name_at..shape.name_at + 8).context("the Switch shape name table is cut short")?;
    slot.copy_from_slice(&id.to_le_bytes());
    Ok(())
}

pub fn write_shape(d: &mut [u8], shape: &Shape, prim: &Prim) -> Result<Outcome> {
    if shape.vertices != prim.arrays[POSITION].len() || shape.index_count != prim.indices.len() {
        bail!("the mod's shape {:#x} has a different number of points than the game's", prim.id);
    }
    let mut changed = false;
    let existing: Vec<Vec<u8>> = shape.buffers.iter().map(|&(at, size, _)| d[at..at + size].to_vec()).collect();
    for (k, b) in fresh_buffers(shape, prim, Some(existing))?.iter().enumerate() {
        put_changed(d, shape.buffers[k].0, b, &mut changed)?;
    }
    put_changed(d, shape.index_at, &index_bytes(shape, prim)?, &mut changed)?;
    let (b, r) = bounds(prim, &parts(d, shape)?);
    put_changed(d, shape.bounds_at, &b, &mut changed)?;
    put_changed(d, shape.radius_at, &r, &mut changed)?;
    Ok(if changed { Outcome::Changed } else { Outcome::Same })
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

fn add32(d: &mut [u8], at: usize, delta: usize) -> Result<()> {
    let v = le32(d, at)? as usize + delta;
    d[at..at + 4].copy_from_slice(&u32::try_from(v)?.to_le_bytes());
    Ok(())
}

fn grow(d: &mut Vec<u8>, m: &Models, delta: usize) -> Result<usize> {
    let end = m.gpu + m.region;
    let old_end = end - m.bf;
    let old_rlt = m.rlt - m.bf;
    let old_len = d.len();
    d.splice(end..end, std::iter::repeat_n(0u8, delta));
    let rlt = m.rlt + delta;
    if d.get(rlt..rlt + 4) != Some(b"_RLT") {
        bail!("the Switch shape model has no relocation table");
    }
    add32(d, m.bf + 0x18, delta)?;
    add32(d, m.bf + 0x1c, delta)?;
    if le32(d, rlt + 4)? as usize == old_rlt {
        add32(d, rlt + 4, delta)?;
    }
    let sections = le32(d, rlt + 8)? as usize;
    let entries = rlt + 0x10 + 0x18 * sections;
    for k in 0..sections {
        let e = rlt + 0x10 + 0x18 * k;
        if le32(d, e + 8)? as usize >= old_end {
            add32(d, e + 8, delta)?;
        }
        let first = le32(d, e + 0x10)? as usize;
        let count = le32(d, e + 0x14)? as usize;
        for j in first..first + count {
            let o = entries + 8 * j;
            let mut off = le32(d, o)? as usize;
            if off >= old_end {
                off += delta;
                d[o..o + 4].copy_from_slice(&u32::try_from(off)?.to_le_bytes());
            }
            let structs = le16(d, o + 4)? as usize;
            let (per, skip) = (*d.get(o + 6).context("cut short")? as usize, *d.get(o + 7).context("cut short")? as usize);
            let mut at = m.bf + off;
            for _ in 0..structs {
                for _ in 0..per {
                    let v = le64(d, at)?;
                    if v >= old_end {
                        d[at..at + 8].copy_from_slice(&((v + delta) as u64).to_le_bytes());
                    }
                    at += 8;
                }
                at += 8 * skip;
            }
        }
    }
    add32(d, m.g3pr + 4, delta)?;
    let next = le32(d, m.g3pr + 0xc)?;
    if next != NONE && next != 0 {
        add32(d, m.g3pr + 0xc, delta)?;
    }
    if le32(d, 0x1c)? as usize == old_len {
        let len = u32::try_from(d.len())?;
        d[0x1c..0x20].copy_from_slice(&len.to_le_bytes());
    }
    Ok(rlt)
}

pub fn relayout(d: &mut Vec<u8>, models: &Models, prims: &[&Prim]) -> Result<usize> {
    let mut indices: Vec<Vec<u8>> = vec![];
    let mut buffers: Vec<Vec<Vec<u8>>> = vec![];
    let mut counts: Vec<(usize, usize)> = vec![];
    let mut layouts: Vec<Option<Vec<(usize, usize)>>> = vec![];
    let mut changed = 0;
    for shape in &models.shapes {
        let width = index_width(shape);
        match prims.iter().find(|p| p.id == shape.id) {
            Some(p) => {
                let resized = p.arrays[POSITION].len() != shape.vertices || p.indices.len() != shape.index_count;
                layouts.push(Some(if resized { split(p.indices.len(), shape.sub_meshes) } else { parts(d, shape)? }));
                indices.push(index_bytes(shape, p)?);
                let existing = shape.buffers.iter().map(|&(at, size, _)| d[at..at + size].to_vec()).collect();
                buffers.push(fresh_buffers(shape, p, Some(existing))?);
                counts.push((p.arrays[POSITION].len(), p.indices.len()));
                changed += 1;
            }
            None => {
                layouts.push(None);
                indices.push(d[shape.index_at..shape.index_at + width * shape.index_count].to_vec());
                buffers.push(shape.buffers.iter().map(|&(at, size, _)| d[at..at + size].to_vec()).collect());
                counts.push((shape.vertices, shape.index_count));
            }
        }
    }
    let mut layout: Vec<u8> = vec![];
    let mut index_at = vec![];
    for b in &indices {
        layout.resize(align(layout.len(), 8), 0);
        index_at.push(layout.len());
        layout.extend(b);
    }
    layout.resize(align(layout.len(), 8), 0);
    let index_len = layout.len();
    let mut vertex_at = vec![];
    for set in &buffers {
        layout.resize(align(layout.len(), 8), 0);
        vertex_at.push(layout.len());
        for b in set {
            layout.resize(align(layout.len(), 8), 0);
            layout.extend(b);
        }
    }
    let region = models.region.max(align(layout.len(), 0x1000));
    let rlt = if region > models.region { grow(d, models, region - models.region)? } else { models.rlt };
    layout.resize(region, 0);
    d[models.gpu..models.gpu + region].copy_from_slice(&layout);
    let put = |d: &mut [u8], at: usize, v: usize| -> Result<()> {
        d[at..at + 4].copy_from_slice(&u32::try_from(v)?.to_le_bytes());
        Ok(())
    };
    put(d, models.info + 4, region)?;
    for (i, shape) in models.shapes.iter().enumerate() {
        let (vertices, index_count) = counts[i];
        let width = index_width(shape);
        put(d, shape.fvtx + 0x50, vertex_at[i])?;
        put(d, shape.fvtx + 0x58, vertices)?;
        for (k, b) in buffers[i].iter().enumerate() {
            put(d, shape.sizes + 0x10 * k, b.len())?;
        }
        put(d, shape.mesh + 0x20, index_at[i])?;
        put(d, shape.mesh + 0x2c, index_count)?;
        let size_at = models.bf + le64(d, shape.mesh + 0x18)?;
        put(d, size_at, width * index_count)?;
        let total = le32(d, shape.fmdl + 0x70)? as i64 + vertices as i64 - shape.vertices as i64;
        put(d, shape.fmdl + 0x70, total.max(0) as usize)?;
        if let (Some(p), Some(parts)) = (prims.iter().find(|p| p.id == shape.id), &layouts[i]) {
            for (k, &(start, count)) in parts.iter().enumerate() {
                put(d, shape.parts_at + 8 * k, width * start)?;
                put(d, shape.parts_at + 8 * k + 4, count)?;
            }
            let (b, r) = bounds(p, parts);
            d[shape.bounds_at..shape.bounds_at + b.len()].copy_from_slice(&b);
            d[shape.radius_at..shape.radius_at + 4].copy_from_slice(&r);
        }
    }
    let sections = le32(d, rlt + 8)? as usize;
    let gpu_rel = models.gpu - models.bf;
    for k in 0..sections {
        let e = rlt + 0x10 + 0x18 * k;
        let pos = le32(d, e + 8)? as usize;
        if k == 1 && pos == gpu_rel {
            put(d, e + 0xc, index_len)?;
        } else if k == 2 && pos >= gpu_rel && pos <= gpu_rel + region {
            put(d, e + 8, gpu_rel + index_len)?;
            put(d, e + 0xc, region - index_len)?;
        }
    }
    Ok(changed)
}
