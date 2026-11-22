use anyhow_ext::{Context, Result, bail};

use super::model::*;

struct R<'a> {
    d: &'a [u8],
    version: u32,
}

impl<'a> R<'a> {
    fn u8(&self, o: usize) -> Result<u8> {
        self.d.get(o).copied().context("Wii U model is cut short")
    }
    fn u16(&self, o: usize) -> Result<u16> {
        Ok(u16::from_be_bytes(self.d.get(o..o + 2).context("Wii U model is cut short")?.try_into()?))
    }
    fn u32(&self, o: usize) -> Result<u32> {
        Ok(u32::from_be_bytes(self.d.get(o..o + 4).context("Wii U model is cut short")?.try_into()?))
    }
    fn i32(&self, o: usize) -> Result<i32> {
        Ok(self.u32(o)? as i32)
    }
    fn f32(&self, o: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32(o)?))
    }
    fn rel(&self, o: usize) -> Result<Option<usize>> {
        let v = self.i32(o)?;
        Ok((v != 0).then(|| (o as i64 + v as i64) as usize))
    }
    fn must(&self, o: usize, what: &str) -> Result<usize> {
        self.rel(o)?.with_context(|| format!("{what} is missing"))
    }
    fn bytes(&self, o: usize, n: usize) -> Result<&'a [u8]> {
        self.d.get(o..o + n).context("Wii U model data is cut short")
    }
    fn str_at(&self, o: usize) -> Result<String> {
        let rest = self.d.get(o..).context("string past the end")?;
        let n = rest.iter().position(|b| *b == 0).context("string without its end")?;
        Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
    }
    fn wide_name(&self, field: usize) -> Result<String> {
        let Some(o) = self.rel(field)? else { return Ok(String::new()) };
        let mut units = Vec::new();
        for i in 0.. {
            match self.u16(o + 2 * i)? {
                0 => break,
                u => units.push(u),
            }
        }
        Ok(String::from_utf16_lossy(&units))
    }
    fn name(&self, field: usize) -> Result<String> {
        match self.rel(field)? {
            Some(o) => self.str_at(o),
            None => Ok(String::new()),
        }
    }
    fn group(&self, field: usize) -> Result<Vec<(String, Option<usize>)>> {
        let Some(g) = self.rel(field)? else { return Ok(vec![]) };
        let n = self.u32(g + 4)? as usize;
        (1..=n)
            .map(|i| {
                let e = g + 8 + 16 * i;
                Ok((self.name(e + 8)?, self.rel(e + 12)?))
            })
            .collect()
    }
    fn f32s(&self, o: usize, n: usize) -> Result<Vec<f32>> {
        (0..n).map(|i| self.f32(o + 4 * i)).collect()
    }
    fn u16s(&self, o: usize, n: usize) -> Result<Vec<u16>> {
        (0..n).map(|i| self.u16(o + 2 * i)).collect()
    }
}

const GROUP_MODELS: usize = 0;
const GROUP_TEXTURES: usize = 1;
const GROUP_TEX_PATTERN: usize = 6;
const GROUP_EXTERNAL: usize = 11;

const MATERIAL_ANIMS: [(usize, &str); 4] = [(3, "_fsp"), (4, "_fcl"), (5, "_fts"), (GROUP_TEX_PATTERN, "_ftp")];
const GROUP_SKELETAL: usize = 2;
const GROUP_BONE_VIS: usize = 7;
const GROUP_MAT_VIS: usize = 8;
const GROUP_SCENE: usize = 10;
const UNSUPPORTED_ANIMS: [(usize, &str); 1] = [
    (9, "shape animations"),
];

fn read_curve(r: &R, c: usize) -> Result<Curve> {
    let flags = r.u16(c)?;
    let n = r.u16(c + 2)? as usize;
    let frame_size = match flags & 3 {
        0 => 4,
        1 => 2,
        2 => 1,
        _ => bail!("curve frame type {flags:#x} isn't known"),
    };
    let key_size = match (flags >> 2) & 3 {
        0 => 4,
        1 => 2,
        2 => 1,
        _ => bail!("curve key type {flags:#x} isn't known"),
    };
    let (key_bytes, key_size) = match (flags >> 4) & 7 {
        0 => (n * 4 * key_size, key_size),
        1 => (n * 2 * key_size, key_size),
        2..=5 => (n * key_size, key_size),
        _ => (n.div_ceil(32) * 4, 4),
    };
    let swap = |data: &[u8], size: usize| -> Vec<u8> {
        data.chunks(size).flat_map(|e| e.iter().rev().copied().collect::<Vec<_>>()).collect()
    };
    let frames = swap(r.bytes(r.must(c + 0x1c, "curve frames")?, n * frame_size)?, frame_size);
    let mut keys = swap(r.bytes(r.must(c + 0x20, "curve keys")?, key_bytes)?, key_size);
    if (flags >> 4) & 7 >= 6 {
        keys.resize(keys.len().max(4 * n), 0);
    }
    Ok(Curve {
        flags,
        key_count: n as u16,
        target: r.u32(c + 4)?,
        start: r.f32(c + 8)?,
        end: r.f32(c + 0xc)?,
        scale: r.u32(c + 0x10)?,
        offset: r.u32(c + 0x14)?,
        delta: r.f32(c + 0x18)?,
        frames,
        keys,
    })
}

fn read_curves(r: &R, field: usize, n: usize) -> Result<Vec<Curve>> {
    if n == 0 {
        return Ok(vec![]);
    }
    let at = r.must(field, "curves")?;
    (0..n).map(|i| read_curve(r, at + 0x24 * i).with_context(|| format!("curve {i}"))).collect()
}

