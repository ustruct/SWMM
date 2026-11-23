use std::path::Path;

use anyhow_ext::{Context, Result};

use crate::tex::{bntx, bntx_of};

pub const COLUMNS: [char; 12] = ['Z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K'];
pub const ROWS: usize = 10;
pub const TILE_METERS: f32 = 1000.0;
pub const WORLD_MIN_X: f32 = -6000.0;
pub const WORLD_MIN_Z: f32 = -5000.0;
pub const WORLD_WIDTH: f32 = TILE_METERS * COLUMNS.len() as f32;
pub const WORLD_DEPTH: f32 = TILE_METERS * ROWS as f32;

const MAP_DIR: &str = "UI/MapTex/MainField";
const BC1_SRGB: u32 = 0x1a06;
const BC1_UNORM: u32 = 0x1a01;

pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 527 + 23) >> 6) as u8, ((g * 259 + 33) >> 6) as u8, ((b * 527 + 23) >> 6) as u8]
}

fn mix(a: [u8; 3], b: [u8; 3], wa: u32, wb: u32) -> [u8; 3] {
    let t = wa + wb;
    [
        ((a[0] as u32 * wa + b[0] as u32 * wb) / t) as u8,
        ((a[1] as u32 * wa + b[1] as u32 * wb) / t) as u8,
        ((a[2] as u32 * wa + b[2] as u32 * wb) / t) as u8,
    ]
}

pub fn decode_bc1(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let bw = width.div_ceil(4);
    let bh = height.div_ceil(4);
    if data.len() < bw * bh * 8 {
        anyhow_ext::bail!("BC1 data is cut short ({} of {} bytes)", data.len(), bw * bh * 8);
    }
    let mut out = vec![0u8; width * height * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let block = &data[(by * bw + bx) * 8..][..8];
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            let (a, b) = (rgb565(c0), rgb565(c1));
            let palette: [[u8; 4]; 4] = if c0 > c1 {
                let c = mix(a, b, 2, 1);
                let d = mix(a, b, 1, 2);
                [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], [c[0], c[1], c[2], 255], [d[0], d[1], d[2], 255]]
            } else {
                let c = mix(a, b, 1, 1);
                [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], [c[0], c[1], c[2], 255], [0, 0, 0, 0]]
            };
            let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
            for py in 0..4 {
                let y = by * 4 + py;
                if y >= height {
                    break;
                }
                for px in 0..4 {
                    let x = bx * 4 + px;
                    if x >= width {
                        continue;
                    }
                    let index = ((bits >> (2 * (py * 4 + px))) & 3) as usize;
                    out[(y * width + x) * 4..][..4].copy_from_slice(&palette[index]);
                }
            }
        }
    }
    Ok(out)
}

fn tile_file(level: &str, column: char, row: usize) -> String {
    format!("MapTex{level}_{column}-{row}.sbmaptex")
}

pub fn read_tile(romfs: &Path, level: &str, column: char, row: usize) -> Result<Image> {
    let path = romfs.join(MAP_DIR).join(tile_file(level, column, row));
    let raw = std::fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?;
    let data = roead::yaz0::decompress_if(&raw);
    let textures = bntx::parse(bntx_of(&data)?)?;
    let texture = textures.first().context("The map tile has no texture")?;
    if texture.format != BC1_SRGB && texture.format != BC1_UNORM {
        anyhow_ext::bail!("The map tile uses an unexpected format ({:#x})", texture.format);
    }
    let level = texture.levels.first().context("The map tile has no image")?;
    let (width, height) = (texture.width as usize, texture.height as usize);
    Ok(Image {
        width,
        height,
        rgba: decode_bc1(level, width, height)?,
    })
}

fn scale_into(tile: &Image, out: &mut Image, left: usize, top: usize, size: usize) {
    for y in 0..size {
        let sy = y * tile.height / size;
        for x in 0..size {
            let sx = x * tile.width / size;
            let src = &tile.rgba[(sy * tile.width + sx) * 4..][..4];
            let at = ((top + y) * out.width + left + x) * 4;
            out.rgba[at..at + 4].copy_from_slice(src);
        }
    }
}

