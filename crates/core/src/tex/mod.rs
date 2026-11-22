pub mod bntx;
pub mod dict;
pub mod flim;
pub mod gx2;
pub mod tegra;

use anyhow_ext::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatInfo {
    pub block: u32,
    pub bytes: u32,
}

pub fn format_info(format: u32) -> Option<FormatInfo> {
    let (block, bytes) = match format >> 8 {
        0x02 => (1, 1),
        0x05 | 0x07 | 0x09 => (1, 2),
        0x0b | 0x0c | 0x0e => (1, 4),
        0x15 => (1, 8),
        0x1a | 0x1d => (4, 8),
        0x1b | 0x1c | 0x1e | 0x1f | 0x20 => (4, 16),
        _ => return None,
    };
    Some(FormatInfo { block, bytes })
}

fn fres_header() -> Vec<u8> {
    let mut h = vec![0u8; 0x150];
    let put = |h: &mut Vec<u8>, o: usize, v: &[u8]| h[o..o + v.len()].copy_from_slice(v);
    put(&mut h, 0, b"FRES    ");
    put(&mut h, 8, &[0x03, 0x00, 0x05, 0x00, 0xff, 0xfe, 0x0c, 0x00]);
    put(&mut h, 0x10, &0x152u32.to_le_bytes());
    put(&mut h, 0x16, &0x128u16.to_le_bytes());
    put(&mut h, 0x20, &0x150u64.to_le_bytes());
    put(&mut h, 0x98, &0xf0u64.to_le_bytes());
    put(&mut h, 0xa0, &0x100u64.to_le_bytes());
    put(&mut h, 0xb0, &0x13cu64.to_le_bytes());
    put(&mut h, 0xc8, &1u32.to_le_bytes());
    put(&mut h, 0xd0, &0x22u32.to_le_bytes());
    put(&mut h, 0xf0, &0x1000u64.to_le_bytes());
    put(&mut h, 0x100, &[0, 0, 0, 0, 1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 1, 0, 0, 0]);
    put(&mut h, 0x110, &0x13cu64.to_le_bytes());
    put(&mut h, 0x118, &[3, 0, 0, 0, 0, 0, 1, 0]);
    put(&mut h, 0x120, &0x140u64.to_le_bytes());
    put(&mut h, 0x128, b"_STR");
    put(&mut h, 0x138, &2u32.to_le_bytes());
    put(&mut h, 0x140, &[0x0d, 0x00]);
    put(&mut h, 0x142, b"textures.bntx");
    h
}

fn fres_rlt(at: usize, name_end: usize, bntx_span: usize) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(b"_RLT");
    r.extend_from_slice(&(at as u32).to_le_bytes());
    r.extend_from_slice(&5u32.to_le_bytes());
    r.extend_from_slice(&0u32.to_le_bytes());
    let sections: [(u32, u32, u32, u32); 5] = [
        (0, name_end as u32, 0, 4),
        (name_end as u32, 0, 4, 0),
        (name_end as u32, 0, 4, 0),
        (name_end as u32, 0, 4, 0),
        (0x1000, bntx_span as u32, 4, 1),
    ];
    for (pos, size, index, count) in sections {
        r.extend_from_slice(&0u64.to_le_bytes());
        r.extend_from_slice(&pos.to_le_bytes());
        r.extend_from_slice(&size.to_le_bytes());
        r.extend_from_slice(&index.to_le_bytes());
        r.extend_from_slice(&count.to_le_bytes());
    }
    for e in [(0x20u32, 7u16, 1u8, 1u8), (0x98, 1, 2, 0), (0xb0, 2, 1, 0x0b), (0x120, 1, 1, 0), (0xf0, 1, 1, 0)] {
        r.extend_from_slice(&e.0.to_le_bytes());
        r.extend_from_slice(&e.1.to_le_bytes());
        r.extend_from_slice(&[e.2, e.3]);
    }
    r
}

pub fn tex_sbfres(name: &str, bntx: &[u8]) -> Result<Vec<u8>> {
    if name.len() > 0xe00 {
        bail!("name too long");
    }
    let mut f = fres_header();
    f.extend_from_slice(&(name.len() as u16).to_le_bytes());
    f.extend_from_slice(name.as_bytes());
    f.push(0);
    if f.len() % 2 == 1 {
        f.push(0);
    }
    let name_end = f.len();
    f.resize(0x1000, 0);
    f.extend_from_slice(bntx);
    let rlt = 0x1000 + bntx.len().div_ceil(0x1000) * 0x1000;
    f.resize(rlt, 0);
    f.extend_from_slice(&fres_rlt(rlt, name_end, rlt - 0x1000));
    let size = f.len() as u32;
    f[0x18..0x1c].copy_from_slice(&(rlt as u32).to_le_bytes());
    f[0x1c..0x20].copy_from_slice(&size.to_le_bytes());
    f[0xb8..0xc0].copy_from_slice(&((name_end - 0x128) as u64).to_le_bytes());
    f[0xf8..0x100].copy_from_slice(&(bntx.len() as u64).to_le_bytes());
    f[0x130..0x134].copy_from_slice(&((rlt - 0x128) as u32).to_le_bytes());
    Ok(f)
}

