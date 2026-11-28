pub mod model;
pub mod switch;
pub mod wiiu;

pub const NEUTRAL_BLACK: &str = "SwmmNeutral_Black";
pub const NEUTRAL_WHITE: &str = "SwmmNeutral_White";
pub const NEUTRAL_NORMAL: &str = "SwmmNeutral_Normal";

fn neutral_for(shader_sampler: &str) -> &'static str {
    let s = shader_sampler.trim_start_matches('_').to_ascii_lowercase();
    if s.starts_with("ao") {
        NEUTRAL_WHITE
    } else if s.starts_with('n') && s[1..].chars().all(|c| c.is_ascii_digit()) {
        NEUTRAL_NORMAL
    } else {
        NEUTRAL_BLACK
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SamplerFix {
    pub material: String,
    pub shader_sampler: String,
    pub missing: String,
    pub texture: Option<&'static str>,
}

pub fn fix_dangling_samplers(file: &mut model::File, neutral_textures: bool) -> Vec<SamplerFix> {
    let mut fixes = Vec::new();
    for model in &mut file.models {
        for mat in &mut model.materials {
            let Some(state) = mat.samplers.first().map(|s| s.1) else { continue };
            for (shader_sampler, target) in mat.shader_assign.samplers.iter_mut() {
                if mat.samplers.iter().any(|(n, _)| n == target) {
                    continue;
                }
                let mut fix = SamplerFix {
                    material: mat.name.clone(),
                    shader_sampler: shader_sampler.clone(),
                    missing: target.clone(),
                    texture: None,
                };
                if neutral_textures && mat.textures.len() == mat.samplers.len() {
                    let texture = neutral_for(shader_sampler);
                    mat.samplers.push((target.clone(), state));
                    mat.textures.push(texture.to_string());
                    fix.texture = Some(texture);
                } else {
                    *target = mat.samplers[0].0.clone();
                }
                fixes.push(fix);
            }
        }
    }
    fixes
}

pub fn repair_switch_samplers(raw: &[u8]) -> Option<(Vec<u8>, Vec<String>)> {
    if raw.get(..4) != Some(b"FRES") || raw.get(0xc..0xe) != Some(&[0xff, 0xfe]) {
        return None;
    }
    let u16_at = |o: usize| raw.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| raw.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let u64_at = |o: usize| raw.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as usize);
    let string = |o: usize| -> Option<&[u8]> {
        let n = u16_at(o)?;
        raw.get(o + 2..o + 2 + n)
    };
    let dict = |o: usize| -> Option<Vec<usize>> {
        if o == 0 {
            return Some(vec![]);
        }
        let n = u32_at(o + 4)?;
        (1..=n).map(|k| u64_at(o + 8 + 16 * k + 8)).collect()
    };
    let mut out = raw.to_vec();
    let mut fixed = Vec::new();
    let models = u64_at(0x28)?;
    for mi in 0..u16_at(0xbc)? {
        let m = models + 0x78 * mi;
        let mats = u64_at(m + 0x40)?;
        for i in 0..u16_at(m + 0x6c)? {
            let o = mats + 0xb8 * i;
            let names = dict(u64_at(o + 0x50)?)?;
            let Some(&first) = names.first() else { continue };
            let have: Vec<&[u8]> = names.iter().filter_map(|&p| string(p)).collect();
            let sa = u64_at(o + 0x28)?;
            if sa == 0 {
                continue;
            }
            let keys = dict(u64_at(sa + 0x28)?)?;
            let values = u64_at(sa + 0x20)?;
            for (j, &key) in keys.iter().enumerate() {
                let slot = values + 8 * j;
                let target = u64_at(slot)?;
                if have.contains(&string(target)?) {
                    continue;
                }
                out[slot..slot + 8].copy_from_slice(&(first as u64).to_le_bytes());
                fixed.push(format!(
                    "{} -> {} (was {})",
                    String::from_utf8_lossy(string(key)?),
                    String::from_utf8_lossy(string(first)?),
                    String::from_utf8_lossy(string(target)?)
                ));
            }
        }
    }
    (!fixed.is_empty()).then_some((out, fixed))
}

pub fn repair_switch_models(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut repaired = 0;
    for path in entries.flatten().map(|e| e.path()) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".sbfres") || name.contains(".Tex") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else { continue };
        let compressed = data.starts_with(b"Yaz0");
        let Ok(raw) = (if compressed { roead::yaz0::decompress(&data).map_err(|_| ()) } else { Ok(data) }) else {
            continue;
        };
        if let Some((fixed, what)) = repair_switch_samplers(&raw) {
            let bytes = if compressed { roead::yaz0::compress(&fixed) } else { fixed };
            if std::fs::write(&path, bytes).is_ok() {
                repaired += 1;
                log::info!(
                    "Model/{name}: material textures that don't exist were pointed at ones that do, so the game doesn't crash: {}",
                    what.join(", ")
                );
            }
        }
    }
    repaired
}

pub fn relocation_entries(positions: &[usize]) -> Vec<(u32, u16, u8, u8)> {
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    let mut i = 0;
    while i < positions.len() {
        let mut n = 1;
        while i + n < positions.len() && positions[i + n] == positions[i] + 8 * n && n < 255 {
            n += 1;
        }
        runs.push((positions[i], n, false));
        i += n;
    }
    let mut out = Vec::new();
    for k in 0..runs.len() {
        if runs[k].2 {
            continue;
        }
        runs[k].2 = true;
        let (p, n) = (runs[k].0, runs[k].1);
        let (mut count, mut stride) = (1usize, None::<usize>);
        for run in runs.iter_mut().skip(k + 1) {
            let (q, m, used) = *run;
            if used || m != n {
                continue;
            }
            let s = match stride {
                Some(s) => s,
                None => {
                    let s = q - p;
                    if s % 8 != 0 || s / 8 < n || s / 8 - n > 255 {
                        break;
                    }
                    stride = Some(s);
                    s
                }
            };
            if q == p + s * count && count < 0xffff {
                run.2 = true;
                count += 1;
            } else if q > p + s * count {
                break;
            }
        }
        let pad = stride.map_or(0, |s| s / 8 - n);
        out.push((p as u32, count as u16, n as u8, pad as u8));
    }
    out.sort_unstable();
    out
}