pub fn build(romfs: &Path, tile_size: usize) -> Result<Image> {
    let level = if tile_size <= 135 {
        "3"
    } else if tile_size <= 300 {
        "2"
    } else if tile_size <= 840 {
        "1"
    } else {
        ""
    };
    let mut out = Image {
        width: tile_size * COLUMNS.len(),
        height: tile_size * ROWS,
        rgba: vec![0u8; tile_size * COLUMNS.len() * tile_size * ROWS * 4],
    };
    for (col, column) in COLUMNS.iter().enumerate() {
        for row in 0..ROWS {
            let tile = read_tile(romfs, level, *column, row)?;
            scale_into(&tile, &mut out, col * tile_size, row * tile_size, tile_size);
        }
    }
    Ok(out)
}

const TERRAIN_INDEX: &str = "Terrain/A/MainField.tscb";
const TERRAIN_DIR: &str = "Terrain/A/MainField";
const HGHT_SIDE: usize = 256;

#[derive(Debug, Clone)]
struct Area {
    x: f32,
    z: f32,
    size: f32,
    name: String,
}

pub struct Terrain {
    romfs: std::path::PathBuf,
    scale: f32,
    max_height: f32,
    areas: Vec<Area>,
    cache: std::collections::HashMap<String, Option<std::sync::Arc<Vec<u16>>>>,
}

fn be_u32(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("The terrain index is cut short")?.try_into()?))
}

fn be_f32(d: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_bits(be_u32(d, o)?))
}

fn c_string(d: &[u8], o: usize) -> Result<String> {
    let tail = d.get(o..).context("The terrain index is cut short")?;
    let end = tail.iter().position(|b| *b == 0).context("The terrain index has a broken name")?;
    Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
}

fn pack_name(name: &str) -> Option<String> {
    let id = u64::from_str_radix(name, 16).ok()?;
    Some(format!("{:0width$X}", id & !3, width = name.len()))
}

impl Terrain {
    pub fn open(romfs: &Path) -> Result<Self> {
        let path = romfs.join(TERRAIN_INDEX);
        let d = std::fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?;
        if d.get(0..4) != Some(b"TSCB".as_slice()) {
            anyhow_ext::bail!("{} isn't a terrain index", path.display());
        }
        let scale = be_f32(&d, 0x10)?;
        let max_height = be_f32(&d, 0x14)?;
        let materials = be_u32(&d, 0x18)? as usize;
        let count = be_u32(&d, 0x1c)? as usize;
        let table = 0x34 + materials * 4 + materials * 0x14;
        if 0x30 + be_u32(&d, 0x30)? as usize != table {
            anyhow_ext::bail!("The terrain index has an unexpected layout");
        }
        let mut areas = Vec::with_capacity(count);
        for i in 0..count {
            let at = table + 4 * i;
            let o = at + be_u32(&d, at)? as usize;
            let name_at = o + 0x20 + be_u32(&d, o + 0x20)? as usize;
            areas.push(Area {
                x: be_f32(&d, o)?,
                z: be_f32(&d, o + 4)?,
                size: be_f32(&d, o + 8)?,
                name: c_string(&d, name_at)?,
            });
        }
        areas.sort_by(|a, b| a.size.total_cmp(&b.size));
        Ok(Self {
            romfs: romfs.to_path_buf(),
            scale,
            max_height,
            areas,
            cache: Default::default(),
        })
    }

    fn heights(&mut self, name: &str) -> Option<std::sync::Arc<Vec<u16>>> {
        if let Some(hit) = self.cache.get(name) {
            return hit.clone();
        }
        let found = (|| {
            let pack = pack_name(name)?;
            let raw = std::fs::read(self.romfs.join(TERRAIN_DIR).join(format!("{pack}.hght.sstera"))).ok()?;
            let data = roead::yaz0::decompress_if(&raw);
            let sarc = roead::sarc::Sarc::new(&*data).ok()?;
            let file = sarc.get_data(&format!("{name}.hght"))?;
            if file.len() < HGHT_SIDE * HGHT_SIDE * 2 {
                return None;
            }
            let values = (0..HGHT_SIDE * HGHT_SIDE)
                .map(|i| u16::from_le_bytes([file[2 * i], file[2 * i + 1]]))
                .collect::<Vec<_>>();
            Some(std::sync::Arc::new(values))
        })();
        self.cache.insert(name.to_string(), found.clone());
        found
    }

