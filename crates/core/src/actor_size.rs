use std::collections::HashMap;

use roead::byml::Byml;

static WIIU: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/wiiu/actor_instsize.bin"));

fn wiiu_sizes() -> &'static HashMap<u32, i32> {
    static T: std::sync::OnceLock<HashMap<u32, i32>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        WIIU.as_chunks::<8>().0.iter()
            .map(|c| (u32::from_le_bytes([c[0], c[1], c[2], c[3]]), i32::from_le_bytes([c[4], c[5], c[6], c[7]])))
            .collect()
    })
}

#[derive(Default)]
pub struct Sizes {
    switch: HashMap<u32, (i32, String, String)>,
    by_model: HashMap<String, f64>,
    by_profile: HashMap<String, f64>,
    most: f64,
}

fn text(b: &Byml) -> Option<String> {
    match b {
        Byml::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn int(b: &Byml) -> Option<i32> {
    match b {
        Byml::I32(v) => Some(*v),
        Byml::U32(v) => Some(*v as i32),
        _ => None,
    }
}

impl Sizes {
    pub fn new(switch_info: &[u8]) -> Option<Self> {
        let b = Byml::from_binary(switch_info).ok()?;
        let Byml::Map(root) = &b else { return None };
        let Some(Byml::Array(actors)) = root.get("Actors") else { return None };
        let mut s = Sizes::default();
        for a in actors {
            let Byml::Map(m) = a else { continue };
            let (Some(name), Some(size)) = (m.get("name").and_then(text), m.get("instSize").and_then(int)) else {
                continue;
            };
            let model = m.get("bfres").and_then(text).unwrap_or_default();
            let profile = m.get("profile").and_then(text).unwrap_or_default();
            let hash = crc32(name.as_bytes());
            if let Some(&w) = wiiu_sizes().get(&hash).filter(|w| **w > 0) {
                let r = size as f64 / w as f64;
                for (map, key) in [(&mut s.by_model, &model), (&mut s.by_profile, &profile)] {
                    if !key.is_empty() {
                        let e = map.entry(key.clone()).or_insert(r);
                        *e = e.max(r);
                    }
                }
                s.most = s.most.max(r);
            }
            s.switch.insert(hash, (size, model, profile));
        }
        Some(s)
    }

    pub fn switch_size(&self, hash: u32, size: i32, model: Option<&str>, profile: Option<&str>) -> i32 {
        let game = self.switch.get(&hash);
        let wiiu = wiiu_sizes().get(&hash).copied();
        if let (Some(g), Some(w)) = (game, wiiu)
            && w == size {
                return g.0;
            }
        let model = model.map(str::to_string).or_else(|| game.map(|g| g.1.clone())).unwrap_or_default();
        let profile = profile.map(str::to_string).or_else(|| game.map(|g| g.2.clone())).unwrap_or_default();
        let own = match (game, wiiu) {
            (Some(g), Some(w)) if w > 0 => Some(g.0 as f64 / w as f64),
            _ => None,
        };
        let ratio = own
            .or_else(|| self.by_model.get(&model).copied())
            .or_else(|| self.by_profile.get(&profile).copied())
            .unwrap_or(if self.most > 0.0 { self.most } else { 2.0 });
        let v = (size as f64 * ratio).ceil() as i64;
        ((v + 7) / 8 * 8).min(i32::MAX as i64) as i32
    }

    pub fn convert_entry(&self, hash: u32, entry: &mut Byml) {
        let Byml::Map(m) = entry else { return };
        let Some(size) = m.get("instSize").and_then(int) else { return };
        let model = m.get("bfres").and_then(text);
        let profile = m.get("profile").and_then(text);
        let new = self.switch_size(hash, size, model.as_deref(), profile.as_deref());
        m.insert("instSize".into(), Byml::I32(new));
    }

    pub fn convert_info(&self, info: &mut Byml) {
        let Byml::Map(root) = info else { return };
        let Some(Byml::Array(actors)) = root.get_mut("Actors") else { return };
        for a in actors {
            let hash = match a {
                Byml::Map(m) => m.get("name").and_then(text).map(|n| crc32(n.as_bytes())),
                _ => None,
            };
            if let Some(h) = hash {
                self.convert_entry(h, a);
            }
        }
    }
}

fn crc32(d: &[u8]) -> u32 {
    crate::stock::crc32(d)
}