pub fn bntx_of(fres: &[u8]) -> Result<&[u8]> {
    if fres.get(..4) != Some(b"FRES") {
        bail!("Not a BFRES file");
    }
    let at = u64::from_le_bytes(fres.get(0xf0..0xf8).context("short")?.try_into()?) as usize;
    let size = u64::from_le_bytes(fres.get(0xf8..0x100).context("short")?.try_into()?) as usize;
    fres.get(at..at + size).context("The BNTX in this BFRES is cut short")
}

pub fn neutral_texture(name: &str) -> bntx::Texture {
    let pixel: [u8; 4] = match name {
        crate::bfres::NEUTRAL_WHITE => [255, 255, 255, 255],
        crate::bfres::NEUTRAL_NORMAL => [128, 128, 255, 255],
        _ => [0, 0, 0, 0],
    };
    bntx::Texture {
        name: name.to_string(),
        format: 0x0b01,
        width: 4,
        height: 4,
        channels: [2, 3, 4, 5],
        levels: vec![pixel.repeat(16)],
        layers: 1,
        array: false,
        user_data: vec![],
    }
}

pub fn add_textures_to_tex(tex: &[u8], fres_name: &str, names: &[&str]) -> Result<Vec<u8>> {
    let compressed = tex.starts_with(b"Yaz0");
    let raw = if compressed { roead::yaz0::decompress(tex)? } else { tex.to_vec() };
    let mut textures = bntx::parse(bntx_of(&raw)?)?;
    let before = textures.len();
    for n in names {
        if !textures.iter().any(|t| t.name == *n) {
            textures.push(neutral_texture(n));
        }
    }
    if textures.len() == before {
        return Ok(tex.to_vec());
    }
    let fres = tex_sbfres(fres_name, &bntx::write("textures", &textures)?)?;
    Ok(if compressed { roead::yaz0::compress(&fres) } else { fres })
}

#[derive(Debug, Clone, Default)]
pub struct Converted {
    pub textures: usize,
    pub notes: Vec<String>,
}

pub fn convert_wiiu(name: &str, tex1: &[u8], tex2: Option<&[u8]>) -> Result<(Vec<u8>, Converted)> {
    convert_wiiu_parts(name, &[tex1], tex2)
}

pub fn convert_wiiu_parts(name: &str, tex1: &[&[u8]], tex2: Option<&[u8]>) -> Result<(Vec<u8>, Converted)> {
    convert_wiiu_split(name, tex1, tex2.as_slice())
}

pub fn convert_wiiu_split(name: &str, tex1: &[&[u8]], tex2: &[&[u8]]) -> Result<(Vec<u8>, Converted)> {
    let (textures, report) = wiiu_textures(tex1, tex2)?;
    let bntx = bntx::write("textures", &textures)?;
    let fres = tex_sbfres(&format!("{name}.Tex"), &bntx)?;
    Ok((roead::yaz0::compress(&fres), report))
}

pub fn convert_wiiu_container(raw: &[u8]) -> Result<Vec<u8>> {
    let (textures, _) = wiiu_textures(&[raw], &[])?;
    let bntx = bntx::write("textures", &textures)?;
    let file = crate::bfres::wiiu::read_textures_container(raw)?;
    if file.external.is_empty() {
        return tex_sbfres(&file.name, &bntx);
    }
    let mut file = file;
    file.external.insert(0, ("textures.bntx".into(), bntx));
    crate::bfres::switch::write(&file)
}