    pub fn height_at(&mut self, x: f32, z: f32) -> Option<f32> {
        let gx = x / self.scale;
        let gz = z / self.scale;
        let candidates: Vec<Area> = self
            .areas
            .iter()
            .filter(|a| (gx - a.x).abs() <= a.size / 2.0 && (gz - a.z).abs() <= a.size / 2.0)
            .cloned()
            .collect();
        for area in candidates {
            let Some(values) = self.heights(&area.name) else {
                continue;
            };
            let last = (HGHT_SIDE - 1) as f32;
            let fx = ((gx - (area.x - area.size / 2.0)) / area.size * last).clamp(0.0, last);
            let fz = ((gz - (area.z - area.size / 2.0)) / area.size * last).clamp(0.0, last);
            let (x0, z0) = (fx.floor() as usize, fz.floor() as usize);
            let (x1, z1) = ((x0 + 1).min(HGHT_SIDE - 1), (z0 + 1).min(HGHT_SIDE - 1));
            let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
            let at = |xx: usize, zz: usize| values[zz * HGHT_SIDE + xx] as f32;
            let top = at(x0, z0) * (1.0 - tx) + at(x1, z0) * tx;
            let bottom = at(x0, z1) * (1.0 - tx) + at(x1, z1) * tx;
            let raw = top * (1.0 - tz) + bottom * tz;
            return Some(raw / 65536.0 * self.max_height);
        }
        None
    }
}

pub fn world_to_map(x: f32, z: f32) -> (f32, f32) {
    ((x - WORLD_MIN_X) / WORLD_WIDTH, (z - WORLD_MIN_Z) / WORLD_DEPTH)
}

pub fn map_to_world(u: f32, v: f32) -> (f32, f32) {
    (WORLD_MIN_X + u.clamp(0.0, 1.0) * WORLD_WIDTH, WORLD_MIN_Z + v.clamp(0.0, 1.0) * WORLD_DEPTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_block() {
        let block = [0x00, 0xf8, 0x1f, 0x00, 0b0100_0100, 0, 0, 0];
        let rgba = decode_bc1(&block, 4, 4).unwrap();
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 0, 255, 255]);
        assert_eq!(&rgba[16..20], &[255, 0, 0, 255]);
        assert!(decode_bc1(&block, 8, 8).is_err());
    }

    #[test]
    fn real_game_files() {
        let Some(romfs) = std::env::var_os("SWMM_ROMFS").map(std::path::PathBuf::from) else {
            return;
        };
        let image = build(&romfs, 135).unwrap();
        assert_eq!((image.width, image.height), (1620, 1350));
        let (u, v) = world_to_map(-254.0, -611.0);
        let at = ((v * image.height as f32) as usize * image.width + (u * image.width as f32) as usize) * 4;
        assert_eq!(image.rgba[at + 3], 255);
        let mut terrain = Terrain::open(&romfs).unwrap();
        let ground = terrain.height_at(-4673.53, 1976.99).unwrap();
        assert!((ground - 132.13).abs() < 4.0, "{ground}");
        assert!(terrain.height_at(0.0, 0.0).is_some());
        assert_eq!(pack_name("560000087B").as_deref(), Some("5600000878"));
    }

    #[test]
    fn coordinates() {
        assert_eq!(world_to_map(-6000.0, -5000.0), (0.0, 0.0));
        assert_eq!(world_to_map(0.0, 0.0), (0.5, 0.5));
        assert_eq!(map_to_world(1.0, 1.0), (6000.0, 5000.0));
        assert_eq!(map_to_world(2.0, -1.0), (6000.0, -5000.0));
    }
}
