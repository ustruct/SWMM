use std::collections::HashMap;

use anyhow_ext::{Result, bail};

use super::model::*;
use crate::tex::dict;

#[derive(Clone, Debug)]
enum To {
    At(usize),
    Str(String),
    Gpu(usize),
    Pool,
    Ext(usize),
    NullListed,
}

struct W {
    b: Vec<u8>,
    ptrs: Vec<(usize, To)>,
    blocks: Vec<usize>,
    index_data: Vec<u8>,
    vertex_data: Vec<u8>,
    vertex_offsets: Vec<usize>,
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

impl W {
    fn alloc(&mut self, size: usize, a: usize) -> usize {
        let at = align(self.b.len(), a);
        self.b.resize(at + size, 0);
        at
    }
    fn put(&mut self, o: usize, v: &[u8]) {
        self.b[o..o + v.len()].copy_from_slice(v);
    }
    fn u8(&mut self, o: usize, v: u8) {
        self.b[o] = v;
    }
    fn u16(&mut self, o: usize, v: u16) {
        self.put(o, &v.to_le_bytes());
    }
    fn u32(&mut self, o: usize, v: u32) {
        self.put(o, &v.to_le_bytes());
    }
    fn ptr(&mut self, o: usize, to: To) {
        self.ptrs.push((o, to));
    }
    fn at(&mut self, o: usize, target: usize) {
        self.ptr(o, To::At(target));
    }
    fn s(&mut self, o: usize, text: &str) {
        self.ptr(o, To::Str(text.to_string()));
    }
    fn block(&mut self, o: usize, magic: &[u8; 4]) {
        self.put(o, magic);
        self.blocks.push(o);
    }
    fn data(&mut self, v: &[u8], a: usize) -> usize {
        let at = self.alloc(v.len().max(1), a);
        self.put(at, v);
        at
    }
    fn strings(&mut self, v: &[String]) -> usize {
        let at = self.alloc(8 * v.len().max(1), 8);
        for (i, s) in v.iter().enumerate() {
            self.s(at + 8 * i, s);
        }
        at
    }
    fn dict<S: AsRef<str>>(&mut self, names: &[S]) -> usize {
        let keys: Vec<&str> = names.iter().map(|n| n.as_ref()).collect();
        let nodes = dict::build(&keys);
        let at = self.alloc(8 + 16 * nodes.len(), 8);
        self.u32(at + 4, keys.len() as u32);
        for (k, node) in nodes.iter().enumerate() {
            let e = at + 8 + 16 * k;
            self.u32(e, node.bit as u32);
            self.u16(e + 4, node.left);
            self.u16(e + 6, node.right);
            let name = if k == 0 { "" } else { keys[k - 1] };
            self.s(e + 8, name);
        }
        at
    }
    fn user_data(&mut self, list: &[UserData], array_field: usize, dict_field: usize) {
        if list.is_empty() {
            self.ptr(dict_field, To::NullListed);
            return;
        }
        let arr = self.user_data_array(list, array_field);
        self.user_data_values(list, arr);
        self.user_data_dict(list, dict_field);
    }
    fn user_data_array(&mut self, list: &[UserData], array_field: usize) -> usize {
        let arr = self.alloc(0x40 * list.len(), 8);
        for (i, u) in list.iter().enumerate() {
            let o = arr + 0x40 * i;
            self.s(o, &u.name);
            let (count, kind) = match &u.value {
                UserValue::Int(v) => (v.len(), 0),
                UserValue::Float(v) => (v.len(), 1),
                UserValue::Str(v, wide) => (v.len(), if *wide { 3 } else { 2 }),
                UserValue::Bytes(v) => (v.len(), 4),
            };
            self.u32(o + 0x10, count as u32);
            self.u8(o + 0x14, kind);
        }
        self.at(array_field, arr);
        arr
    }
    fn user_data_values(&mut self, list: &[UserData], arr: usize) {
        for (i, u) in list.iter().enumerate() {
            let empty = match &u.value {
                UserValue::Int(v) => v.is_empty(),
                UserValue::Float(v) => v.is_empty(),
                UserValue::Str(v, _) => v.is_empty(),
                UserValue::Bytes(v) => v.is_empty(),
            };
            if empty {
                continue;
            }
            let data = match &u.value {
                UserValue::Int(v) => self.data(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>(), 8),
                UserValue::Float(v) => self.data(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>(), 8),
                UserValue::Str(v, _) => self.strings(v),
                UserValue::Bytes(v) => self.data(v, 8),
            };
            self.at(arr + 0x40 * i + 8, data);
        }
    }
    fn user_data_dict(&mut self, list: &[UserData], dict_field: usize) {
        let names: Vec<&str> = list.iter().map(|u| u.name.as_str()).collect();
        let d = self.dict(&names);
        self.at(dict_field, d);
    }
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn write(file: &File) -> Result<Vec<u8>> {
    let mut w = W { b: vec![0; 0xd0], ptrs: vec![], blocks: vec![], index_data: vec![], vertex_data: vec![],
                    vertex_offsets: vec![] };
    let gpu = file.models.iter().any(|m| !m.vertex_buffers.is_empty());
    let n_models = file.models.len();
    let models = w.alloc(0x78 * n_models, 8);
    let n_skel = file.skeletal_anims.len();
    let skel = w.alloc(0x60 * n_skel, 8);
    let n_anims = file.material_anims.len();
    let anims = w.alloc(0x78 * n_anims, 8);
    let n_vis = file.bone_vis_anims.len();
    let vis = w.alloc(0x68 * n_vis, 8);
    let n_scene = file.scene_anims.len();
    let scenes = w.alloc(0x68 * n_scene, 8);
    let buffer_info = w.alloc(0x20, 8);
    let ext_array = (!file.external.is_empty()).then(|| w.alloc(0x10 * file.external.len(), 8));
    w.s(0x20, &file.name);
    let kinds: [(usize, usize, Vec<&str>); 5] = [
        (0x28, models, file.models.iter().map(|m| m.name.as_str()).collect()),
        (0x38, skel, file.skeletal_anims.iter().map(|m| m.name.as_str()).collect()),
        (0x48, anims, file.material_anims.iter().map(|m| m.name.as_str()).collect()),
        (0x58, vis, file.bone_vis_anims.iter().map(|m| m.name.as_str()).collect()),
        (0x78, scenes, file.scene_anims.iter().map(|m| m.name.as_str()).collect()),
    ];
    for (field, array, names) in kinds {
        if names.is_empty() {
            w.ptr(field + 8, To::NullListed);
        } else {
            w.at(field, array);
            let d = w.dict(&names);
            w.at(field + 8, d);
        }
    }
    {
        let f = 0x70;
        w.ptr(f, To::NullListed);
    }
    if gpu {
        w.ptr(0x88, To::Pool);
        w.at(0x90, buffer_info);
    }
    match ext_array {
        Some(a) => {
            w.at(0x98, a);
            let names: Vec<&str> = file.external.iter().map(|e| e.0.as_str()).collect();
            let d = w.dict(&names);
            w.at(0xa0, d);
            for (i, (_, data)) in file.external.iter().enumerate() {
                w.ptr(a + 0x10 * i, To::Ext(i));
                w.u32(a + 0x10 * i + 8, data.len() as u32);
            }
        }
        None => w.ptr(0xa0, To::NullListed),
    }
    w.u16(0xbc, n_models as u16);
    w.u16(0xbe, n_skel as u16);
    w.u16(0xc0, n_anims as u16);
    w.u16(0xc2, n_vis as u16);
    w.u16(0xc6, n_scene as u16);
    w.u16(0xc8, file.external.len() as u16);
    w.u32(buffer_info, 0x22);
    if gpu {
        w.ptr(buffer_info + 8, To::Gpu(0));
    }
    for (i, m) in file.models.iter().enumerate() {
        write_model(&mut w, models + 0x78 * i, m)?;
    }
    for (i, a) in file.skeletal_anims.iter().enumerate() {
        write_skeletal_anim(&mut w, skel + 0x60 * i, a);
    }
    for (i, m) in file.material_anims.iter().enumerate() {
        write_material_anim(&mut w, anims + 0x78 * i, m);
    }
    for (i, a) in file.bone_vis_anims.iter().enumerate() {
        write_bone_vis_anim(&mut w, vis + 0x68 * i, a);
    }
    for (i, a) in file.scene_anims.iter().enumerate() {
        write_scene_anim(&mut w, scenes + 0x68 * i, a);
    }
    finish(w, file, gpu.then_some(buffer_info))
}

fn write_scene_anim(w: &mut W, o: usize, a: &SceneAnim) {
    w.block(o, b"FSCN");
    w.s(o + 0x10, &a.name);
    w.s(o + 0x18, &a.path);
    w.alloc(0x68, 8);
    let cams = w.alloc(0x48 * a.cameras.len(), 8);
    w.at(o + 0x20, cams);
    if a.cameras.is_empty() {
        w.ptr(o + 0x28, To::NullListed);
    } else {
        let names: Vec<&str> = a.cameras.iter().map(|c| c.name.as_str()).collect();
        let here = align(w.b.len(), 8);
        for f in [0x30, 0x40, 0x50] {
            w.at(o + f, here);
        }
        let d = w.dict(&names);
        w.at(o + 0x28, d);
    }
    for f in [0x38, 0x48, 0x58] {
        w.ptr(o + f, To::NullListed);
    }
    w.u16(o + 0x60, a.user_data.len() as u16);
    w.u16(o + 0x62, a.cameras.len() as u16);
    for (i, c) in a.cameras.iter().enumerate() {
        let e = cams + 0x48 * i;
        w.put(e, b"FCAM");
        w.s(e + 0x10, &c.name);
        let curves = w.alloc(0x30 * c.curves.len(), 8);
        w.at(e + 0x18, curves);
        let base = w.data(&floats(&c.base), 8);
        w.at(e + 0x20, base);
        let here = align(w.b.len(), 8);
        w.at(e + 0x28, here);
        w.ptr(e + 0x30, To::NullListed);
        for (k, curve) in c.curves.iter().enumerate() {
            write_curve(w, curves + 0x30 * k, curve);
        }
        w.u16(e + 0x38, c.flags);
        w.u32(e + 0x3c, c.frames as u32);
        w.u8(e + 0x40, c.curves.len() as u8);
        w.u32(e + 0x44, c.baked_size);
    }
}

fn write_bone_vis_anim(w: &mut W, o: usize, a: &BoneVisAnim) {
    w.block(o, b"FBVS");
    w.s(o + 0x10, &a.name);
    w.s(o + 0x18, &a.path);
    let bind = w.alloc(2 * a.bind_indices.len().max(1), 8);
    for (i, b) in a.bind_indices.iter().enumerate() {
        w.u16(bind + 2 * i, *b);
    }
    w.at(o + 0x28, bind);
    if !a.names.is_empty() {
        let names = w.strings(&a.names);
        w.at(o + 0x40, names);
    }
    let curves = (!a.curves.is_empty()).then(|| w.alloc(0x30 * a.curves.len(), 8));
    if let Some(c) = curves {
        w.at(o + 0x30, c);
    }
    let base = w.data(&a.base.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(), 8);
    w.at(o + 0x38, base);
    if a.user_data.is_empty() {
        let here = align(w.b.len(), 8);
        w.at(o + 0x48, here);
        w.ptr(o + 0x50, To::NullListed);
    } else {
        w.user_data(&a.user_data, o + 0x48, o + 0x50);
    }
    w.u32(o + 0x58, a.flags);
    w.u32(o + 0x5c, a.frames as u32);
    w.u16(o + 0x60, a.names.len() as u16);
    w.u16(o + 0x62, a.curves.len() as u16);
    w.u32(o + 0x64, a.baked_size);
    if let Some(c) = curves {
        for (k, curve) in a.curves.iter().enumerate() {
            write_curve(w, c + 0x30 * k, curve);
        }
    }
}

fn write_skeletal_anim(w: &mut W, o: usize, a: &SkeletalAnim) {
    w.block(o, b"FSKA");
    w.s(o + 0x10, &a.name);
    w.s(o + 0x18, &a.path);
    let bind = w.alloc(2 * a.bind_indices.len().max(1), 8);
    for (i, b) in a.bind_indices.iter().enumerate() {
        w.u16(bind + 2 * i, *b);
    }
    w.at(o + 0x28, bind);
    let bones = w.alloc(0x28 * a.bones.len(), 8);
    w.at(o + 0x30, bones);
    if a.user_data.is_empty() {
        let here = align(w.b.len(), 8);
        w.at(o + 0x38, here);
        w.ptr(o + 0x40, To::NullListed);
    } else {
        w.user_data(&a.user_data, o + 0x38, o + 0x40);
    }
    w.u32(o + 0x48, a.flags);
    w.u32(o + 0x4c, a.frames as u32);
    w.u32(o + 0x50, a.bones.iter().map(|b| b.curves.len()).sum::<usize>() as u32);
    w.u32(o + 0x54, a.baked_size);
    w.u16(o + 0x58, a.bones.len() as u16);
    w.u16(o + 0x5a, a.user_data.len() as u16);
    for (i, b) in a.bones.iter().enumerate() {
        let e = bones + 0x28 * i;
        w.s(e, &b.name);
        if b.base.is_empty() {
            w.ptr(e + 0x10, To::NullListed);
        } else {
            let base = w.data(&floats(&b.base), 8);
            w.at(e + 0x10, base);
        }
        if !b.curves.is_empty() {
            let c = w.alloc(0x30 * b.curves.len(), 8);
            w.at(e + 8, c);
            for (k, curve) in b.curves.iter().enumerate() {
                write_curve(w, c + 0x30 * k, curve);
            }
        }
        w.u32(e + 0x18, b.flags);
        w.u8(e + 0x1c, b.begin_rotate);
        w.u8(e + 0x1d, b.begin_translate);
        w.u8(e + 0x1e, b.curves.len() as u8);
        w.u8(e + 0x1f, b.base_translate);
        w.u32(e + 0x20, b.begin_curve as u32);
    }
}

fn write_material_anim(w: &mut W, o: usize, a: &MaterialAnim) {
    w.put(o, b"FMAA");
    w.s(o + 0x10, &a.name);
    w.s(o + 0x18, &a.path);
    if a.materials.is_empty() {
        for f in [0x28, 0x30, 0x38] {
            w.ptr(o + f, To::NullListed);
        }
        w.user_data(&a.user_data, o + 0x48, o + 0x50);
        w.u32(o + 0x60, a.flags);
        w.u32(o + 0x68, a.frames as u32);
        w.u32(o + 0x6c, a.baked_size);
        w.u16(o + 0x70, a.param_anim_count);
        w.u16(o + 0x72, a.pattern_anim_count);
        return;
    }
    let bind = w.alloc(2 * a.bind_indices.len().max(1), 8);
    for (i, b) in a.bind_indices.iter().enumerate() {
        w.u16(bind + 2 * i, *b);
    }
    w.at(o + 0x28, bind);
    let mats = w.alloc(0x40 * a.materials.len().max(1), 8);
    w.at(o + 0x30, mats);
    let nt = a.textures.len();
    if nt > 0 {
        let bind = w.alloc(8 * nt, 8);
        w.put(bind, &vec![0xff; 8 * nt]);
        w.at(o + 0x58, bind);
        let runtime = w.alloc(8 * nt, 8);
        w.at(o + 0x38, runtime);
        let names = w.strings(&a.textures);
        w.at(o + 0x40, names);
    } else {
        let here = align(w.b.len(), 8);
        w.at(o + 0x38, here);
    }
    w.user_data(&a.user_data, o + 0x48, o + 0x50);
    w.u32(o + 0x60, a.flags);
    w.u16(o + 0x64, a.materials.len() as u16);
    w.u16(o + 0x66, a.materials.iter().map(|m| m.curves.len()).sum::<usize>() as u16);
    w.u32(o + 0x68, a.frames as u32);
    w.u32(o + 0x6c, a.baked_size);
    w.u16(o + 0x70, a.param_anim_count);
    w.u16(o + 0x72, a.pattern_anim_count);
    w.u16(o + 0x76, nt as u16);
    for (i, m) in a.materials.iter().enumerate() {
        let md = mats + 0x40 * i;
        w.s(md, &m.name);
        if !m.params.is_empty() {
            let p = w.alloc(0x18 * m.params.len(), 8);
            for (k, info) in m.params.iter().enumerate() {
                let e = p + 0x18 * k;
                w.s(e, &info.name);
                for (j, v) in [info.begin_curve, info.float_curves, info.int_curves, info.begin_constant, info.constants,
                               info.sub_bind].into_iter().enumerate() {
                    w.u16(e + 8 + 2 * j, v);
                }
            }
            w.at(md + 8, p);
        }
        if !m.patterns.is_empty() {
            let p = w.alloc(0x10 * m.patterns.len(), 8);
            for (k, info) in m.patterns.iter().enumerate() {
                let e = p + 0x10 * k;
                w.s(e, &info.name);
                w.u16(e + 8, info.curve as u16);
                w.u16(e + 0xa, info.begin_constant as u16);
                w.u8(e + 0xc, info.sub_bind as u8);
            }
            w.at(md + 0x10, p);
        }
        if !m.constants.is_empty() {
            let c = w.alloc(8 * m.constants.len(), 8);
            for (k, (target, value)) in m.constants.iter().enumerate() {
                w.u32(c + 8 * k, *target);
                w.u32(c + 8 * k + 4, *value);
            }
            w.at(md + 0x20, c);
        }
        if !m.curves.is_empty() {
            let c = w.alloc(0x30 * m.curves.len(), 8);
            w.at(md + 0x18, c);
            for (k, curve) in m.curves.iter().enumerate() {
                write_curve(w, c + 0x30 * k, curve);
            }
        }
        w.u16(md + 0x28, m.param_curve_begin as u16);
        w.u16(md + 0x2a, m.pattern_curve_begin as u16);
        for (k, v) in m.visibility.iter().enumerate() {
            w.u16(md + 0x2c + 2 * k, *v as u16);
        }
        w.u16(md + 0x32, m.params.len() as u16);
        w.u16(md + 0x34, m.patterns.len() as u16);
        w.u16(md + 0x36, m.constants.len() as u16);
        w.u16(md + 0x38, m.curves.len() as u16);
    }
}

fn write_curve(w: &mut W, c: usize, k: &Curve) {
    let frames = w.data(&k.frames, 8);
    w.at(c, frames);
    let keys = w.data(&k.keys, 8);
    w.at(c + 8, keys);
    w.u16(c + 0x10, k.flags);
    w.u16(c + 0x12, k.key_count);
    w.u32(c + 0x14, k.target);
    w.put(c + 0x18, &floats(&[k.start, k.end]));
    w.u32(c + 0x20, k.scale);
    w.u32(c + 0x24, k.offset);
    w.put(c + 0x28, &floats(&[k.delta]));
}

fn write_model(w: &mut W, o: usize, m: &Model) -> Result<()> {
    w.block(o, b"FMDL");
    w.s(o + 0x10, &m.name);
    w.s(o + 0x18, &m.path);
    let vtx = w.alloc(0x60 * m.vertex_buffers.len(), 8);
    let mat = w.alloc(0xb8 * m.materials.len(), 8);
    let shp = w.alloc(0x70 * m.shapes.len(), 8);
    let hollow = m.vertex_buffers.is_empty() && m.shapes.is_empty() && m.materials.is_empty();
    let nothing = hollow && m.user_data.is_empty();
    let list = |w: &mut W, field: usize, at: usize, n: usize| {
        if n > 0 {
            w.at(field, at)
        } else if nothing {
            w.ptr(field, To::NullListed)
        } else {
            let here = align(w.b.len(), 8);
            w.at(field, here)
        }
    };
    list(w, o + 0x28, vtx, m.vertex_buffers.len());
    list(w, o + 0x30, shp, m.shapes.len());
    list(w, o + 0x40, mat, m.materials.len());
    let user_array = if m.user_data.is_empty() {
        if hollow {
            w.ptr(o + 0x50, To::NullListed);
        } else {
            let here = align(w.b.len(), 8);
            w.at(o + 0x50, here);
        }
        w.ptr(o + 0x58, To::NullListed);
        None
    } else {
        Some(w.user_data_array(&m.user_data, o + 0x50))
    };
    let skl = w.alloc(0x48, 8);
    w.at(o + 0x20, skl);
    w.u16(o + 0x68, m.vertex_buffers.len() as u16);
    w.u16(o + 0x6a, m.shapes.len() as u16);
    w.u16(o + 0x6c, m.materials.len() as u16);
    w.u16(o + 0x6e, m.user_data.len() as u16);
    w.u32(o + 0x70, m.total_vertices);
    write_skeleton(w, skl, &m.skeleton)?;
    for (field, names) in [
        (o + 0x38, m.shapes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()),
        (o + 0x48, m.materials.iter().map(|s| s.name.as_str()).collect()),
    ] {
        if names.is_empty() {
            w.ptr(field, To::NullListed);
        } else {
            let d = w.dict(&names);
            w.at(field, d);
        }
    }
    if user_array.is_some() {
        w.user_data_dict(&m.user_data, o + 0x58);
    }
    for (i, s) in m.shapes.iter().enumerate() {
        let v = s.vertex_buffer as usize;
        if v >= m.vertex_buffers.len() {
            bail!("shape {} uses a vertex buffer that isn't there", s.name);
        }
        write_shape(w, shp + 0x70 * i, s, vtx + 0x60 * v)?;
    }
    for (i, v) in m.vertex_buffers.iter().enumerate() {
        write_vertex_buffer(w, vtx + 0x60 * i, v, i as u16)?;
    }
    if let Some(arr) = user_array {
        w.user_data_values(&m.user_data, arr);
    }
    for (i, mt) in m.materials.iter().enumerate() {
        write_material(w, mat + 0xb8 * i, mt, i as u16)?;
    }
    Ok(())
}

fn data_or_here(w: &mut W, field: usize, v: &[u8]) {
    let at = if v.is_empty() { align(w.b.len(), 8) } else { w.data(v, 8) };
    w.at(field, at);
}

fn write_skeleton(w: &mut W, o: usize, s: &Skeleton) -> Result<()> {
    w.block(o, b"FSKL");
    let bones = w.alloc(0x50 * s.bones.len(), 8);
    w.at(o + 0x18, bones);
    w.u32(o + 0x38, s.flags);
    w.u16(o + 0x3c, s.bones.len() as u16);
    w.u16(o + 0x3e, s.smooth_count);
    w.u16(o + 0x40, s.rigid_count);
    for (i, b) in s.bones.iter().enumerate() {
        let p = bones + 0x50 * i;
        w.s(p, &b.name);
        if b.user_data.is_empty() {
            w.ptr(p + 8, To::NullListed);
        }
        w.user_data(&b.user_data, p + 8, p + 0x10);
        w.u16(p + 0x18, b.index);
        w.u16(p + 0x1a, b.parent);
        w.u16(p + 0x1c, b.smooth as u16);
        w.u16(p + 0x1e, b.rigid as u16);
        w.u16(p + 0x20, b.billboard as u16);
        w.u16(p + 0x22, b.user_data.len() as u16);
        w.u32(p + 0x24, b.flags);
        w.put(p + 0x28, &floats(&b.srt));
    }
    let list: Vec<u8> = s.matrix_to_bone.iter().flat_map(|x| x.to_le_bytes()).collect();
    data_or_here(w, o + 0x20, &list);
    data_or_here(w, o + 0x28, &floats(&s.inverse));
    let names: Vec<&str> = s.bones.iter().map(|b| b.name.as_str()).collect();
    let d = w.dict(&names);
    w.at(o + 0x10, d);
    Ok(())
}

fn write_vertex_buffer(w: &mut W, o: usize, v: &VertexBuffer, index: u16) -> Result<()> {
    w.put(o, b"FVTX");
    let nb = v.buffers.len();
    let attrs = w.alloc(0x10 * v.attributes.len(), 8);
    for (i, a) in v.attributes.iter().enumerate() {
        let p = attrs + 0x10 * i;
        w.s(p, &a.name);
        w.u32(p + 8, a.format);
        w.u16(p + 0xc, a.offset);
        w.u16(p + 0xe, a.buffer as u16);
    }
    w.at(o + 0x10, attrs);
    w.ptr(o + 0x20, To::Pool);
    let runtime = w.alloc(0x48 * nb, 8);
    let sizes = w.alloc(0x10 * nb, 8);
    let strides = w.alloc(0x10 * nb, 8);
    w.alloc(0x20, 8);
    let user = w.alloc(8 * nb, 8);
    w.at(o + 0x28, runtime);
    w.at(o + 0x30, user);
    w.at(o + 0x38, sizes);
    w.at(o + 0x40, strides);
    let names: Vec<&str> = v.attributes.iter().map(|a| a.name.as_str()).collect();
    let d = w.dict(&names);
    w.at(o + 0x18, d);
    let start = align(w.vertex_data.len(), 8);
    w.vertex_data.resize(start, 0);
    for (i, b) in v.buffers.iter().enumerate() {
        let at = align(w.vertex_data.len(), 8);
        w.vertex_data.resize(at, 0);
        w.vertex_data.extend_from_slice(&b.data);
        w.u32(sizes + 0x10 * i, b.data.len() as u32);
        w.u32(strides + 0x10 * i, b.stride);
    }
    w.u32(o + 0x50, start as u32);
    w.vertex_offsets.push(o + 0x50);
    w.u8(o + 0x54, v.attributes.len() as u8);
    w.u8(o + 0x55, nb as u8);
    w.u16(o + 0x56, index);
    w.u32(o + 0x58, v.vertex_count);
    w.u8(o + 0x5c, v.skin_count);
    Ok(())
}

fn write_shape(w: &mut W, o: usize, s: &Shape, vtx: usize) -> Result<()> {
    w.put(o, b"FSHP");
    w.s(o + 0x10, &s.name);
    w.at(o + 0x18, vtx);
    let meshes = w.alloc(0x38 * s.meshes.len(), 8);
    w.at(o + 0x20, meshes);
    let skin: Vec<u8> = s.skin_bones.iter().flat_map(|x| x.to_le_bytes()).collect();
    data_or_here(w, o + 0x28, &skin);
    let bb = w.data(&floats(&s.bounding_boxes), 8);
    w.at(o + 0x40, bb);
    let rd = w.data(&floats(&s.radii), 8);
    w.at(o + 0x48, rd);
    w.u32(o + 0x58, s.flags);
    w.u16(o + 0x5c, s.index);
    w.u16(o + 0x5e, s.material);
    w.u16(o + 0x60, s.bone);
    w.u16(o + 0x62, s.vertex_buffer);
    w.u16(o + 0x64, s.skin_bones.len() as u16);
    w.u8(o + 0x66, s.vertex_skin_count);
    w.u8(o + 0x67, s.meshes.len() as u8);
    w.u8(o + 0x69, s.target_attributes);
    for (k, m) in s.meshes.iter().enumerate() {
        let p = meshes + 0x38 * k;
        let subs: Vec<u8> = m.sub_meshes.iter().flat_map(|(a, b)| [a.to_le_bytes(), b.to_le_bytes()].concat()).collect();
        let sub = w.data(&subs, 8);
        let runtime = w.alloc(0x48, 8);
        let size = w.alloc(0x30, 8);
        w.u32(size, m.indices.len() as u32);
        w.at(p, sub);
        w.ptr(p + 8, To::Pool);
        w.at(p + 0x10, runtime);
        w.at(p + 0x18, size);
        let at = align(w.index_data.len(), 8);
        w.index_data.resize(at, 0);
        w.index_data.extend_from_slice(&m.indices);
        w.u32(p + 0x20, at as u32);
        w.u32(p + 0x24, m.primitive);
        w.u32(p + 0x28, m.index_format);
        w.u32(p + 0x2c, m.index_count);
        w.u32(p + 0x30, m.first_vertex);
        w.u16(p + 0x34, m.sub_meshes.len() as u16);
    }
    Ok(())
}

fn write_material(w: &mut W, o: usize, m: &Material, index: u16) -> Result<()> {
    w.block(o, b"FMAT");
    w.s(o + 0x10, &m.name);
    let (nt, ns, np) = (m.textures.len(), m.samplers.len(), m.params.len());
    let ri = w.alloc(0x18 * m.render_infos.len().max(1), 8);
    for (i, r) in m.render_infos.iter().enumerate() {
        let p = ri + 0x18 * i;
        w.s(p, &r.name);
        let (count, kind) = match &r.value {
            RenderValue::Int(v) => (v.len(), 0u8),
            RenderValue::Float(v) => (v.len(), 1),
            RenderValue::Str(v) => (v.len(), 2),
        };
        w.u16(p + 0x10, count as u16);
        w.u8(p + 0x12, kind);
    }
    for strings in [true, false] {
        for (i, r) in m.render_infos.iter().enumerate() {
            let data = match &r.value {
                RenderValue::Str(v) if strings && !v.is_empty() => w.strings(v),
                RenderValue::Int(v) if !strings && !v.is_empty() => {
                    w.data(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>(), 4)
                }
                RenderValue::Float(v) if !strings && !v.is_empty() => w.data(&floats(v), 4),
                _ => continue,
            };
            w.at(ri + 0x18 * i + 8, data);
        }
    }
    w.at(o + 0x18, ri);
    if nt > 0 {
        let rt = w.alloc(8 * nt, 8);
        w.at(o + 0x30, rt);
    }
    if ns > 0 {
        let info = w.alloc(0x20 * ns, 8);
        for (i, (_, s)) in m.samplers.iter().enumerate() {
            w.put(info + 0x20 * i, s);
        }
        w.at(o + 0x48, info);
        let rt = w.alloc(0x78 * ns, 8);
        w.at(o + 0x40, rt);
    } else {
        let here = align(w.b.len(), 8);
        w.at(o + 0x48, here);
        w.at(o + 0x40, here);
    }
    if np > 0 {
        let pa = w.alloc(0x20 * np, 8);
        for (i, p) in m.params.iter().enumerate() {
            let q = pa + 0x20 * i;
            w.s(q + 8, &p.name);
            w.u8(q + 0x10, p.kind);
            w.u8(q + 0x11, p.size);
            w.u16(q + 0x12, p.offset);
            w.u32(q + 0x14, p.unknown as u32);
            w.u16(q + 0x18, p.depended);
            w.u16(q + 0x1a, p.depend);
        }
        w.at(o + 0x58, pa);
        let data = w.data(&m.param_data, 8);
        w.at(o + 0x68, data);
    }
    let vol = w.alloc(np.div_ceil(64).max(1) * 8, 8);
    w.at(o + 0x80, vol);
    if m.user_data.is_empty() {
        let here = align(w.b.len(), 8);
        w.at(o + 0x70, here);
        w.ptr(o + 0x78, To::NullListed);
    } else {
        w.user_data(&m.user_data, o + 0x70, o + 0x78);
    }
    if ns > 0 {
        let s = w.alloc(8 * ns, 8);
        w.put(s, &vec![0xff; 8 * ns]);
        w.at(o + 0x90, s);
    }
    if nt > 0 {
        let t = w.alloc(8 * nt, 8);
        w.put(t, &vec![0xff; 8 * nt]);
        w.at(o + 0x98, t);
        let tn = w.strings(&m.textures);
        w.at(o + 0x38, tn);
    }
    let sa = w.alloc(0x48, 8);
    let a = &m.shader_assign;
    w.s(sa, &a.archive);
    w.s(sa + 8, &a.model);
    let lists = [&a.attributes, &a.samplers, &a.options];
    for (k, list) in lists.iter().enumerate() {
        let values: Vec<String> = list.iter().map(|p| p.1.clone()).collect();
        let arr = if values.is_empty() { align(w.b.len(), 8) } else { w.strings(&values) };
        w.at(sa + 0x10 + 0x10 * k, arr);
    }
    for (k, list) in lists.iter().enumerate() {
        let keys: Vec<&str> = list.iter().map(|p| p.0.as_str()).collect();
        if keys.is_empty() {
            w.ptr(sa + 0x18 + 0x10 * k, To::NullListed);
        } else {
            let d = w.dict(&keys);
            w.at(sa + 0x18 + 0x10 * k, d);
        }
    }
    w.u32(sa + 0x40, a.revision);
    w.u8(sa + 0x44, a.attributes.len() as u8);
    w.u8(sa + 0x45, a.samplers.len() as u8);
    w.u16(sa + 0x46, a.options.len() as u16);
    w.at(o + 0x28, sa);
    let names: Vec<&str> = m.render_infos.iter().map(|r| r.name.as_str()).collect();
    let d = w.dict(&names);
    w.at(o + 0x20, d);
    if ns > 0 {
        let names: Vec<&str> = m.samplers.iter().map(|s| s.0.as_str()).collect();
        let d = w.dict(&names);
        w.at(o + 0x50, d);
    } else {
        w.ptr(o + 0x50, To::NullListed);
    }
    if np > 0 {
        let names: Vec<&str> = m.params.iter().map(|p| p.name.as_str()).collect();
        let d = w.dict(&names);
        w.at(o + 0x60, d);
    }
    w.u32(o + 0xa0, m.flags);
    w.u16(o + 0xa4, index);
    w.u16(o + 0xa6, m.render_infos.len() as u16);
    w.u8(o + 0xa8, ns as u8);
    w.u8(o + 0xa9, nt as u8);
    w.u16(o + 0xaa, np as u16);
    w.u16(o + 0xac, m.volatile_count);
    w.u16(o + 0xae, m.param_data.len() as u16);
    w.u16(o + 0xb0, m.raw_param_size);
    w.u16(o + 0xb2, m.user_data.len() as u16);
    Ok(())
}

fn ext_align(file: &File) -> usize {
    if file.external.iter().any(|(_, d)| d.starts_with(b"BNTX")) { 0x1000 } else { 0x100 }
}

fn finish(mut w: W, file: &File, buffer_info: Option<usize>) -> Result<Vec<u8>> {
    let mut order: Vec<String> = vec![String::new()];
    let mut seen: HashMap<String, usize> = HashMap::new();
    seen.insert(String::new(), 0);
    for (_, to) in &w.ptrs {
        if let To::Str(s) = to
            && !seen.contains_key(s) {
                seen.insert(s.clone(), order.len());
                order.push(s.clone());
            }
    }
    let pool = align(w.b.len(), 8);
    w.b.resize(pool + 0x14, 0);
    w.put(pool, b"_STR");
    w.blocks.push(pool);
    w.u32(pool + 0x10, (order.len() - 1) as u32);
    let mut pos = Vec::with_capacity(order.len());
    for s in &order {
        let at = w.b.len();
        pos.push(at);
        w.b.extend_from_slice(&(s.len() as u16).to_le_bytes());
        w.b.extend_from_slice(s.as_bytes());
        w.b.push(0);
        if w.b.len() % 2 == 1 {
            w.b.push(0);
        }
    }
    let pool_end = w.b.len();
    w.ptr(0xb0, To::At(pool + 0x14));
    let str_pos = |s: &str| pos[seen[s]];
    let index_len = align(w.index_data.len(), 8);
    let (gpu, region, mem_pool, ext, rlt, mut ext_at) = match buffer_info {
        None if !file.external.is_empty() => {
            let one = |d: &[u8]| if d.starts_with(b"BNTX") { 0x1000 } else { 0x100 };
            let mut prev = one(&file.external[0].1);
            let ext = align(pool_end, prev);
            w.b.resize(ext, 0);
            let mut ext_at = Vec::new();
            for (_, data) in &file.external {
                let at = align(w.b.len(), prev.max(one(data)));
                w.b.resize(at, 0);
                ext_at.push(at);
                w.b.extend_from_slice(data);
                prev = one(data);
            }
            let rlt = align(w.b.len(), prev);
            w.b.resize(rlt, 0);
            (pool_end, 0, pool_end, ext, rlt, ext_at)
        }
        None => {
            let end = align(pool_end, 8);
            w.b.resize(end, 0);
            (pool_end, 0, pool_end, pool_end, end, Vec::new())
        }
        Some(buffer_info) => {
            let gpu = align(pool_end, 0x1000);
            for f in w.vertex_offsets.clone() {
                let v = u32::from_le_bytes(w.b[f..f + 4].try_into()?) + index_len as u32;
                w.u32(f, v);
            }
            let used = index_len + w.vertex_data.len();
            let region = align(used.max(1), 0x1000);
            w.b.resize(gpu, 0);
            w.b.extend_from_slice(&w.index_data.clone());
            w.b.resize(gpu + index_len, 0);
            w.b.extend_from_slice(&w.vertex_data.clone());
            w.b.resize(gpu + region, 0);
            w.u32(buffer_info + 4, region as u32);
            let mem_pool = gpu + region;
            w.b.resize(mem_pool + 0x120, 0);
            let ext = if file.external.is_empty() { w.b.len() } else { align(w.b.len(), 0x100) };
            w.b.resize(ext, 0);
            let mut ext_at = Vec::new();
            for (_, data) in &file.external {
                let at = align(w.b.len(), 0x100);
                w.b.resize(at, 0);
                ext_at.push(at);
                w.b.extend_from_slice(data);
            }
            let rlt = if file.external.is_empty() { align(w.b.len(), 8) } else { align(w.b.len(), 0x100) };
            w.b.resize(rlt, 0);
            (gpu, region, mem_pool, ext, rlt, ext_at)
        }
    };
    let ext_at = std::mem::take(&mut ext_at);
    let mut by_section: [Vec<usize>; 5] = Default::default();
    for (o, to) in w.ptrs.clone() {
        let (value, section) = match &to {
            To::At(t) => (*t, 0),
            To::Str(s) => (str_pos(s), 0),
            To::Gpu(x) => (gpu + x, if *x < index_len { 1 } else { 2 }),
            To::Pool => (mem_pool, 3),
            To::Ext(i) => (ext_at[*i], 4),
            To::NullListed => (0, 0),
        };
        w.b[o..o + 8].copy_from_slice(&(value as u64).to_le_bytes());
        by_section[section].push(o);
    }
    let mut blocks = w.blocks.clone();
    blocks.sort_unstable();
    for (i, b) in blocks.iter().enumerate() {
        let next = blocks.get(i + 1).copied();
        w.u32(b + 4, next.map_or(0, |n| n - b) as u32);
        w.u32(b + 8, (next.unwrap_or(rlt) - b) as u32);
    }
    w.put(0, b"FRES    ");
    let alignment = match (buffer_info.is_some(), file.external.is_empty()) {
        (true, _) => 0x0c,
        (false, false) => ext_align(file).trailing_zeros() as u8,
        (false, true) => 0x03,
    };
    w.put(8, &[0x03, 0x00, 0x05, 0x00, 0xff, 0xfe, alignment, 0x00]);
    w.u32(0x10, (str_pos(&file.name) + 2) as u32);
    w.u16(0x16, blocks.first().copied().unwrap_or(pool) as u16);
    w.u32(0x18, rlt as u32);
    w.u32(0xb8, (pool_end - pool) as u32);
    let sections = [
        (0usize, pool_end),
        (gpu, index_len),
        (gpu + index_len, region - index_len),
        (mem_pool, if buffer_info.is_some() { 0x120 } else { 0 }),
        (ext, if buffer_info.is_some() || !file.external.is_empty() { rlt - ext } else { 0 }),
    ];
    let mut entries: Vec<(u32, u16, u8, u8)> = Vec::new();
    let mut table = Vec::new();
    for (s, list) in by_section.iter_mut().enumerate() {
        list.sort_unstable();
        list.dedup();
        let first = entries.len();
        entries.extend(super::relocation_entries(list));
        table.push((sections[s].0, sections[s].1, first, entries.len() - first));
    }
    let mut r = Vec::new();
    r.extend_from_slice(b"_RLT");
    r.extend_from_slice(&(rlt as u32).to_le_bytes());
    r.extend_from_slice(&5u32.to_le_bytes());
    r.extend_from_slice(&0u32.to_le_bytes());
    for (pos, size, first, count) in table {
        r.extend_from_slice(&0u64.to_le_bytes());
        r.extend_from_slice(&(pos as u32).to_le_bytes());
        r.extend_from_slice(&(size as u32).to_le_bytes());
        r.extend_from_slice(&(first as u32).to_le_bytes());
        r.extend_from_slice(&(count as u32).to_le_bytes());
    }
    for (pos, structs, n, pad) in entries {
        r.extend_from_slice(&pos.to_le_bytes());
        r.extend_from_slice(&structs.to_le_bytes());
        r.extend_from_slice(&[n, pad]);
    }
    w.b.extend_from_slice(&r);
    let size = w.b.len() as u32;
    w.u32(0x1c, size);
    Ok(w.b)
}
