use anyhow_ext::{Context, Result, bail};

use crate::bfres::model::{UserData, UserValue};

#[derive(Debug, Clone)]
pub struct Surface {
    pub name: String,
    pub dim: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mips: u32,
    pub format: u32,
    pub aa: u32,
    pub image_size: u32,
    pub mip_size: u32,
    pub tile_mode: u32,
    pub swizzle: u32,
    pub alignment: u32,
    pub pitch: u32,
    pub mip_offsets: [u32; 13],
    pub comp_sel: [u8; 4],
    pub image: Vec<u8>,
    pub mip_data: Vec<u8>,
    pub data_at: Option<usize>,
    pub mip_at: Option<usize>,
    pub user_data: Vec<UserData>,
}

fn be32(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(o..o + 4).context("Wii U BFRES is cut short")?.try_into()?))
}
fn be16(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(b.get(o..o + 2).context("Wii U BFRES is cut short")?.try_into()?))
}
fn rel(b: &[u8], o: usize) -> Result<Option<usize>> {
    let v = be32(b, o)? as i32;
    Ok((v != 0).then(|| (o as i64 + v as i64) as usize))
}
fn cstr_at(b: &[u8], o: usize) -> Result<String> {
    let rest = b.get(o..).context("string past the end")?;
    let n = rest.iter().position(|c| *c == 0).context("string without its end")?;
    Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
}

fn user_data(b: &[u8], field: usize) -> Result<Vec<UserData>> {
    let Some(g) = rel(b, field)? else { return Ok(vec![]) };
    let n = be32(b, g + 4)? as usize;
    (1..=n)
        .map(|i| {
            let e = g + 8 + 16 * i;
            let name = match rel(b, e + 8)? {
                Some(o) => cstr_at(b, o)?,
                None => String::new(),
            };
            let u = rel(b, e + 12)?.context("user data without data")?;
            let count = be16(b, u + 4)? as usize;
            let value = match b.get(u + 6).copied() {
                Some(0) => UserValue::Int((0..count).map(|k| Ok(be32(b, u + 8 + 4 * k)? as i32)).collect::<Result<_>>()?),
                Some(1) => UserValue::Float(
                    (0..count).map(|k| Ok(f32::from_bits(be32(b, u + 8 + 4 * k)?))).collect::<Result<_>>()?,
                ),
                Some(2) => UserValue::Str(
                    (0..count)
                        .map(|k| match rel(b, u + 8 + 4 * k)? {
                            Some(o) => cstr_at(b, o),
                            None => Ok(String::new()),
                        })
                        .collect::<Result<_>>()?,
                    false,
                ),
                Some(3) => UserValue::Str(
                    (0..count)
                        .map(|k| {
                            let Some(mut o) = rel(b, u + 8 + 4 * k)? else { return Ok(String::new()) };
                            let mut units = Vec::new();
                            loop {
                                match be16(b, o)? {
                                    0 => break,
                                    c => units.push(c),
                                }
                                o += 2;
                            }
                            Ok(String::from_utf16_lossy(&units))
                        })
                        .collect::<Result<_>>()?,
                    false,
                ),
                Some(4) => UserValue::Bytes(b.get(u + 8..u + 8 + count).context("user data cut short")?.to_vec()),
                k => bail!("texture user data {name}: type {k:?} isn't supported"),
            };
            Ok(UserData { name, value })
        })
        .collect()
}

fn name_at(b: &[u8], o: usize) -> Result<String> {
    let len = be32(b, o.checked_sub(4).context("bad name")?)? as usize;
    Ok(String::from_utf8_lossy(b.get(o..o + len).context("name cut short")?).into_owned())
}

