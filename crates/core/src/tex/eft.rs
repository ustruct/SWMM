use anyhow_ext::{Context, Result, bail};

use super::{bntx, gx2};

const NONE: u32 = 0xffff_ffff;

fn be32(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("effect texture data is cut short")?.try_into()?))
}

fn le32(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).context("effect texture data is cut short")?.try_into()?))
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

pub struct WiiuTexture {
    pub id: u64,
    pub texture: bntx::Texture,
}

fn surface(desc: &[u8]) -> Result<gx2::Surface> {
    let width = u16::from_be_bytes([desc[0], desc[1]]) as u32;
    let height = u16::from_be_bytes([desc[2], desc[3]]) as u32;
    let depth = be32(desc, 4)?;
    let sel = be32(desc, 8)?.to_be_bytes();
    let mips = be32(desc, 0xc)?.max(1);
    let format = be32(desc, 0x10)?;
    let tile_mode = be32(desc, 0x14)?;
    let swizzle = be32(desc, 0x18)?;
    if width == 0 || height == 0 {
        bail!("an effect texture with no size");
    }
    Ok(gx2::Surface {
        name: String::new(),
        dim: 1,
        width,
        height,
        depth,
        mips,
        format,
        aa: 0,
        image_size: 0,
        mip_size: 0,
        tile_mode,
        swizzle,
        alignment: 0,
        pitch: 0,
        mip_offsets: [0; 13],
        comp_sel: sel,
        image: vec![],
        mip_data: vec![],
        data_at: None,
        mip_at: None,
        user_data: vec![],
    })
}

