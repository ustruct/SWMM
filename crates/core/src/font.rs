use std::collections::HashMap;

use anyhow_ext::{Context, Result, bail};

use crate::tex::{bntx, gx2};

fn be16(d: &[u8], o: usize) -> Result<usize> {
    Ok(u16::from_be_bytes(d.get(o..o + 2).context("the font is cut short")?.try_into()?) as usize)
}

fn be32(d: &[u8], o: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("the font is cut short")?.try_into()?) as usize)
}

fn p16(v: &mut Vec<u8>, x: usize) {
    v.extend((x as u16).to_le_bytes());
}

fn p32(v: &mut Vec<u8>, x: usize) {
    v.extend((x as u32).to_le_bytes());
}

fn put32(d: &mut [u8], o: usize, v: usize) {
    d[o..o + 4].copy_from_slice(&(v as u32).to_le_bytes());
}

fn pad(v: &mut Vec<u8>, to: usize) {
    while !v.len().is_multiple_of(to) {
        v.push(0);
    }
}

fn sheet_format(f: usize) -> Option<(u32, u32, [u8; 4])> {
    const ALPHA: [u8; 4] = [1, 1, 1, 2];
    const COLOUR: [u8; 4] = [2, 3, 4, 5];
    Some(match f {
        0 => (0x01a, 0x0b01, COLOUR),
        8 => (0x001, 0x0201, ALPHA),
        9 => (0x031, 0x1a01, COLOUR),
        10 => (0x032, 0x1b01, COLOUR),
        11 => (0x033, 0x1c01, COLOUR),
        12 => (0x034, 0x1d01, ALPHA),
        14 => (0x41a, 0x0b06, COLOUR),
        15 => (0x431, 0x1a06, COLOUR),
        16 => (0x432, 0x1b06, COLOUR),
        17 => (0x433, 0x1c06, COLOUR),
        _ => return None,
    })
}

pub fn to_switch(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < 0x14 || &raw[..4] != b"FFNT" || raw[4..6] != [0xfe, 0xff] || raw[8] != 3 {
        bail!("not a Wii U font (version 3)");
    }
    let header = be16(raw, 6)?;
    let count = be16(raw, 0x10)?;
    let mut sections = Vec::with_capacity(count);
    let mut o = header;
    for _ in 0..count {
        let size = be32(raw, o + 4)?;
        let sec = raw.get(o..o + size).context("a font section is cut short")?;
        sections.push((o, sec));
        o += size;
    }
    let mut out = raw[..header].to_vec();
    out[4..6].copy_from_slice(&[0xff, 0xfe]);
    out[6..8].reverse();
    out[8..12].copy_from_slice(&0x0401_0000u32.to_le_bytes());
    let mut moved: HashMap<usize, usize> = HashMap::new();
    let mut written = Vec::new();
    for &(o, sec) in &sections {
        let at = out.len();
        let new = match &sec[..4] {
            b"FINF" => {
                let mut v = sec[..0x20.min(sec.len())].to_vec();
                v.truncate(8);
                v.extend_from_slice(&sec[8..0xc]);
                p16(&mut v, be16(sec, 0xc)?);
                p16(&mut v, be16(sec, 0xe)?);
                v.extend_from_slice(&sec[0x10..0x14]);
                for f in [0x14, 0x18, 0x1c] {
                    p32(&mut v, be32(sec, f)?);
                }
                v
            }
            b"TGLP" => glyphs(sec, o, at)?,
            b"CWDH" => {
                let mut v = sec[..8].to_vec();
                p16(&mut v, be16(sec, 8)?);
                p16(&mut v, be16(sec, 0xa)?);
                p32(&mut v, be32(sec, 0xc)?);
                let n = (be16(sec, 0xa)? + 1).saturating_sub(be16(sec, 8)?);
                v.extend_from_slice(sec.get(0x10..0x10 + 3 * n).context("the widths are cut short")?);
                pad(&mut v, 4);
                v
            }
            b"CMAP" => char_map(sec)?,
            b"KRNG" => continue,
            m => bail!("font section {} isn't known", String::from_utf8_lossy(m)),
        };
        moved.insert(o + 8, at + 8);
        written.push((&sec[..4], at));
        out.extend(new);
        let n = out.len() - at;
        put32(&mut out, at + 4, n);
    }
    let map = |old: usize| -> Result<usize> {
        if old == 0 { Ok(0) } else { moved.get(&old).copied().context("an offset to no section") }
    };
    for &(magic, at) in &written {
        let fields: &[usize] = match magic {
            b"FINF" => &[0x14, 0x18, 0x1c],
            b"CWDH" => &[0xc],
            b"CMAP" => &[0x14],
            _ => &[],
        };
        for &f in fields {
            let old = u32::from_le_bytes(out[at + f..at + f + 4].try_into()?) as usize;
            let new = map(old)?;
            put32(&mut out, at + f, new);
        }
    }
    let n = out.len();
    put32(&mut out, 0xc, n);
    out[0x10..0x12].copy_from_slice(&(written.len() as u16).to_le_bytes());
    Ok(out)
}