fn wiiu_textures(tex1: &[&[u8]], tex2: &[&[u8]]) -> Result<(Vec<bntx::Texture>, Converted)> {
    let unpack = |d: &[u8]| -> Result<Vec<u8>> {
        if d.starts_with(b"Yaz0") { Ok(roead::yaz0::decompress(d)?) } else { Ok(d.to_vec()) }
    };
    let parts = tex1
        .iter()
        .map(|d| gx2::parse(&unpack(d)?).context("Couldn't read the Wii U texture file"))
        .collect::<Result<Vec<_>>>()?;
    let mut t2 = Vec::new();
    for d in tex2 {
        t2.extend(gx2::parse(&unpack(d)?).context("Couldn't read the Wii U mip file (Tex2)")?);
    }
    if parts.iter().all(|p| p.is_empty()) {
        bail!("no textures in it");
    }
    let mut report = Converted::default();
    let mut textures = Vec::new();
    for (t1, s) in parts.iter().flat_map(|p| p.iter().map(move |s| (p, s))) {
        let base = s;
        let twin = t2.iter().find(|x| {
            x.name == s.name && x.width == s.width && x.height == s.height && x.format == s.format && x.mips == s.mips
        });
        let own_mips = s.mip_at.is_some_and(|m| {
            s.mip_data.len() as u32 >= s.mip_size
                && t1.iter().all(|o| o.data_at.is_none_or(|d| d + o.image_size as usize <= m
                    || d >= m + s.mip_size as usize))
        });
        let mips: Option<&[u8]> = match twin {
            Some(t) if s.mip_size > 0 => Some(&t.image),
            _ if own_mips && s.mip_size > 0 => Some(&s.mip_data),
            _ => None,
        };
        let format = gx2::to_bntx_format(base.format)
            .with_context(|| format!("{}: Wii U texture format {:#x} isn't supported yet", base.name, base.format))?;
        let mut levels = gx2::untile_all(base, mips).with_context(|| base.name.to_string())?;
        if base.format & 0x3f == 0x0a {
            for l in &mut levels {
                for px in l.chunks_mut(2) {
                    let v = u16::from_le_bytes([px[0], px[1]]);
                    let v = (v & 0x83e0) | ((v >> 10) & 0x1f) | ((v & 0x1f) << 10);
                    px.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        if (levels.len() as u32) < base.mips {
            report.notes.push(format!(
                "{}: {} of its {} mip levels (the rest weren't in the mod)",
                base.name,
                levels.len(),
                base.mips
            ));
        }
        textures.push(bntx::Texture {
            name: base.name.clone(),
            format,
            width: base.width,
            height: base.height,
            channels: gx2::to_bntx_channels(base.comp_sel),
            levels,
            layers: gx2::slices(base),
            array: base.dim == gx2::DIM_2D_ARRAY,
            user_data: base.user_data.clone(),
        });
    }
    report.textures = textures.len();
    Ok((textures, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn wiiu_file(name: &str, w: u32, h: u32, format: u32, mips: u32, swizzle: u32, levels: &[Vec<u8>],
                 kind: u8) -> Vec<u8> {
        let (bpp, _) = gx2::format_bits(format).unwrap();
        let mut s = gx2::Surface {
            name: name.into(), dim: 1, width: w, height: h, depth: 1, mips, format, aa: 0, image_size: 0,
            mip_size: 0, tile_mode: 4, swizzle, alignment: 0, pitch: 0, mip_offsets: [0; 13],
            comp_sel: [0, 1, 2, 3], image: vec![], mip_data: vec![], data_at: None, mip_at: None, user_data: vec![],
        };
        let tile = |l: &gx2::LevelInfo, linear: &[u8]| {
            let bpe = (bpp / 8) as usize;
            let mut out = vec![0u8; l.size as usize];
            for y in 0..l.height_elems {
                for x in 0..l.width_elems {
                    let a = gx2::address(x, y, l, bpp, swizzle) as usize;
                    let i = (y * l.width_elems + x) as usize * bpe;
                    out[a..a + bpe].copy_from_slice(&linear[i..i + bpe]);
                }
            }
            out
        };
        let l0 = gx2::level_info(&s, 0).unwrap();
        s.pitch = l0.pitch;
        s.alignment = l0.base_align;
        let image = tile(&l0, &levels[0]);
        let mut mip_data = Vec::new();
        for level in 1..mips {
            let l = gx2::level_info(&s, level).unwrap();
            let at = mip_data.len().div_ceil(l.base_align as usize) * l.base_align as usize;
            if level == 1 {
                s.mip_offsets[0] = image.len().div_ceil(l.base_align as usize) as u32 * l.base_align;
            } else {
                s.mip_offsets[level as usize - 1] = at as u32;
            }
            mip_data.resize(at, 0);
            mip_data.extend_from_slice(&tile(&l, &levels[level as usize]));
        }
        let mut f = vec![0u8; 0x6c];
        let be = |f: &mut Vec<u8>, o: usize, v: u32| f[o..o + 4].copy_from_slice(&v.to_be_bytes());
        f[..4].copy_from_slice(b"FRES");
        be(&mut f, 4, 0x0304_0002);
        f[8..10].copy_from_slice(&[0xfe, 0xff]);
        let group = f.len();
        f.resize(group + 8 + 32, 0);
        be(&mut f, 0x24, (group - 0x24) as u32);
        be(&mut f, group + 4, 1);
        let ftex = f.len();
        f.resize(ftex + 0xc0, 0);
        be(&mut f, group + 8 + 16 + 12, (ftex - (group + 8 + 16 + 12)) as u32);
        f[ftex..ftex + 4].copy_from_slice(b"FTEX");
        for (o, v) in [(4, 1), (8, w), (0xc, h), (0x10, 1), (0x14, mips), (0x18, format), (0x24, image.len() as u32),
                       (0x2c, mip_data.len() as u32), (0x34, 4), (0x38, swizzle),
                       (0x3c, s.alignment), (0x40, s.pitch), (0x88, 0x0001_0203)] {
            be(&mut f, ftex + o, v);
        }
        for k in 0..13 {
            be(&mut f, ftex + 0x44 + 4 * k, s.mip_offsets[k]);
        }
        let name_at = f.len() + 4;
        be(&mut f, ftex + 0xa8, (name_at - (ftex + 0xa8)) as u32);
        f.extend_from_slice(&(name.len() as u32).to_be_bytes());
        f.extend_from_slice(name.as_bytes());
        f.push(0);
        f.resize(f.len().div_ceil(0x100) * 0x100, 0);
        let at = f.len();
        be(&mut f, ftex + 0xb0, (at - (ftex + 0xb0)) as u32);
        match kind {
            2 => f.extend_from_slice(&mip_data),
            _ => f.extend_from_slice(&image),
        }
        let at = f.len();
        be(&mut f, ftex + 0xb4, (at - (ftex + 0xb4)) as u32);
        if kind == 0 {
            f.extend_from_slice(&mip_data);
        }
        f
    }

    fn levels(w: u32, h: u32, mips: u32, bpe: usize) -> Vec<Vec<u8>> {
        (0..mips)
            .map(|m| {
                let (wb, hb) = ((w >> m).max(1).div_ceil(4) as usize, (h >> m).max(1).div_ceil(4) as usize);
                (0..wb * hb * bpe).map(|i| (i * 31 + m as usize * 7) as u8).collect()
            })
            .collect()
    }

    #[test]
    fn wiiu_texture_to_switch() {
        for (w, h, fmt, bpe, sw) in [(256u32, 128u32, 0x431u32, 8usize, 0x300u32), (512, 512, 0x35, 16, 0x100),
                                     (64, 32, 0x33, 16, 0)] {
            let mips = (w.max(h)).trailing_zeros() + 1;
            let lv = levels(w, h, mips, bpe);
            let one = wiiu_file("Test_Alb", w, h, fmt, mips, sw, &lv, 0);
            let tex1 = wiiu_file("Test_Alb", w, h, fmt, mips, sw, &lv, 1);
            let tex2 = wiiu_file("Test_Alb", w, h, fmt, mips, sw, &lv, 2);
            for (t1, t2) in [(&one, None), (&tex1, Some(tex2.as_slice()))] {
                let (out, r) = convert_wiiu("Test", t1, t2).unwrap();
                assert_eq!(r.textures, 1);
                assert!(r.notes.is_empty(), "{:?}", r.notes);
                let fres = roead::yaz0::decompress(&out).unwrap();
                let t = &bntx::parse(bntx_of(&fres).unwrap()).unwrap()[0];
                assert_eq!((t.name.as_str(), t.width, t.height), ("Test_Alb", w, h));
                assert_eq!(t.format, gx2::to_bntx_format(fmt).unwrap());
                assert_eq!(t.channels, [2, 3, 4, 5]);
                assert_eq!(t.levels, lv, "{w}x{h} {fmt:#x}");
            }
        }
    }

    #[test]
    fn neutral_textures_added() {
        let lv = levels(64, 32, 7, 16);
        let one = wiiu_file("Test_Alb", 64, 32, 0x33, 7, 0, &lv, 0);
        let (tex, _) = convert_wiiu("Test", &one, None).unwrap();
        let names = [crate::bfres::NEUTRAL_BLACK, crate::bfres::NEUTRAL_NORMAL];
        let out = add_textures_to_tex(&tex, "Test.Tex", &names).unwrap();
        let before = bntx::parse(bntx_of(&roead::yaz0::decompress(&tex).unwrap()).unwrap()).unwrap();
        let after = bntx::parse(bntx_of(&roead::yaz0::decompress(&out).unwrap()).unwrap()).unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0], before[0]);
        let black = after.iter().find(|t| t.name == crate::bfres::NEUTRAL_BLACK).unwrap();
        assert_eq!(black.levels, vec![vec![0u8; 64]]);
        let normal = after.iter().find(|t| t.name == crate::bfres::NEUTRAL_NORMAL).unwrap();
        assert_eq!(normal.levels, vec![[128u8, 128, 255, 255].repeat(16)]);
        assert_eq!(add_textures_to_tex(&out, "Test.Tex", &names).unwrap(), out);
    }
}