pub fn parse(b: &[u8]) -> Result<Vec<Surface>> {
    if b.get(..4) != Some(b"FRES") || be16(b, 8)? != 0xfeff {
        bail!("Not a Wii U BFRES file");
    }
    let Some(group) = rel(b, 0x24)? else { return Ok(vec![]) };
    let count = be32(b, group + 4)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 1..=count {
        let e = group + 8 + 16 * i;
        let Some(t) = rel(b, e + 12)? else { continue };
        if b.get(t..t + 4) != Some(b"FTEX") {
            bail!("Texture {i} of this BFRES isn't an FTEX section");
        }
        let f = |o: usize| be32(b, t + o);
        let mut mip_offsets = [0u32; 13];
        for (k, m) in mip_offsets.iter_mut().enumerate() {
            *m = f(0x44 + 4 * k)?;
        }
        let name = match rel(b, t + 0xa8)? {
            Some(n) => name_at(b, n)?,
            None => format!("texture{i}"),
        };
        let image_size = f(0x24)?;
        let mip_size = f(0x2c)?;
        let clip = |at: Option<usize>, size: u32| -> Vec<u8> {
            at.map(|d| b.get(d.min(b.len())..(d + size as usize).min(b.len())).unwrap_or_default().to_vec())
                .unwrap_or_default()
        };
        let data_at = rel(b, t + 0xb0)?;
        let mip_at = rel(b, t + 0xb4)?;
        let image = clip(data_at, image_size.max(mip_size));
        let mip_data = clip(mip_at, mip_size);
        let cs = f(0x88)?.to_be_bytes();
        out.push(Surface {
            name,
            dim: f(0x04)?,
            width: f(0x08)?,
            height: f(0x0c)?,
            depth: f(0x10)?,
            mips: f(0x14)?,
            format: f(0x18)?,
            aa: f(0x1c)?,
            image_size,
            mip_size,
            tile_mode: f(0x34)?,
            swizzle: f(0x38)?,
            alignment: f(0x3c)?,
            pitch: f(0x40)?,
            mip_offsets,
            comp_sel: cs,
            image,
            mip_data,
            data_at,
            mip_at,
            user_data: user_data(b, t + 0xb8)?,
        });
    }
    Ok(out)
}

pub fn format_bits(format: u32) -> Option<(u32, bool)> {
    Some(match format & 0x3f {
        0x01 => (8, false),
        0x07 | 0x08 | 0x0a => (16, false),
        0x19 | 0x1a => (32, false),
        0x31 | 0x34 => (64, true),
        0x32 | 0x33 | 0x35 => (128, true),
        _ => return None,
    })
}

pub fn to_bntx_format(format: u32) -> Option<u32> {
    Some(match format {
        0x001 => 0x0201,
        0x007 => 0x0901,
        0x00a => 0x0501,
        0x01a => 0x0b01,
        0x41a => 0x0b06,
        0x031 => 0x1a01,
        0x431 => 0x1a06,
        0x032 => 0x1b01,
        0x432 => 0x1b06,
        0x033 => 0x1c01,
        0x433 => 0x1c06,
        0x034 => 0x1d01,
        0x234 => 0x1d02,
        0x035 => 0x1e01,
        0x235 => 0x1e02,
        _ => return None,
    })
}

pub fn to_bntx_channels(sel: [u8; 4]) -> [u8; 4] {
    sel.map(|s| match s {
        0 => 2,
        1 => 3,
        2 => 4,
        3 => 5,
        4 => 0,
        _ => 1,
    })
}

const PIPES: u32 = 2;
const BANKS: u32 = 4;
const PIPE_INTERLEAVE: u32 = 256;

fn next_pow2(x: u32) -> u32 {
    x.max(1).next_power_of_two()
}

fn align(x: u32, a: u32) -> u32 {
    x.div_ceil(a) * a
}

fn thickness(tile_mode: u32) -> u32 {
    match tile_mode {
        3 | 7 | 11 | 13 | 15 => 4,
        16 => 8,
        _ => 1,
    }
}

fn is_bank_swapped(tile_mode: u32) -> bool {
    matches!(tile_mode, 8..=11 | 14 | 15)
}

fn aspect_ratio(tile_mode: u32) -> u32 {
    match tile_mode {
        5 | 9 => 2,
        6 | 10 => 4,
        _ => 1,
    }
}