fn read_param_anim(r: &R, a: usize, name: &str) -> Result<MaterialAnim> {
    if r.bytes(a, 4)? != b"FSHU" {
        bail!("{name} isn't an FSHU section");
    }
    let n_mat = r.u16(a + 0x14)? as usize;
    let mats_at = if n_mat > 0 { r.must(a + 0x2c, "material animations")? } else { 0 };
    let mut materials = Vec::with_capacity(n_mat);
    for i in 0..n_mat {
        let m = mats_at + 0x20 * i;
        let (n_param, n_curve, n_const) = (r.u16(m)? as usize, r.u16(m + 2)? as usize, r.u16(m + 4)? as usize);
        let params = match n_param {
            0 => vec![],
            _ => {
                let p = r.must(m + 0x14, "param animations")?;
                (0..n_param)
                    .map(|k| {
                        let o = p + 0x10 * k;
                        Ok(ParamAnimInfo {
                            name: r.name(o + 0xc)?,
                            begin_curve: r.u16(o)?,
                            float_curves: r.u16(o + 2)?,
                            int_curves: r.u16(o + 4)?,
                            begin_constant: r.u16(o + 6)?,
                            constants: r.u16(o + 8)?,
                            sub_bind: r.u16(o + 0xa)?,
                        })
                    })
                    .collect::<Result<_>>()?
            }
        };
        let constants = match n_const {
            0 => vec![],
            _ => {
                let c = r.must(m + 0x1c, "constants")?;
                (0..n_const).map(|k| Ok((r.u32(c + 8 * k)?, r.u32(c + 8 * k + 4)?))).collect::<Result<_>>()?
            }
        };
        materials.push(MaterialAnimData {
            name: r.name(m + 0x10)?,
            param_curve_begin: r.i32(m + 8)? as i16,
            pattern_curve_begin: -1,
            visibility: [-1; 3],
            params,
            patterns: vec![],
            curves: read_curves(r, m + 0x18, n_curve)?,
            constants,
        });
    }
    Ok(MaterialAnim {
        name: name.to_string(),
        path: r.name(a + 8)?,
        flags: r.u32(a + 0xc)?,
        frames: r.i32(a + 0x10)?,
        baked_size: switch_baked_size(r.u32(a + 0x20)?, materials.iter().flat_map(|m| &m.curves)),
        bind_indices: bind_indices(r, a + 0x28, n_mat)?,
        param_anim_count: r.u32(a + 0x18)? as u16,
        pattern_anim_count: 0,
        textures: vec![],
        materials,
        user_data: read_user_data(r, a + 0x30)?,
    })
}

fn baked_sizes(c: &Curve) -> (u32, u32) {
    let frames = (c.end - c.start).floor().max(0.0) as u32 + 1;
    let wiiu = match (c.flags >> 4) & 7 {
        0..=3 => 4 * frames + 20,
        4 | 5 => frames + 8,
        _ => frames.div_ceil(8) + 8,
    };
    (wiiu, (wiiu + 8).div_ceil(8) * 8)
}

fn switch_baked_size<'a>(wiiu: u32, curves: impl Iterator<Item = &'a Curve>) -> u32 {
    if wiiu == 0 {
        return 0;
    }
    curves.map(|c| baked_sizes(c).1).sum()
}

fn read_skeletal_anim(r: &R, a: usize, name: &str) -> Result<SkeletalAnim> {
    if r.bytes(a, 4)? != b"FSKA" {
        bail!("{name} isn't an FSKA section");
    }
    let n = r.u16(a + 0x14)? as usize;
    let bones_at = if n > 0 { r.must(a + 0x20, "bone animations")? } else { 0 };
    let mut bones = Vec::with_capacity(n);
    for i in 0..n {
        let b = bones_at + 0x18 * i;
        let flags = r.u32(b)?;
        let name = r.name(b + 4)?;
        let count: usize =
            [(1 << 3, 3), (1 << 4, 4), (1 << 5, 3)].iter().filter(|(f, _)| flags & f != 0).map(|(_, n)| n).sum();
        let base = match r.rel(b + 0x14)? {
            Some(o) if count > 0 => r.f32s(o, count)?,
            None if count == 0 => vec![],
            _ => bail!("bone animation {name}: base values don't match its flags"),
        };
        bones.push(BoneAnim {
            flags,
            begin_rotate: r.u8(b + 8)?,
            begin_translate: r.u8(b + 9)?,
            base_translate: r.u8(b + 0xb)?,
            begin_curve: r.i32(b + 0xc)?.swap_bytes(),
            curves: read_curves(r, b + 0x10, r.u8(b + 0xa)? as usize).with_context(|| format!("bone animation {name}"))?,
            base,
            name,
        });
    }
    Ok(SkeletalAnim {
        name: name.to_string(),
        path: r.name(a + 8)?,
        flags: r.u32(a + 0xc)?,
        frames: r.i32(a + 0x10)?,
        baked_size: switch_baked_size(r.u32(a + 0x1c)?, bones.iter().flat_map(|b| &b.curves)),
        bind_indices: bind_indices(r, a + 0x28, n)?,
        bones,
        user_data: read_user_data(r, a + 0x2c)?,
    })
}