fn glyphs(sec: &[u8], from: usize, at: usize) -> Result<Vec<u8>> {
    let sheets = sec[0xa] as usize;
    let (sheet_size, format, width, height, data) =
        (be32(sec, 0xc)?, be16(sec, 0x12)?, be16(sec, 0x18)? as u32, be16(sec, 0x1a)? as u32, be32(sec, 0x1c)?);
    let (gx2_format, bntx_format, channels) =
        sheet_format(format).with_context(|| format!("font sheet format {format} isn't supported yet"))?;
    let first = data.checked_sub(from).context("the glyph sheets are before their section")?;
    let mut layers = Vec::new();
    for i in 0..sheets {
        let image = sec
            .get(first + i * sheet_size..first + (i + 1) * sheet_size)
            .context("a glyph sheet is cut short")?
            .to_vec();
        let mut s = gx2::Surface {
            name: "0".into(),
            dim: 1,
            width,
            height,
            depth: 1,
            mips: 1,
            format: gx2_format,
            aa: 0,
            image_size: sheet_size as u32,
            mip_size: 0,
            tile_mode: 4,
            swizzle: 0,
            alignment: 0,
            pitch: 0,
            mip_offsets: [0; 13],
            comp_sel: [0, 1, 2, 3],
            image,
            mip_data: vec![],
            data_at: Some(0),
            mip_at: None,
            user_data: vec![],
        };
        s.pitch = gx2::level_info(&s, 0)?.pitch;
        let level = gx2::untile_all(&s, None)?.into_iter().next().context("a sheet without its image")?;
        layers.push(level);
    }
    let texture = bntx::Texture {
        name: "0".into(),
        format: bntx_format,
        width,
        height,
        channels,
        levels: vec![layers.concat()],
        layers: sheets as u32,
        array: true,
        user_data: vec![],
    };
    let tex = bntx::write("0", &[texture])?;
    let mut v = sec[..8].to_vec();
    v.extend_from_slice(&sec[8..0xc]);
    p32(&mut v, tex.len() / sheets.max(1));
    for f in [0x10, 0x12, 0x14, 0x16, 0x18, 0x1a] {
        p16(&mut v, be16(sec, f)?);
    }
    let start = (at + 0x20).div_ceil(0x1000) * 0x1000;
    p32(&mut v, start);
    v.resize(start - at, 0);
    v.extend(tex);
    Ok(v)
}

fn char_map(sec: &[u8]) -> Result<Vec<u8>> {
    let (first, last, method, next) = (be16(sec, 8)?, be16(sec, 0xa)?, be16(sec, 0xc)?, be32(sec, 0x10)?);
    let mut v = sec[..8].to_vec();
    p32(&mut v, first);
    p32(&mut v, last);
    p16(&mut v, method);
    p16(&mut v, be16(sec, 0xe)?);
    p32(&mut v, next);
    match method {
        0 => p16(&mut v, be16(sec, 0x14)?),
        1 => {
            for i in 0..(last + 1).saturating_sub(first) {
                p16(&mut v, be16(sec, 0x14 + 2 * i)?);
            }
        }
        2 => {
            let n = be16(sec, 0x14)?;
            p16(&mut v, n);
            p16(&mut v, 0);
            for i in 0..n {
                p32(&mut v, be16(sec, 0x16 + 4 * i)?);
                p16(&mut v, be16(sec, 0x18 + 4 * i)?);
                p16(&mut v, 0);
            }
        }
        m => bail!("character map method {m} isn't known"),
    }
    pad(&mut v, 4);
    Ok(v)
}