fn mip_tile_mode(base: u32, bpp: u32, level: u32, width: u32, height: u32) -> u32 {
    let mut mode = base;
    if level == 0 {
        return mode;
    }
    let w = next_pow2(width);
    let h = next_pow2(height);
    mode = match mode {
        8 => 4,
        9 => 5,
        10 => 6,
        11 => 7,
        14 => 12,
        15 => 13,
        m => m,
    };
    let micro_bytes = (bpp * thickness(mode) * 64).div_ceil(8);
    let width_align = if micro_bytes < PIPE_INTERLEAVE { PIPE_INTERLEAVE / micro_bytes } else { 1 };
    let (mut mw, mut mh) = (8 * BANKS, 8 * PIPES);
    match mode {
        5 => {
            mw >>= 1;
            mh *= 2;
        }
        6 => {
            mw >>= 2;
            mh *= 4;
        }
        _ => {}
    }
    if matches!(mode, 4 | 5 | 6 | 12) && (w < width_align * mw || h < mh) {
        mode = 2;
    } else if matches!(mode, 7 | 13) && (w < width_align * mw || h < mh) {
        mode = 3;
    }
    match mode {
        3 => 2,
        7 => 4,
        13 => 12,
        m => m,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelInfo {
    pub tile_mode: u32,
    pub pitch: u32,
    pub height: u32,
    pub size: u32,
    pub base_align: u32,
    pub width_elems: u32,
    pub height_elems: u32,
}

pub const DIM_2D_ARRAY: u32 = 5;

pub fn slices(s: &Surface) -> u32 {
    if s.dim == DIM_2D_ARRAY { s.depth.max(1) } else { 1 }
}

pub fn level_info(s: &Surface, level: u32) -> Result<LevelInfo> {
    let (bpp, bc) = format_bits(s.format).with_context(|| format!("texture format {:#x}", s.format))?;
    if s.aa != 0 || !(s.dim == 1 && s.depth <= 1 || s.dim == DIM_2D_ARRAY) {
        bail!("only 2D textures and 2D texture arrays are converted (dimension {}, depth {})", s.dim, s.depth);
    }
    let (mut w, mut h) = ((s.width >> level).max(1), (s.height >> level).max(1));
    let real = (if bc { w.div_ceil(4) } else { w }, if bc { h.div_ceil(4) } else { h });
    if bc {
        if level == 0 {
            w = align(w, 4);
            h = align(h, 4);
        } else {
            w = next_pow2(w);
            h = next_pow2(h);
        }
        w = w.div_ceil(4);
        h = h.div_ceil(4);
    }
    let base = s.tile_mode & 0xf;
    if is_bank_swapped(base) {
        bail!("bank-swapped tile mode {base} isn't supported");
    }
    let mode = mip_tile_mode(base, bpp, level, w, h);
    let th = thickness(mode);
    let (pitch_align, height_align, base_align) = match mode {
        0 => (if bpp == 8 { 8 } else { 1 }, 1, 1),
        1 => ((PIPE_INTERLEAVE * 8 / bpp).max(64), 1, PIPE_INTERLEAVE),
        2 | 3 => ((PIPE_INTERLEAVE / bpp / th).max(8), 8, PIPE_INTERLEAVE),
        _ => {
            let ratio = aspect_ratio(mode);
            let mw = 8 * BANKS / ratio;
            let mh = ratio * 8 * PIPES;
            let pa = mw.max(mw * (PIPE_INTERLEAVE / bpp / (8 * th)));
            let macro_bytes = (bpp * mh * mw).div_ceil(8);
            let ba = if th == 1 {
                macro_bytes.max((mh * bpp * pa).div_ceil(8))
            } else {
                PIPE_INTERLEAVE.max((4 * mh * bpp * pa).div_ceil(8))
            };
            (pa, mh, ba)
        }
    };
    let pitch = align(w, pitch_align);
    let height = align(h, height_align);
    let size = (pitch * height * bpp * th).div_ceil(8);
    Ok(LevelInfo { tile_mode: mode, pitch, height, size, base_align, width_elems: real.0, height_elems: real.1 })
}

fn pixel_index(x: u32, y: u32, bpp: u32) -> u32 {
    let (x0, x1, x2) = (x & 1, (x >> 1) & 1, (x >> 2) & 1);
    let (y0, y1, y2) = (y & 1, (y >> 1) & 1, (y >> 2) & 1);
    let bits = match bpp {
        8 => [x0, x1, x2, y1, y0, y2],
        16 => [x0, x1, x2, y0, y1, y2],
        32 | 96 => [x0, x1, y0, x2, y1, y2],
        64 => [x0, y0, x1, x2, y1, y2],
        _ => [y0, x0, x1, x2, y1, y2],
    };
    bits.iter().enumerate().fold(0, |a, (i, b)| a | (b << i))
}

fn pipe_of(x: u32, y: u32) -> u32 {
    ((y >> 3) ^ (x >> 3)) & 1
}

fn bank_of(x: u32, y: u32) -> u32 {
    let b0 = ((y / (16 * PIPES)) ^ (x >> 3)) & 1;
    let b1 = ((y / (8 * PIPES)) ^ (x >> 4)) & 1;
    b0 | (b1 << 1)
}

pub fn address(x: u32, y: u32, l: &LevelInfo, bpp: u32, swizzle: u32) -> u64 {
    address_in_slice(x, y, 0, l, bpp, swizzle)
}

pub fn address_in_slice(x: u32, y: u32, slice: u32, l: &LevelInfo, bpp: u32, swizzle: u32) -> u64 {
    let slice_bytes = l.size as u64;
    match l.tile_mode {
        0 | 1 => slice as u64 * slice_bytes + (y as u64 * l.pitch as u64 + x as u64) * bpp as u64 / 8,
        2 | 3 => {
            let th = thickness(l.tile_mode);
            let micro_bytes = (64 * th * bpp).div_ceil(8) as u64;
            let tiles_per_row = (l.pitch / 8) as u64;
            let tile = micro_bytes * ((x / 8) as u64 + (y / 8) as u64 * tiles_per_row);
            slice as u64 * slice_bytes + tile + (bpp * pixel_index(x, y, bpp) / 8) as u64
        }
        mode => {
            let th = thickness(mode);
            let elem = (bpp * pixel_index(x, y, bpp) / 8) as u64;
            let pipe_swizzle = (swizzle >> 8) & 1;
            let bank_swizzle = (swizzle >> 9) & 3;
            let mut bank_pipe = pipe_of(x, y) + PIPES * bank_of(x, y);
            let rotation = match mode {
                4..=11 => PIPES * ((BANKS >> 1) - 1),
                12..=15 => 1,
                _ => 0,
            };
            let sw = pipe_swizzle + PIPES * bank_swizzle;
            bank_pipe ^= sw + (slice / th) * rotation;
            bank_pipe %= PIPES * BANKS;
            let pipe = bank_pipe % PIPES;
            let bank = bank_pipe / PIPES;
            let ratio = aspect_ratio(mode);
            let (mtw, mth) = (8 * BANKS / ratio, 8 * PIPES * ratio);
            let tiles_per_row = (l.pitch / mtw) as u64;
            let macro_bytes = ((th * bpp * mth * mtw).div_ceil(8)) as u64;
            let macro_offset = ((x / mtw) as u64 + tiles_per_row * (y / mth) as u64) * macro_bytes;
            let slice_offset = slice_bytes * (slice / th) as u64;
            let total = elem + ((macro_offset + slice_offset) >> 3);
            let group_mask = (PIPE_INTERLEAVE - 1) as u64;
            let high = (total & !group_mask) << 3;
            let low = total & group_mask;
            ((bank as u64) << 9) | ((pipe as u64) << 8) | low | high
        }
    }
}

pub fn untile(data: &[u8], l: &LevelInfo, bpp: u32, swizzle: u32) -> Result<Vec<u8>> {
    untile_slice(data, 0, l, bpp, swizzle)
}

pub fn untile_slice(data: &[u8], slice: u32, l: &LevelInfo, bpp: u32, swizzle: u32) -> Result<Vec<u8>> {
    let bpe = (bpp / 8) as usize;
    let mut out = vec![0u8; l.width_elems as usize * l.height_elems as usize * bpe];
    for y in 0..l.height_elems {
        for x in 0..l.width_elems {
            let a = address_in_slice(x, y, slice, l, bpp, swizzle) as usize;
            let src = data.get(a..a + bpe).context("texture data is shorter than its layout")?;
            let o = (y as usize * l.width_elems as usize + x as usize) * bpe;
            out[o..o + bpe].copy_from_slice(src);
        }
    }
    Ok(out)
}

pub fn untile_all(s: &Surface, mips: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
    let (bpp, _) = format_bits(s.format).with_context(|| format!("texture format {:#x} isn't supported", s.format))?;
    let l0 = level_info(s, 0)?;
    let n = slices(s);
    let untile_level = |data: &[u8], l: &LevelInfo| -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for k in 0..n {
            out.extend(untile_slice(data, k, l, bpp, s.swizzle)?);
        }
        Ok(out)
    };
    if l0.pitch != s.pitch || l0.size * n != s.image_size {
        bail!(
            "its layout doesn't add up (pitch {} / {} expected, size {:#x} / {:#x} expected, tile mode {})",
            s.pitch,
            l0.pitch,
            s.image_size,
            l0.size * n,
            s.tile_mode
        );
    }
    if s.alignment != 0 && l0.base_align != s.alignment {
        log::warn!(
            "{}: its header says alignment {:#x}, its layout gives {:#x}; read with the layout's",
            s.name,
            s.alignment,
            l0.base_align
        );
    }
    let image = s.image.get(..(l0.size * n) as usize).context("texture data is cut short")?;
    let mut levels = vec![untile_level(image, &l0)?];
    let Some(mips) = mips else { return Ok(levels) };
    if s.mips <= 1 {
        return Ok(levels);
    }
    if (mips.len() as u32) < s.mip_size {
        log::warn!("{}: its mip data is cut short ({:#x} of {:#x} bytes)", s.name, mips.len(), s.mip_size);
    }
    let mut end = 0u32;
    let mut prev_macro = l0.tile_mode >= 4;
    for level in 1..s.mips {
        let li = level_info(s, level)?;
        let at = if level == 1 { 0 } else { s.mip_offsets[level as usize - 1] };
        let plain = align(end, li.base_align);
        let shifted = align(end + if prev_macro { s.swizzle & 0x700 } else { 0 }, li.base_align);
        if level > 1 && at != plain && at != shifted {
            log::warn!("{}: mip level {level} isn't where its layout puts it ({at:#x} / {plain:#x}): read where the header says", s.name);
        }
        let room = li.size * if n > 1 { next_pow2(n) } else { 1 };
        let Some(data) = mips.get(at as usize..(at + room) as usize) else {
            log::warn!("{}: mip levels from {level} on are cut short: left out", s.name);
            break;
        };
        levels.push(untile_level(data, &li)?);
        end = at + room;
        prev_macro = li.tile_mode >= 4;
    }
    if end > s.mip_size {
        log::warn!("{}: its mip levels need {end:#x} bytes, the header says {:#x}", s.name, s.mip_size);
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(w: u32, h: u32, format: u32, tile_mode: u32, swizzle: u32) -> Surface {
        Surface {
            name: "t".into(),
            dim: 1,
            width: w,
            height: h,
            depth: 1,
            mips: 1,
            format,
            aa: 0,
            image_size: 0,
            mip_size: 0,
            tile_mode,
            swizzle,
            alignment: 0,
            pitch: 0,
            mip_offsets: [0; 13],
            comp_sel: [0, 1, 2, 3],
            image: vec![],
            mip_data: vec![],
            data_at: None,
            mip_at: None,
            user_data: vec![],
        }
    }

    #[test]
    fn tiling_is_one_to_one() {
        for (w, h, fmt) in [(256, 256, 0x31), (512, 128, 0x33), (64, 64, 0x1a), (128, 256, 0x35), (96, 40, 0x31),
                            (32, 32, 0x01)] {
            for mode in [2, 4] {
                for swizzle in [0, 0x100, 0x300, 0x700] {
                    let s = surface(w, h, fmt, mode, swizzle);
                    let (bpp, _) = format_bits(fmt).unwrap();
                    for level in 0..4 {
                        let l = level_info(&s, level).unwrap();
                        let mut seen = std::collections::HashSet::new();
                        for y in 0..l.height {
                            for x in 0..l.pitch {
                                let a = address(x, y, &l, bpp, swizzle);
                                assert!(a + (bpp / 8) as u64 <= l.size as u64, "{w}x{h} {fmt:#x} mode {mode} level {level}");
                                assert!(seen.insert(a), "{w}x{h} {fmt:#x} mode {mode} level {level}: twice at {a:#x}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn layout_of_common_textures() {
        let s = surface(256, 256, 0x31, 4, 0);
        let l0 = level_info(&s, 0).unwrap();
        assert_eq!((l0.pitch, l0.height, l0.size, l0.base_align), (64, 64, 0x8000, 0x1000));
        let l1 = level_info(&s, 1).unwrap();
        assert_eq!((l1.tile_mode, l1.pitch, l1.size), (4, 32, 0x2000));
        let l2 = level_info(&s, 2).unwrap();
        assert_eq!((l2.tile_mode, l2.pitch, l2.height, l2.size), (2, 16, 16, 0x800));
    }
}