fn material_visibility(a: BoneVisAnim) -> MaterialAnim {
    let mut curves: Vec<Option<Curve>> = vec![None; a.names.len()];
    for c in a.curves {
        if let Some(slot) = curves.get_mut(c.target as usize) {
            *slot = Some(Curve { target: 0, ..c });
        }
    }
    let mut before = 0i16;
    let materials: Vec<MaterialAnimData> = a
        .names
        .iter()
        .zip(curves)
        .enumerate()
        .map(|(k, (name, curve))| {
            let visible = a.base.get(k / 32).is_some_and(|w| w >> (k % 32) & 1 != 0);
            let (visibility, curves, constants) = match curve {
                Some(c) => ([before, 0, -1], vec![c], vec![]),
                None => ([before, -1, 0], vec![], vec![(0, visible as u32)]),
            };
            before += curves.len() as i16;
            MaterialAnimData {
                name: name.clone(),
                param_curve_begin: -1,
                pattern_curve_begin: -1,
                visibility,
                params: vec![],
                patterns: vec![],
                curves,
                constants,
            }
        })
        .collect();
    let baked_size = if a.baked_size == 0 {
        0
    } else {
        materials
            .iter()
            .flat_map(|m: &MaterialAnimData| &m.curves)
            .map(|c| (4 * ((c.end - c.start).floor().max(0.0) as u32 + 1).div_ceil(32) + 16u32).div_ceil(8) * 8)
            .sum()
    };
    MaterialAnim {
        name: format!("{}_fvm", a.name),
        path: a.path,
        flags: a.flags & 0xff,
        frames: a.frames,
        baked_size,
        bind_indices: a.bind_indices,
        param_anim_count: 0,
        pattern_anim_count: 0,
        textures: vec![],
        materials,
        user_data: a.user_data,
    }
}

fn read_scene_anim(r: &R, a: usize, name: &str) -> Result<SceneAnim> {
    if r.bytes(a, 4)? != b"FSCN" {
        bail!("{name} isn't an FSCN section");
    }
    if r.u16(a + 0x10)? > 0 || r.u16(a + 0x12)? > 0 {
        bail!("light and fog animations aren't supported yet");
    }
    let mut cameras = Vec::new();
    for (cname, at) in r.group(a + 0x14)? {
        let c = at.context("camera animation without data")?;
        if r.bytes(c, 4)? != b"FCAM" {
            bail!("camera animation {cname} isn't an FCAM section");
        }
        let curves = read_curves(r, c + 0x18, r.u8(c + 0xc)? as usize)?;
        let base = match r.rel(c + 0x1c)? {
            Some(o) => r.f32s(o, 11)?,
            None => vec![],
        };
        cameras.push(CameraAnim {
            name: if cname.is_empty() { r.name(c + 0x14)? } else { cname },
            flags: r.u16(c + 4)?,
            frames: r.i32(c + 8)?,
            baked_size: switch_baked_size(r.u32(c + 0x10)?, curves.iter()),
            curves,
            base,
            user_data: read_user_data(r, c + 0x20)?,
        });
    }
    Ok(SceneAnim { name: name.to_string(), path: r.name(a + 8)?, cameras, user_data: read_user_data(r, a + 0x20)? })
}

fn read_bone_vis_anim(r: &R, a: usize, name: &str) -> Result<BoneVisAnim> {
    if r.bytes(a, 4)? != b"FVIS" {
        bail!("{name} isn't an FVIS section");
    }
    let n = r.u16(a + 0x14)? as usize;
    let n_curves = r.u16(a + 0x16)? as usize;
    let names = match r.rel(a + 0x24)? {
        Some(o) => (0..n).map(|i| r.name(o + 4 * i)).collect::<Result<_>>()?,
        None if n == 0 => vec![],
        None => bail!("bone names are missing"),
    };
    let base = match r.rel(a + 0x2c)? {
        Some(o) => (0..n.div_ceil(32)).map(|i| r.u32(o + 4 * i)).collect::<Result<_>>()?,
        None => vec![0; n.div_ceil(32)],
    };
    let wiiu_baked = r.u32(a + 0x18)?;
    Ok(BoneVisAnim {
        name: name.to_string(),
        path: r.name(a + 8)?,
        flags: r.u16(a + 0xc)? as u32,
        frames: r.i32(a + 0x10)?,
        baked_size: (2 * wiiu_baked).div_ceil(8) * 8,
        bind_indices: bind_indices(r, a + 0x20, n)?,
        names,
        curves: read_curves(r, a + 0x28, n_curves)?,
        base,
        user_data: read_user_data(r, a + 0x30)?,
    })
}

fn bind_indices(r: &R, field: usize, n: usize) -> Result<Vec<u16>> {
    match r.rel(field)? {
        Some(o) => r.u16s(o, n),
        None => Ok(vec![0xffff; n]),
    }
}

