use anyhow_ext::{Context, Result, bail};

use super::{bntx, gx2};

fn format(flim: u8, linear: bool) -> Option<(u32, u32, [u8; 4])> {
    const RGBA: [u8; 4] = [2, 3, 4, 5];
    const LUM: [u8; 4] = [2, 2, 2, 1];
    const ALPHA: [u8; 4] = [1, 1, 1, 2];
    Some(match flim {
        0 => (0x01, 0x0201, LUM),
        1 => (0x01, 0x0201, ALPHA),
        3 => (0x07, 0x0901, [2, 2, 2, 3]),
        5 => (0x08, 0x0701, [2, 3, 4, 1]),
        9 => (0x1a, 0x0b01, RGBA),
        12 if linear => (0x31, 0x1a01, [2, 3, 0, 4]),
        12 => (0x31, 0x1a01, RGBA),
        13 => (0x32, 0x1b01, RGBA),
        14 => (0x33, 0x1c01, RGBA),
        15 => (0x34, 0x1d01, LUM),
        16 => (0x34, 0x1d01, ALPHA),
        17 if linear => (0x35, 0x1e01, [2, 3, 0, 1]),
        17 => (0x35, 0x1e01, [2, 2, 2, 3]),
        20 => (0x41a, 0x0b06, RGBA),
        21 => (0x431, 0x1a06, RGBA),
        22 => (0x432, 0x1b06, RGBA),
        23 => (0x433, 0x1c06, RGBA),
        24 => (0x19, 0x0e01, RGBA),
        _ => return None,
    })
}

pub fn to_texture(name: &str, data: &[u8]) -> Result<bntx::Texture> {
    let foot = data.len().checked_sub(0x28).context("too short for a FLIM texture")?;
    let f = &data[foot..];
    if &f[..4] != b"FLIM" || f[4..6] != [0xfe, 0xff] || &f[0x14..0x18] != b"imag" {
        bail!("not a Wii U FLIM texture");
    }
    let u16_at = |o: usize| u16::from_be_bytes([f[o], f[o + 1]]) as u32;
    let (width, height) = (u16_at(0x1c), u16_at(0x1e));
    let (fmt, tile) = (f[0x22], f[0x23]);
    let image_size = u32::from_be_bytes(f[0x24..0x28].try_into()?);
    let marker = name.rfind(['^', '+']).map(|i| name.as_bytes()[i]);
    let (gx2_format, format, channels) =
        format(fmt, marker == Some(b'+')).with_context(|| format!("FLIM format {fmt} isn't supported yet"))?;
    let mut s = gx2::Surface {
        name: name.to_string(),
        dim: 1,
        width,
        height,
        depth: 1,
        mips: 1,
        format: gx2_format,
        aa: 0,
        image_size,
        mip_size: 0,
        tile_mode: (tile & 0x1f) as u32,
        swizzle: ((tile >> 5) as u32) << 8,
        alignment: 0,
        pitch: 0,
        mip_offsets: [0; 13],
        comp_sel: [0, 1, 2, 3],
        image: data[..foot].to_vec(),
        mip_data: vec![],
        data_at: Some(0),
        mip_at: None,
        user_data: vec![],
    };
    s.pitch = gx2::level_info(&s, 0)?.pitch;
    let mut levels = gx2::untile_all(&s, None)?;
    if fmt == 5 {
        for l in &mut levels {
            for px in l.chunks_mut(2) {
                let v = u16::from_le_bytes([px[0], px[1]]);
                let v = (v >> 11) | (v & 0x07e0) | (v << 11);
                px.copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(bntx::Texture {
        name: name.to_string(),
        format,
        width,
        height,
        channels,
        levels,
        layers: 1,
        array: false,
        user_data: vec![],
    })
}

pub fn combined(images: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut textures = images
        .iter()
        .map(|(n, d)| to_texture(n, d).with_context(|| n.clone()))
        .collect::<Result<Vec<_>>>()?;
    textures.sort_by(|a, b| a.name.cmp(&b.name));
    bntx::write("__Combined", &textures)
}