fn decode(desc: &[u8], data: &[u8]) -> Result<bntx::Texture> {
    let mut s = surface(desc)?;
    let l0 = gx2::level_info(&s, 0)?;
    s.pitch = l0.pitch;
    s.image_size = l0.size;
    let mut mip_start = l0.size as usize;
    let mut end = 0usize;
    for level in 1..s.mips.min(14) {
        let li = gx2::level_info(&s, level)?;
        if level == 1 {
            mip_start = align(l0.size as usize, li.base_align as usize);
        } else {
            let at = align(end, li.base_align as usize);
            s.mip_offsets[level as usize - 1] = at as u32;
            end = at;
        }
        end += li.size as usize;
    }
    s.mip_size = end as u32;
    s.image = data.get(..l0.size as usize).context("effect texture image is cut short")?.to_vec();
    let mips = data.get(mip_start.min(data.len())..).unwrap_or_default();
    let format = gx2::to_bntx_format(s.format).with_context(|| format!("effect texture format {:#x} isn't supported yet", s.format))?;
    let mut levels = gx2::untile_all(&s, if s.mips > 1 { Some(mips) } else { None })?;
    if s.format & 0x3f == 0x0a {
        for l in &mut levels {
            for px in l.chunks_mut(2) {
                let v = u16::from_le_bytes([px[0], px[1]]);
                let v = (v & 0x83e0) | ((v >> 10) & 0x1f) | ((v & 0x1f) << 10);
                px.copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(bntx::Texture {
        name: String::new(),
        format,
        width: s.width,
        height: s.height,
        channels: gx2::to_bntx_channels(s.comp_sel),
        levels,
        layers: 1,
        array: false,
        user_data: vec![],
    })
}

pub fn wiiu_textures(d: &[u8], texa: usize) -> Result<Vec<WiiuTexture>> {
    if d.get(texa..texa + 4) != Some(b"TEXA") {
        bail!("not an effect texture list");
    }
    let mut out = vec![];
    let child = be32(d, texa + 8)?;
    if child == NONE || child == 0 || be32(d, texa + 4)? == 0 {
        return Ok(out);
    }
    let mut at = texa + child as usize;
    loop {
        if d.get(at..at + 4) != Some(b"TEXR") {
            bail!("an effect texture entry isn't where its list says");
        }
        let gx = be32(d, at + 8)? as usize;
        let desc = d.get(at + 0x20..at + gx).context("effect texture header is cut short")?;
        if desc.len() < 0x30 {
            bail!("effect texture header is too short");
        }
        let id = be32(desc, 0x24)? as u64;
        let g = at + gx;
        if d.get(g..g + 4) != Some(b"GX2B") {
            bail!("effect texture {id:#x} has no image data");
        }
        let size = be32(d, g + 4)? as usize;
        let start = g + be32(d, g + 0x14)? as usize;
        let data = d.get(start..start + size).context("effect texture image is cut short")?;
        let texture = decode(desc, data).with_context(|| format!("effect texture {id:#010x}"))?;
        out.push(WiiuTexture { id, texture });
        let next = be32(d, at + 0xc)?;
        if next == NONE || next == 0 {
            break;
        }
        at += next as usize;
    }
    Ok(out)
}

pub struct SwitchTextures {
    pub names: Vec<(u64, String)>,
    pub textures: Vec<bntx::Texture>,
}

pub fn switch_textures(d: &[u8], grtf: usize) -> Result<SwitchTextures> {
    if d.get(grtf..grtf + 4) != Some(b"GRTF") {
        bail!("not a Switch effect texture list");
    }
    let mut names = vec![];
    let size = le32(d, grtf + 4)? as usize;
    let child = le32(d, grtf + 8)?;
    if size == 0 || child == NONE {
        return Ok(SwitchTextures { names, textures: vec![] });
    }
    let g = grtf + child as usize;
    if d.get(g..g + 4) == Some(b"GTNT") {
        let len = le32(d, g + 4)? as usize;
        let mut k = g + le32(d, g + 0x14)? as usize;
        let end = k + len;
        while k + 16 <= end {
            let id = u64::from_le_bytes(d[k..k + 8].try_into()?);
            let step = le32(d, k + 8)? as usize;
            let n = le32(d, k + 12)? as usize;
            let raw = d.get(k + 16..k + 16 + n).context("texture name is cut short")?;
            names.push((id, String::from_utf8_lossy(raw.split(|b| *b == 0).next().unwrap_or_default()).into_owned()));
            if step == 0 {
                break;
            }
            k += step;
        }
    }
    let bn = grtf + le32(d, grtf + 0x14)? as usize;
    let textures = bntx::parse(d.get(bn..bn + size).context("effect BNTX is cut short")?)?;
    Ok(SwitchTextures { names, textures })
}

pub fn build_grtf(at: usize, header: &[u8], names: &[(u64, String)], bntx: &[u8], next_align: Option<usize>) -> Vec<u8> {
    let mut gtnt = vec![];
    for (i, (id, name)) in names.iter().enumerate() {
        let start = gtnt.len();
        let n = name.len() + 1;
        gtnt.extend(id.to_le_bytes());
        gtnt.extend(0u32.to_le_bytes());
        gtnt.extend((n as u32).to_le_bytes());
        gtnt.extend(name.as_bytes());
        gtnt.push(0);
        let len = align(16 + n + 1, 8);
        gtnt.resize(start + len, 0);
        if i + 1 < names.len() {
            gtnt[start + 8..start + 12].copy_from_slice(&(len as u32).to_le_bytes());
        }
    }
    let mut out = header.get(..0x20).map(<[u8]>::to_vec).unwrap_or_else(|| vec![0; 0x20]);
    out[0..4].copy_from_slice(b"GRTF");
    let put = |o: &mut Vec<u8>, at: usize, v: u32| o[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let gtnt_next = align(0x20 + gtnt.len(), 16);
    let mut g = vec![0u8; 0x20];
    g[0..4].copy_from_slice(b"GTNT");
    put(&mut g, 4, gtnt.len() as u32);
    put(&mut g, 8, NONE);
    put(&mut g, 0xc, gtnt_next as u32);
    put(&mut g, 0x10, NONE);
    put(&mut g, 0x14, 0x20);
    out.extend(g);
    out.extend(gtnt);
    let min = 0x20 + gtnt_next;
    let mut binoff = align(at + min, 0x1000) - at;
    if binoff < min {
        binoff += 0x1000;
    }
    out.resize(binoff, 0);
    out.extend_from_slice(bntx);
    let mut next = align(out.len(), 16);
    if let Some(want) = next_align {
        next += (want as isize - (at + next) as isize).rem_euclid(0x1000) as usize;
    }
    out.resize(next, 0);
    put(&mut out, 4, bntx.len() as u32);
    put(&mut out, 8, 0x20);
    put(&mut out, 0xc, next as u32);
    put(&mut out, 0x10, NONE);
    put(&mut out, 0x14, binoff as u32);
    out
}

pub fn texture_name(id: u64) -> String {
    format!("SwmmEffectTexture_{id:08x}")
}