fn read_tex_pattern(r: &R, a: usize, name: &str) -> Result<MaterialAnim> {
    if r.bytes(a, 4)? != b"FTXP" {
        bail!("{name} isn't an FTXP section");
    }
    let n_mat = r.u16(a + 0x16)? as usize;
    let mats_at = if n_mat > 0 { r.must(a + 0x2c, "material animations")? } else { 0 };
    let mut materials = Vec::with_capacity(n_mat);
    for i in 0..n_mat {
        let m = mats_at + 0x1c * i;
        let (n_pat, n_curve) = (r.u16(m)? as usize, r.u16(m + 2)? as usize);
        let base = r.rel(m + 0x18)?;
        let mut constants = Vec::new();
        let mut patterns = Vec::with_capacity(n_pat);
        if n_pat > 0 {
            let p = r.must(m + 0x10, "pattern animations")?;
            for k in 0..n_pat {
                let o = p + 8 * k;
                let curve = r.u8(o)? as i8;
                let begin_constant = if curve < 0 {
                    let b = base.context("a pattern animation without a curve has no base value")?;
                    constants.push((0, r.u16(b + 2 * k)? as i16 as i32 as u32));
                    (constants.len() - 1) as i16
                } else {
                    -1
                };
                patterns.push(PatternAnimInfo {
                    name: r.name(o + 4)?,
                    curve: curve as i16,
                    begin_constant,
                    sub_bind: r.u8(o + 1)? as i8,
                });
            }
        }
        materials.push(MaterialAnimData {
            name: r.name(m + 0xc)?,
            param_curve_begin: -1,
            pattern_curve_begin: r.i32(m + 4)? as i16,
            visibility: [-1; 3],
            params: vec![],
            patterns,
            curves: read_curves(r, m + 0x14, n_curve)?,
            constants,
        });
    }
    Ok(MaterialAnim {
        name: name.to_string(),
        path: r.name(a + 8)?,
        flags: r.u16(a + 0xc)? as u32,
        frames: r.i32(a + 0x10)?,
        baked_size: switch_baked_size(r.u32(a + 0x20)?, materials.iter().flat_map(|m| &m.curves)),
        bind_indices: bind_indices(r, a + 0x28, n_mat)?,
        param_anim_count: 0,
        pattern_anim_count: r.u32(a + 0x18)? as u16,
        textures: r.group(a + 0x30)?.into_iter().map(|(n, _)| n).collect(),
        materials,
        user_data: read_user_data(r, a + 0x34)?,
    })
}

pub fn read_textures_container(d: &[u8]) -> Result<File> {
    let mut r = R { d, version: 0 };
    r.version = r.u32(4)?;
    if d.get(..4) != Some(b"FRES") || r.u16(8)? != 0xfeff {
        bail!("Not a Wii U BFRES file");
    }
    let mut file = File {
        name: r.name(0x14)?,
        models: vec![],
        skeletal_anims: vec![],
        material_anims: vec![],
        bone_vis_anims: vec![],
        scene_anims: vec![],
        external: vec![],
    };
    for (name, at) in r.group(0x20 + 4 * GROUP_EXTERNAL)? {
        let at = at.context("external file without data")?;
        let data_at = r.must(at, "external file data")?;
        let size = r.u32(at + 4)? as usize;
        file.external.push((name, r.bytes(data_at, size)?.to_vec()));
    }
    Ok(file)
}

pub fn read(d: &[u8], allow_animations_dropped: bool) -> Result<File> {
    let mut r = R { d, version: 0 };
    if d.get(..4) != Some(b"FRES") || r.u16(8)? != 0xfeff {
        bail!("Not a Wii U BFRES file");
    }
    let version = r.u32(4)?;
    r.version = version;
    if version >> 24 != 4 && version >> 24 != 3 {
        bail!("Wii U BFRES version {version:#x} isn't supported");
    }
    let counts: Vec<u16> = (0..12).map(|i| r.u16(0x50 + 2 * i)).collect::<Result<_>>()?;
    if counts[GROUP_TEXTURES] != 0 {
        bail!("it holds textures inside the model file, which SWMM doesn't convert yet");
    }
    let unsupported: Vec<String> = UNSUPPORTED_ANIMS
        .iter()
        .filter(|(g, _)| counts[*g] > 0)
        .map(|(g, what)| format!("{} {what}", counts[*g]))
        .collect();
    if !unsupported.is_empty() && !allow_animations_dropped {
        bail!("it holds {}, which SWMM can't convert from Wii U yet", unsupported.join(", "));
    }
    let mut file =
        File {
        name: r.name(0x14)?,
        models: vec![],
        skeletal_anims: vec![],
        material_anims: vec![],
        bone_vis_anims: vec![],
        scene_anims: vec![],
        external: vec![],
    };
    for (name, at) in r.group(0x20 + 4 * GROUP_MODELS)? {
        let at = at.context("model without data")?;
        file.models.push(read_model(&r, at, name)?);
    }
    if (counts[GROUP_SKELETAL] > 0 || counts[GROUP_BONE_VIS] > 0 || counts[GROUP_MAT_VIS] > 0) && version >> 24 != 4 {
        bail!("animations of Wii U BFRES version {version:#x} aren't supported");
    }
    for (name, at) in r.group(0x20 + 4 * GROUP_SKELETAL)? {
        let at = at.context("animation without data")?;
        file.skeletal_anims.push(read_skeletal_anim(&r, at, &name).with_context(|| format!("animation {name}"))?);
    }
    for (name, at) in r.group(0x20 + 4 * GROUP_BONE_VIS)? {
        let at = at.context("animation without data")?;
        file.bone_vis_anims.push(read_bone_vis_anim(&r, at, &name).with_context(|| format!("animation {name}"))?);
    }
    for (group, suffix) in MATERIAL_ANIMS {
        if counts[group] > 0 && version >> 24 != 4 {
            bail!("animations of Wii U BFRES version {version:#x} aren't supported");
        }
        for (name, at) in r.group(0x20 + 4 * group)? {
            let at = at.context("animation without data")?;
            let anim = if group == GROUP_TEX_PATTERN { read_tex_pattern(&r, at, &name) } else { read_param_anim(&r, at, &name) };
            let mut anim = anim.with_context(|| format!("animation {name}"))?;
            anim.name = format!("{name}{suffix}");
            file.material_anims.push(anim);
        }
    }
    if counts[GROUP_SCENE] > 0 && version >> 24 != 4 {
        bail!("animations of Wii U BFRES version {version:#x} aren't supported");
    }
    for (name, at) in r.group(0x20 + 4 * GROUP_SCENE)? {
        let at = at.context("animation without data")?;
        file.scene_anims.push(read_scene_anim(&r, at, &name).with_context(|| format!("animation {name}"))?);
    }
    for (name, at) in r.group(0x20 + 4 * GROUP_MAT_VIS)? {
        let at = at.context("animation without data")?;
        let anim = read_bone_vis_anim(&r, at, &name).with_context(|| format!("animation {name}"))?;
        file.material_anims.push(material_visibility(anim));
    }
    file.material_anims.sort_by(|a, b| a.name.cmp(&b.name));
    for (name, at) in r.group(0x20 + 4 * GROUP_EXTERNAL)? {
        let at = at.context("external file without data")?;
        let data_at = r.must(at, "external file data")?;
        let size = r.u32(at + 4)? as usize;
        file.external.push((name, r.bytes(data_at, size)?.to_vec()));
    }
    Ok(file)
}

