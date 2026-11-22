use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use roead::{sarc::Sarc, yaz0};

static TABLE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/wiiu/stock.txt.zst"));

fn table() -> &'static HashMap<(u32, u32), String> {
    static T: OnceLock<HashMap<(u32, u32), String>> = OnceLock::new();
    T.get_or_init(|| {
        let text = zstd::decode_all(TABLE).ok().and_then(|d| String::from_utf8(d).ok()).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let mut it = l.splitn(3, ' ');
                let crc = u32::from_str_radix(it.next()?, 16).ok()?;
                let size = u32::from_str_radix(it.next()?, 16).ok()?;
                Some(((crc, size), it.next()?.to_string()))
            })
            .collect()
    })
}

pub fn crc32(data: &[u8]) -> u32 {
    static T: OnceLock<[u32; 256]> = OnceLock::new();
    let t = T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8))
}

pub fn switch_twin(raw: &[u8]) -> Option<&'static str> {
    table().get(&(crc32(raw), raw.len() as u32)).map(String::as_str).filter(|p| *p != "-")
}

pub fn wiiu_only(raw: &[u8]) -> bool {
    table().get(&(crc32(raw), raw.len() as u32)).is_some_and(|p| p == "-")
}

pub fn is_game_file(raw: &[u8]) -> bool {
    table().contains_key(&(crc32(raw), raw.len() as u32))
}

#[derive(Default)]
pub struct Game {
    pub romfs: Option<PathBuf>,
    archives: HashMap<String, Vec<u8>>,
    sizes: Option<Option<crate::actor_size::Sizes>>,
}

impl std::fmt::Debug for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Game").field("romfs", &self.romfs).finish()
    }
}

impl Clone for Game {
    fn clone(&self) -> Self {
        Self { romfs: self.romfs.clone(), archives: HashMap::new(), sizes: None }
    }
}

fn plain(d: Vec<u8>) -> Option<Vec<u8>> {
    if d.starts_with(b"Yaz0") { yaz0::decompress(&d).ok() } else { Some(d) }
}

impl Game {
    pub fn new(romfs: Option<&Path>) -> Self {
        Self { romfs: romfs.map(Path::to_path_buf), archives: HashMap::new(), sizes: None }
    }

    pub fn actor_sizes(&mut self) -> Option<&crate::actor_size::Sizes> {
        if self.sizes.is_none() {
            let info = self.read("Actor/ActorInfo.product.sbyml");
            self.sizes = Some(info.and_then(|d| crate::actor_size::Sizes::new(&d)));
        }
        self.sizes.as_ref().and_then(|s| s.as_ref())
    }

    fn aoc_root(romfs: &Path) -> Option<PathBuf> {
        let parent = romfs.parent()?;
        [parent.join("aoc1"), parent.join("aoc"), parent.parent()?.join("DLC/romfs")]
            .into_iter()
            .find(|d| d.join("Pack").is_dir() || d.join("Map").is_dir())
    }

    pub fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let romfs = self.romfs.clone()?;
        let (romfs, path) = match path.strip_prefix("aoc/") {
            Some(rest) => (Self::aoc_root(&romfs)?, rest),
            None => (romfs, path),
        };
        let parts: Vec<&str> = path.split("//").collect();
        let (last, archives) = parts.split_last()?;
        if archives.is_empty() {
            return plain(std::fs::read(romfs.join(last)).ok()?);
        }
        let mut key = String::new();
        for (i, a) in archives.iter().enumerate() {
            let outer = key.clone();
            if !key.is_empty() {
                key.push_str("//");
            }
            key.push_str(a);
            if !self.archives.contains_key(&key) {
                let d = if i == 0 {
                    plain(std::fs::read(romfs.join(a)).ok()?)?
                } else {
                    let parent = self.archives.get(&outer)?;
                    plain(Sarc::new(parent.as_slice()).ok()?.get_data(a)?.to_vec())?
                };
                if self.archives.len() > 64 {
                    self.archives.clear();
                }
                self.archives.insert(key.clone(), d);
            }
        }
        let sarc = Sarc::new(self.archives.get(&key)?.as_slice()).ok()?;
        plain(sarc.get_data(last)?.to_vec())
    }
}

pub fn twins() -> Vec<&'static str> {
    let mut v: Vec<&str> = table().values().map(String::as_str).filter(|p| *p != "-").collect();
    v.sort();
    v
}