fn read_model(r: &R, m: usize, name: String) -> Result<Model> {
    if r.bytes(m, 4)? != b"FMDL" {
        bail!("model {name} isn't an FMDL section");
    }
    let skeleton = read_skeleton(r, r.must(m + 0xc, "skeleton")?).with_context(|| format!("{name}: skeleton"))?;
    let n_vtx = r.u16(m + 0x20)? as usize;
    let vtx_array = if n_vtx == 0 { r.rel(m + 0x10)?.unwrap_or(0) } else { r.must(m + 0x10, "vertex buffers")? };
    let vertex_buffers =
        (0..n_vtx).map(|i| read_vertex_buffer(r, vtx_array + 0x20 * i).with_context(|| format!("{name}: vertex buffer {i}")))
            .collect::<Result<Vec<_>>>()?;
    let shapes = r
        .group(m + 0x14)?
        .into_iter()
        .map(|(n, at)| read_shape(r, at.context("shape without data")?, n.clone()).with_context(|| format!("shape {n}")))
        .collect::<Result<Vec<_>>>()?;
    let materials = r
        .group(m + 0x18)?
        .into_iter()
        .map(|(n, at)| read_material(r, at.context("material without data")?, n.clone()).with_context(|| format!("material {n}")))
        .collect::<Result<Vec<_>>>()?;
    let user_data = read_user_data(r, m + 0x1c)?;
    Ok(Model {
        name,
        path: r.name(m + 8)?,
        skeleton,
        vertex_buffers,
        shapes,
        materials,
        user_data,
        total_vertices: r.u32(m + 0x28)?,
    })
}

fn read_user_data(r: &R, field: usize) -> Result<Vec<UserData>> {
    r.group(field)?
        .into_iter()
        .map(|(name, at)| {
            let o = at.context("user data without data")?;
            let count = r.u16(o + 4)? as usize;
            let kind = r.u8(o + 6)?;
            let value = match kind {
                0 => UserValue::Int((0..count).map(|k| r.i32(o + 8 + 4 * k)).collect::<Result<_>>()?),
                1 => UserValue::Float(r.f32s(o + 8, count)?),
                2 => UserValue::Str((0..count).map(|k| r.name(o + 8 + 4 * k)).collect::<Result<_>>()?, false),
                3 => UserValue::Str(
                    (0..count).map(|k| r.wide_name(o + 8 + 4 * k)).collect::<Result<_>>()?,
                    false,
                ),
                4 => UserValue::Bytes(r.bytes(o + 8, count)?.to_vec()),
                k => bail!("user data {name} has an unknown kind {k}"),
            };
            Ok(UserData { name, value })
        })
        .collect()
}

fn read_skeleton(r: &R, s: usize) -> Result<Skeleton> {
    let flags = r.u32(s + 4)?;
    let n = r.u16(s + 8)? as usize;
    let smooth = r.u16(s + 0xa)? as usize;
    let rigid = r.u16(s + 0xc)? as usize;
    let bones_at = r.must(s + 0x14, "bones")?;
    let mut bones = Vec::with_capacity(n);
    for i in 0..n {
        let b = bones_at + 0x40 * i;
        bones.push(Bone {
            name: r.name(b)?,
            index: r.u16(b + 4)?,
            parent: r.u16(b + 6)?,
            smooth: r.u16(b + 8)? as i16,
            rigid: r.u16(b + 0xa)? as i16,
            billboard: r.u16(b + 0xc)? as i16,
            flags: r.u32(b + 0x10)?,
            srt: r.f32s(b + 0x14, 10)?.try_into().map_err(|_| anyhow_ext::anyhow!("bone transform"))?,
            user_data: read_user_data(r, b + 0x3c)?,
        });
    }
    let matrix_to_bone = match r.rel(s + 0x18)? {
        Some(o) => r.u16s(o, smooth + rigid)?,
        None => vec![],
    };
    let inverse = match r.rel(s + 0x1c)? {
        Some(o) => r.f32s(o, 12 * smooth)?,
        None => vec![],
    };
    Ok(Skeleton { flags, bones, smooth_count: smooth as u16, rigid_count: rigid as u16, matrix_to_bone, inverse })
}

pub fn attribute_format(gx2: u32) -> Result<(u32, usize, usize)> {
    let kind = match gx2 & 0xf00 {
        0x000 => 1,
        0x100 => 3,
        0x200 => 2,
        0x300 => 4,
        0x800 => 5,
        _ => bail!("vertex format {gx2:#x} isn't supported yet"),
    };
    let float = kind == 5;
    let (channel, comp, size) = match (gx2 & 0xff, float) {
        (0x00, false) => (0x02, 1, 1),
        (0x02, false) => (0x0a, 2, 2),
        (0x04, false) => (0x09, 1, 2),
        (0x05, false) => (0x14, 4, 4),
        (0x06, true) => (0x14, 4, 4),
        (0x07, false) => (0x12, 2, 4),
        (0x08, true) => (0x12, 2, 4),
        (0x0a, false) => (0x0b, 1, 4),
        (0x0b, false) => (0x0e, 4, 4),
        (0x0c, false) => (0x17, 4, 8),
        (0x0d, true) => (0x17, 4, 8),
        (0x0e, false) => (0x15, 2, 8),
        (0x0f, true) => (0x15, 2, 8),
        (0x10, false) => (0x18, 4, 12),
        (0x11, true) => (0x18, 4, 12),
        (0x12, false) => (0x19, 4, 16),
        (0x13, true) => (0x19, 4, 16),
        _ => bail!("vertex format {gx2:#x} isn't supported yet"),
    };
    Ok(((channel << 8) | kind, comp, size))
}

fn read_vertex_buffer(r: &R, v: usize) -> Result<VertexBuffer> {
    if r.bytes(v, 4)? != b"FVTX" {
        bail!("vertex buffer isn't an FVTX section");
    }
    let n_attr = r.u8(v + 4)? as usize;
    let n_buf = r.u8(v + 5)? as usize;
    let attrs_at = r.must(v + 0x10, "attributes")?;
    let mut attributes = Vec::with_capacity(n_attr);
    for i in 0..n_attr {
        let a = attrs_at + 0xc * i;
        let gx2 = r.u32(a + 8)?;
        attributes.push(Attribute {
            name: r.name(a)?,
            buffer: r.u8(a + 4)?,
            offset: r.u16(a + 6)?,
            format: attribute_format(gx2)?.0,
            gx2_format: gx2,
        });
    }
    let bufs_at = r.must(v + 0x18, "buffers")?;
    let mut buffers = Vec::with_capacity(n_buf);
    for b in 0..n_buf {
        let o = bufs_at + 0x18 * b;
        let size = r.u32(o + 4)? as usize;
        let stride = r.u16(o + 0xc)? as usize;
        let data = r.bytes(r.must(o + 0x14, "vertex data")?, size)?;
        let swapped = swap_vertices(data, stride, &attributes, b as u8)?;
        let mut packed = 0;
        for a in attributes.iter().filter(|a| a.buffer as usize == b) {
            packed = packed.max(a.offset as usize + attribute_format(a.gx2_format)?.2);
        }
        let packed = packed.div_ceil(4) * 4;
        let (stride_out, out) = if packed > 0 && packed < stride {
            (packed, swapped.chunks(stride).flat_map(|v| &v[..packed.min(v.len())]).copied().collect())
        } else {
            (stride, swapped)
        };
        buffers.push(Buffer { stride: stride_out as u32, data: out });
    }
    Ok(VertexBuffer {
        vertex_count: r.u32(v + 8)?,
        skin_count: r.u8(v + 0xc)?,
        attributes,
        buffers,
    })
}

fn swap_vertices(data: &[u8], stride: usize, attrs: &[Attribute], buffer: u8) -> Result<Vec<u8>> {
    let mut out = data.to_vec();
    if stride == 0 {
        return Ok(out);
    }
    for a in attrs.iter().filter(|a| a.buffer == buffer) {
        let (_, comp, size) = attribute_format(a.gx2_format)?;
        let half = a.gx2_format & 0x800 != 0 && comp == 2;
        let mut v = 0;
        while v + stride <= data.len() {
            let mut c = a.offset as usize;
            while c < a.offset as usize + size {
                let at = v + c;
                let src = data.get(at..at + comp).context("vertex attribute runs past its buffer")?;
                let mut x: Vec<u8> = src.iter().rev().copied().collect();
                if half && x[1] & 0x7c == 0 && (x[0] != 0 || x[1] & 0x03 != 0) {
                    x = vec![0, x[1] & 0x80];
                }
                out[at..at + comp].copy_from_slice(&x);
                c += comp;
            }
            v += stride;
        }
    }
    Ok(out)
}

fn read_shape(r: &R, s: usize, name: String) -> Result<Shape> {
    if r.bytes(s, 4)? != b"FSHP" {
        bail!("shape {name} isn't an FSHP section");
    }
    let n_skin = r.u16(s + 0x14)? as usize;
    let n_mesh = r.u8(s + 0x17)? as usize;
    let n_key = r.u8(s + 0x18)?;
    if n_key != 0 {
        bail!("shape {name} has key shapes (morphs), which SWMM can't convert yet");
    }
    let meshes_at = r.must(s + 0x24, "meshes")?;
    let mut meshes = Vec::with_capacity(n_mesh);
    let mut n_sub_total = 0;
    for k in 0..n_mesh {
        let o = meshes_at + 0x1c * k;
        let prim = match r.u32(o)? {
            1 => 0,
            2 => 1,
            3 => 2,
            4 => 3,
            6 => 4,
            p => bail!("shape {name}: primitive type {p} isn't supported yet"),
        };
        let (index_format, size) = match r.u32(o + 4)? {
            0 | 4 => (1, 2),
            1 | 9 => (2, 4),
            f => bail!("shape {name}: index format {f} isn't supported yet"),
        };
        let n_sub = r.u16(o + 0xc)? as usize;
        n_sub_total += n_sub;
        let subs_at = r.must(o + 0x10, "sub-meshes")?;
        let sub_meshes = (0..n_sub)
            .map(|i| Ok((r.u32(subs_at + 8 * i)?, r.u32(subs_at + 8 * i + 4)?)))
            .collect::<Result<Vec<_>>>()?;
        let ib = r.must(o + 0x14, "index buffer")?;
        let bytes = r.u32(ib + 4)? as usize;
        let raw = r.bytes(r.must(ib + 0x14, "index data")?, bytes)?;
        let indices = raw.chunks(size).flat_map(|c| c.iter().rev().copied().collect::<Vec<_>>()).collect();
        meshes.push(Mesh {
            primitive: prim,
            index_format,
            index_count: r.u32(o + 8)?,
            first_vertex: r.u32(o + 0x18)?,
            sub_meshes,
            indices,
        });
    }
    let skin_bones = match r.rel(s + 0x28)? {
        Some(o) => r.u16s(o, n_skin)?,
        None => vec![],
    };
    let boxes = n_sub_total + n_mesh;
    let nodes = r.u16(s + 0x1a)?;
    let bounding_boxes = if r.version < 0x0405_0000 && nodes > 0 {
        let root = r.f32s(r.must(s + 0x34, "bounding boxes")?, 6)?;
        root.iter().copied().cycle().take(6 * boxes).collect()
    } else {
        match r.rel(s + 0x30)? {
            Some(o) => r.f32s(o, 6 * boxes)?,
            None => vec![0.0; 6 * boxes],
        }
    };
    let radii = if r.version < 0x0405_0000 {
        vec![r.f32(s + 0x1c)?; n_mesh]
    } else {
        match r.rel(s + 0x1c)? {
            Some(o) => r.f32s(o, n_mesh)?,
            None => vec![0.0; n_mesh],
        }
    };
    Ok(Shape {
        name,
        flags: r.u32(s + 8)?,
        index: r.u16(s + 0xc)?,
        material: r.u16(s + 0xe)?,
        bone: r.u16(s + 0x10)?,
        vertex_buffer: r.u16(s + 0x12)?,
        skin_bones,
        vertex_skin_count: r.u8(s + 0x16)?,
        target_attributes: r.u8(s + 0x19)?,
        meshes,
        bounding_boxes,
        radii,
    })
}

const FUNC: [&str; 8] = ["never", "less", "equal", "lequal", "greater", "nequal", "gequal", "always"];

fn blend(v: u32) -> Result<&'static str> {
    Ok(match v {
        0 => "zero",
        1 => "one",
        2 => "src_color",
        3 => "one_minus_src_color",
        4 => "src_alpha",
        5 => "one_minus_src_alpha",
        6 => "dst_alpha",
        7 => "one_minus_dst_alpha",
        8 => "dst_color",
        9 => "one_minus_dst_color",
        10 => "src_alpha_saturate",
        13 => "const_color",
        14 => "one_minus_const_color",
        15 => "const_alpha",
        16 => "one_minus_const_alpha",
        _ => bail!("blend factor {v} isn't supported yet"),
    })
}

fn blend_op(v: u32) -> Result<&'static str> {
    Ok(match v {
        0 => "add",
        1 => "sub",
        2 => "min",
        3 => "max",
        4 => "rev_sub",
        _ => bail!("blend operation {v} isn't supported yet"),
    })
}

fn render_state(r: &R, o: usize) -> Result<(Vec<RenderInfo>, f32)> {
    let flags = r.u32(o)?;
    let poly = r.u32(o + 4)?;
    let depth = r.u32(o + 8)?;
    let afunc = r.u32(o + 0xc)?;
    let aref = r.f32(o + 0x10)?;
    let ctl = r.u32(o + 0x1c)?;
    let constant = r.f32s(o + 0x20, 4)?;
    let s = |n: &str, v: &str| RenderInfo { name: n.into(), value: RenderValue::Str(vec![v.into()]) };
    let tf = |b: bool| if b { "true" } else { "false" };
    let mode = match flags & 0xf {
        0 => "custom",
        1 => "opaque",
        2 => "mask",
        3 => "translucent",
        m => bail!("render mode {m} isn't supported yet"),
    };
    let bmode = match (flags >> 4) & 0xf {
        0 => "none",
        1 => "color",
        2 => "logic",
        m => bail!("blend mode {m} isn't supported yet"),
    };
    let face = ["both", "back", "front", "none"][(poly & 3) as usize];
    let out = vec![
        s("gsys_render_state_mode", mode),
        s("gsys_render_state_display_face", face),
        s("gsys_render_state_blend_mode", bmode),
        s("gsys_depth_test_enable", tf(depth & 2 != 0)),
        s("gsys_depth_test_write", tf(depth & 4 != 0)),
        s("gsys_depth_test_func", FUNC[((depth >> 4) & 7) as usize]),
        s("gsys_color_blend_rgb_src_func", blend(ctl & 0x1f)?),
        s("gsys_color_blend_rgb_dst_func", blend((ctl >> 8) & 0x1f)?),
        s("gsys_color_blend_rgb_op", blend_op((ctl >> 5) & 7)?),
        s("gsys_color_blend_alpha_src_func", blend((ctl >> 16) & 0x1f)?),
        s("gsys_color_blend_alpha_dst_func", blend((ctl >> 24) & 0x1f)?),
        s("gsys_color_blend_alpha_op", blend_op((ctl >> 21) & 7)?),
        s("gsys_alpha_test_enable", tf(afunc & 8 != 0)),
        s("gsys_alpha_test_func", FUNC[(afunc & 7) as usize]),
        RenderInfo { name: "gsys_color_blend_const_color".into(), value: RenderValue::Float(constant) },
        RenderInfo { name: "gsys_alpha_test_value".into(), value: RenderValue::Float(vec![aref]) },
    ];
    Ok((out, aref))
}

pub fn sampler(r0: u32, r1: u32) -> [u8; 0x20] {
    let mut out = [0u8; 0x20];
    out[0] = (r0 & 7) as u8;
    out[1] = ((r0 >> 3) & 7) as u8;
    out[2] = ((r0 >> 6) & 7) as u8;
    out[3] = ((r0 >> 26) & 7) as u8;
    out[4] = ((r0 >> 22) & 3) as u8;
    out[5] = 1 << ((r0 >> 19) & 7);
    let mag = (r0 >> 9) & 7;
    let min = (r0 >> 12) & 7;
    let mip = (r0 >> 17) & 3;
    let filter = mip | ((if mag == 1 { 2 } else { 1 }) << 2) | ((if min == 1 { 2 } else { 1 }) << 4);
    out[6..8].copy_from_slice(&(filter as u16).to_le_bytes());
    let min_lod = (r1 & 0x3ff) as f32 / 64.0;
    let max_lod = ((r1 >> 10) & 0x3ff) as f32 / 64.0;
    let mut bias = ((r1 >> 20) & 0xfff) as i32;
    if bias & 0x800 != 0 {
        bias -= 0x1000;
    }
    out[8..12].copy_from_slice(&min_lod.to_le_bytes());
    out[12..16].copy_from_slice(&max_lod.to_le_bytes());
    out[16..20].copy_from_slice(&(bias as f32 / 64.0).to_le_bytes());
    out
}

fn read_material(r: &R, m: usize, name: String) -> Result<Material> {
    if r.bytes(m, 4)? != b"FMAT" {
        bail!("material {name} isn't an FMAT section");
    }
    let n_tex = r.u8(m + 0x10)? as usize;
    let n_smp = r.u8(m + 0x11)? as usize;
    let n_par = r.u16(m + 0x12)? as usize;
    let n_volatile = r.u16(m + 0x14)?;
    let data_size = r.u16(m + 0x16)? as usize;
    let raw_size = r.u16(m + 0x18)?;
    let mut render_infos = Vec::new();
    for (n, at) in r.group(m + 0x1c)? {
        let o = at.context("render info without data")?;
        let count = r.u16(o)? as usize;
        let value = match r.u8(o + 2)? {
            0 => RenderValue::Int((0..count).map(|k| r.i32(o + 8 + 4 * k)).collect::<Result<_>>()?),
            1 => RenderValue::Float(r.f32s(o + 8, count)?),
            2 => RenderValue::Str((0..count).map(|k| r.name(o + 8 + 4 * k)).collect::<Result<_>>()?),
            t => bail!("render info {n} has an unknown kind {t}"),
        };
        render_infos.push(RenderInfo { name: n, value });
    }
    let (state, alpha_ref) = render_state(r, r.must(m + 0x20, "render state")?)?;
    render_infos.extend(state);
    let sa = r.must(m + 0x24, "shader assign")?;
    let map = |field: usize| -> Result<Vec<(String, String)>> {
        r.group(field)?
            .into_iter()
            .map(|(k, at)| Ok((k, r.str_at(at.context("shader assign value")?)?)))
            .collect()
    };
    let shader_assign = ShaderAssign {
        archive: r.name(sa)?,
        model: r.name(sa + 4)?,
        revision: r.u32(sa + 8)?,
        attributes: map(sa + 0x10)?,
        samplers: map(sa + 0x14)?,
        options: map(sa + 0x18)?,
    };
    let textures_at = r.rel(m + 0x28)?;
    let textures = (0..n_tex)
        .map(|k| r.name(textures_at.context("texture names")? + 8 * k))
        .collect::<Result<Vec<_>>>()?;
    let samplers_at = r.rel(m + 0x2c)?;
    let samplers = (0..n_smp)
        .map(|k| {
            let o = samplers_at.context("samplers")? + 0x18 * k;
            Ok((r.name(o + 0x10)?, sampler(r.u32(o)?, r.u32(o + 4)?)))
        })
        .collect::<Result<Vec<_>>>()?;
    let params_at = r.rel(m + 0x34)?;
    let mut params = Vec::with_capacity(n_par);
    for k in 0..n_par {
        let o = params_at.context("shader params")? + 0x14 * k;
        params.push(ShaderParam {
            name: r.name(o + 0x10)?,
            kind: r.u8(o)?,
            size: r.u8(o + 1)?,
            offset: r.u16(o + 2)?,
            unknown: r.i32(o + 4)?,
            depended: r.u16(o + 0xc)?,
            depend: r.u16(o + 0xe)?,
        });
    }
    let mut param_data = Vec::with_capacity(data_size);
    if data_size > 0 {
        let raw = r.bytes(r.must(m + 0x3c, "shader param data")?, data_size)?;
        for c in raw.chunks(4) {
            param_data.extend(c.iter().rev());
        }
    }
    if let Some(p) = params.iter().find(|p| p.name == "gsys_alpha_test_ref_value") {
        let at = p.offset as usize;
        if let Some(slot) = param_data.get_mut(at..at + 4) {
            slot.copy_from_slice(&alpha_ref.to_le_bytes());
        }
    }
    if let Some(v) = r.rel(m + 0x44)? {
        let flags = r.bytes(v, n_par.div_ceil(8))?;
        if flags.iter().any(|b| *b != 0) {
            bail!("material {name} has volatile shader parameters, which SWMM can't convert yet");
        }
    }
    Ok(Material {
        name,
        flags: r.u32(m + 8)?,
        render_infos,
        shader_assign,
        textures,
        samplers,
        params,
        param_data,
        volatile_count: n_volatile,
        raw_param_size: raw_size,
        user_data: read_user_data(r, m + 0x40)?,
    })
}
