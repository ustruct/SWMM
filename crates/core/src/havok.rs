use std::collections::HashMap;

use anyhow_ext::{Context, Result, bail};

struct Object<'a> {
    class: String,
    at: usize,
    size: usize,
    bytes: &'a [u8],
    local: HashMap<usize, usize>,
}

struct Packfile<'a> {
    header: &'a [u8],
    classes: Vec<(u32, String)>,
    objects: Vec<Object<'a>>,
    globals: Vec<(usize, usize, usize, usize)>,
    len: usize,
}

fn be32(d: &[u8], o: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("the Havok file is cut short")?.try_into()?) as usize)
}

fn read_packfile(d: &[u8]) -> Result<Packfile<'_>> {
    if d.get(..8) != Some(&[0x57, 0xe0, 0xe0, 0x57, 0x10, 0xc0, 0xc0, 0x10]) {
        bail!("not a Havok packfile");
    }
    if d.get(0x10..0x12) != Some(&[4, 0]) {
        bail!("not a Wii U Havok packfile (4-byte pointers, big endian)");
    }
    if d.get(0x28..0x36) != Some(b"hk_2014.2.0-r1") {
        bail!("Havok version {} isn't supported", String::from_utf8_lossy(d.get(0x28..0x36).unwrap_or_default()));
    }
    let predicate = u16::from_be_bytes([d[0x3e], d[0x3f]]) as usize;
    let header_len = 0x40 + predicate;
    let section = |i: usize| -> Result<[usize; 7]> {
        let o = header_len + 0x40 * i;
        let mut v = [0; 7];
        for (k, x) in v.iter_mut().enumerate() {
            *x = be32(d, o + 20 + 4 * k)?;
        }
        Ok(v)
    };
    let names = section(0)?;
    let data = section(2)?;
    let mut classes = Vec::new();
    let mut by_offset = HashMap::new();
    let mut o = names[0];
    while o + 5 < names[0] + names[6] && d[o] != 0xff {
        let sig = be32(d, o)? as u32;
        let start = o + 5;
        let end = start + d[start..].iter().position(|&b| b == 0).context("a class name without its end")?;
        let name = String::from_utf8_lossy(&d[start..end]).into_owned();
        by_offset.insert(start - names[0], classes.len());
        classes.push((sig, name));
        o = end + 1;
    }
    let base = data[0];
    let fixups = |from: usize, to: usize, size: usize| -> Result<Vec<Vec<usize>>> {
        let mut out = Vec::new();
        let mut o = base + from;
        while o + size <= base + to {
            if d[o] == 0xff {
                break;
            }
            out.push((0..size / 4).map(|k| be32(d, o + 4 * k)).collect::<Result<Vec<_>>>()?);
            o += size;
        }
        Ok(out)
    };
    let locals = fixups(data[1], data[2], 8)?;
    let globals_raw = fixups(data[2], data[3], 12)?;
    let virtuals = fixups(data[3], data[4], 12)?;
    let mut objects: Vec<Object> = Vec::new();
    for (i, v) in virtuals.iter().enumerate() {
        let at = v[0];
        let end = virtuals.get(i + 1).map_or(data[1], |n| n[0]);
        let class = by_offset.get(&v[2]).map(|&k| classes[k].1.clone()).context("an object of no known class")?;
        d.get(base + at..base + end).context("an object is cut short")?;
        objects.push(Object {
            class,
            at,
            size: end - at,
            bytes: &d[base + at..],
            local: HashMap::new(),
        });
    }
    let spans: Vec<(usize, usize)> = objects.iter().map(|o| (o.at, o.at + o.size)).collect();
    let owner = |pos: usize| spans.iter().position(|&(a, b)| pos >= a && pos < b);
    let mut local_by_obj: Vec<Vec<(usize, usize)>> = vec![vec![]; objects.len()];
    for l in &locals {
        if let Some(k) = owner(l[0]) {
            local_by_obj[k].push((l[0] - objects[k].at, l[1].wrapping_sub(objects[k].at)));
        }
    }
    for (k, ls) in local_by_obj.into_iter().enumerate() {
        objects[k].local = ls.into_iter().collect();
    }
    let mut globals = Vec::new();
    for g in &globals_raw {
        let (Some(s), Some(t)) = (owner(g[0]), spans.iter().position(|&(a, _)| a == g[2]).or_else(|| owner(g[2])))
        else {
            bail!("a pointer to no object");
        };
        globals.push((s, g[0] - objects[s].at, t, g[2] - objects[t].at));
    }
    Ok(Packfile { header: &d[..header_len], classes, objects, globals, len: base + data[6] })
}

struct Ctx<'a> {
    r: &'a [u8],
    rp: usize,
    local: &'a HashMap<usize, usize>,
    w: Vec<u8>,
    slocal: Vec<(usize, usize)>,
    pointers: HashMap<usize, usize>,
    eof_at: Option<usize>,
    positions: HashMap<usize, usize>,
}

impl Ctx<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        self.positions.entry(self.rp).or_insert(self.w.len());
        let s = self.r.get(self.rp..self.rp + n).with_context(|| format!("the object is cut short at {:#x}", self.rp))?;
        self.rp += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        let v = self.take(1)?[0];
        self.w.push(v);
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        let v = u16::from_be_bytes(self.take(2)?.try_into()?);
        self.w.extend(v.to_le_bytes());
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32> {
        let v = u32::from_be_bytes(self.take(4)?.try_into()?);
        self.w.extend(v.to_le_bytes());
        Ok(v)
    }
    fn u64(&mut self) -> Result<u64> {
        let v = u64::from_be_bytes(self.take(8)?.try_into()?);
        self.w.extend(v.to_le_bytes());
        Ok(v)
    }
    fn n32(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.u32()?;
        }
        Ok(())
    }
    fn n8(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.u8()?;
        }
        Ok(())
    }
    fn vec4(&mut self, n: usize) -> Result<()> {
        self.n32(4 * n)
    }
    fn ptr(&mut self) -> Result<(usize, usize)> {
        let (wo, so) = (self.rp, self.w.len());
        self.take(4)?;
        self.w.extend([0; 8]);
        self.pointers.insert(wo, so);
        Ok((wo, so))
    }
    fn word(&mut self) -> Result<()> {
        let v = u32::from_be_bytes(self.take(4)?.try_into()?);
        self.w.extend((v as u64).to_le_bytes());
        Ok(())
    }
    fn counter(&mut self) -> Result<usize> {
        let n = self.u32()?;
        self.u32()?;
        Ok(n as usize)
    }
    fn array(&mut self) -> Result<((usize, usize), usize)> {
        let p = self.ptr()?;
        Ok((p, self.counter()?))
    }
    fn align(&mut self, n: usize) {
        self.ralign(n);
        self.walign(n);
        self.positions.entry(self.rp).or_insert(self.w.len());
    }
    fn ralign(&mut self, n: usize) {
        self.rp = self.rp.div_ceil(n) * n;
    }
    fn walign(&mut self, n: usize) {
        while !self.w.len().is_multiple_of(n) {
            self.w.push(0);
        }
    }
    fn wpad4(&mut self) {
        self.w.extend([0; 4]);
    }
    fn cstr(&mut self) -> Result<()> {
        let n = self.r.get(self.rp..).and_then(|s| s.iter().position(|&b| b == 0)).context("a string without its end")?;
        let s = self.take(n + 1)?.to_vec();
        self.w.extend(s);
        Ok(())
    }
    fn at(&mut self, p: (usize, usize), always: bool, f: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let _ = always;
        let dst = self.local.get(&p.0).copied();
        if dst.is_none() {
            return Ok(());
        }
        self.slocal.push((p.1, self.w.len()));
        let saved = self.rp;
        if let Some(d) = dst {
            self.rp = d;
            f(self)?;
        }
        self.rp = saved;
        Ok(())
    }
}

fn referenced(c: &mut Ctx) -> Result<()> {
    c.ptr()?;
    c.u32()?;
    Ok(())
}

fn hkp_shape(c: &mut Ctx) -> Result<()> {
    referenced(c)?;
    c.n8(4)?;
    c.word()
}

fn convex(c: &mut Ctx) -> Result<()> {
    hkp_shape(c)?;
    c.u32()?;
    Ok(())
}

fn bv_tree(c: &mut Ctx) -> Result<()> {
    hkp_shape(c)?;
    c.u8()?;
    c.align(4);
    Ok(())
}

fn aabb(c: &mut Ctx) -> Result<()> {
    c.vec4(2)
}

fn shape_object(c: &mut Ctx, class: &str) -> Result<()> {
    match class {
        "hkpBoxShape" => {
            convex(c)?;
            c.align(16);
            c.vec4(1)
        }
        "hkpCapsuleShape" => {
            convex(c)?;
            c.align(16);
            c.vec4(2)
        }
        "hkpSphereShape" => {
            convex(c)?;
            c.n32(3)?;
            c.align(16);
            Ok(())
        }
        "hkpCylinderShape" => {
            convex(c)?;
            c.n32(2)?;
            c.align(16);
            c.vec4(4)?;
            c.align(16);
            Ok(())
        }
        "hkpConvexTransformShape" | "hkpConvexTranslateShape" => {
            convex(c)?;
            c.walign(8);
            c.ptr()?;
            c.ptr()?;
            c.u32()?;
            c.align(8);
            if class == "hkpConvexTransformShape" {
                c.align(16);
                c.vec4(3)?;
                c.vec4(1)
            } else {
                c.align(16);
                c.vec4(1)
            }
        }
        "hkpConvexVerticesShape" => {
            convex(c)?;
            c.align(16);
            c.vec4(2)?;
            let (rotated, n_rot) = c.array()?;
            c.u32()?;
            c.u8()?;
            c.align(4);
            let (planes, n_planes) = c.array()?;
            let (conn, _) = (c.ptr()?, 0);
            if c.local.contains_key(&conn.0) {
                bail!("convex shapes with connectivity aren't supported");
            }
            c.align(16);
            c.at(rotated, true, |c| c.vec4(3 * n_rot))?;
            c.walign(16);
            c.at(planes, true, |c| c.vec4(n_planes))
        }
        "hkpListShape" => {
            hkp_shape(c)?;
            c.align(16);
            c.u8()?;
            c.u8()?;
            c.align(8);
            let (children, n) = c.array()?;
            c.u16()?;
            c.u16()?;
            c.align(16);
            c.vec4(2)?;
            c.n32(8)?;
            if n > 0 {
                c.at(children, false, |c| {
                    for _ in 0..n {
                        c.ptr()?;
                        c.u32()?;
                        c.u16()?;
                        c.u16()?;
                        c.u32()?;
                        c.align(16);
                    }
                    Ok(())
                })?;
            }
            Ok(())
        }
        "hkpBvCompressedMeshShape" => bv_compressed_mesh(c),
        "hkpStaticCompoundShape" => static_compound_shape(c),
        k => bail!("Havok class {k} isn't supported"),
    }
}

fn bv_compressed_mesh(c: &mut Ctx) -> Result<()> {
    bv_tree(c)?;
    c.walign(16);
    c.ptr()?;
    c.u32()?;
    c.n8(3)?;
    c.align(4);
    let (filters, n_filters) = c.array()?;
    let (user, n_user) = c.array()?;
    let (strings, n_strings) = c.array()?;
    c.align(16);
    let (nodes, n_nodes) = c.array()?;
    c.align(16);
    aabb(c)?;
    c.n32(3)?;
    c.walign(16);
    let (sections, n_sections) = c.array()?;
    let (prims, n_prims) = c.array()?;
    let (shared_idx, n_shared_idx) = c.array()?;
    let (packed, n_packed) = c.array()?;
    let (shared, n_shared) = c.array()?;
    let (runs, n_runs) = c.array()?;
    c.walign(16);
    c.at(filters, true, |c| c.n32(n_filters))?;
    c.walign(16);
    c.at(user, true, |c| c.n32(n_user))?;
    c.walign(16);
    if n_strings > 0 {
        c.at(strings, false, |c| {
            let ptrs = (0..n_strings).map(|_| c.ptr()).collect::<Result<Vec<_>>>()?;
            for p in ptrs {
                c.slocal.push((p.1, c.w.len()));
                c.cstr()?;
                c.align(2);
            }
            c.walign(16);
            Ok(())
        })?;
    }
    if n_nodes > 0 {
        c.at(nodes, false, |c| c.n8(5 * n_nodes))?;
        c.walign(16);
    }
    if n_sections > 0 {
        let mut section_nodes = Vec::new();
        c.at(sections, false, |c| {
            for _ in 0..n_sections {
                let (p, n) = c.array()?;
                c.align(16);
                aabb(c)?;
                section_nodes.push((p, n));
                c.n32(3 * 2)?;
                c.n32(4)?;
                c.n8(2)?;
                c.u16()?;
                c.n8(4)?;
            }
            Ok(())
        })?;
        c.walign(16);
        for (p, n) in section_nodes {
            if n > 0 {
                c.at(p, false, |c| c.n8(4 * n))?;
                c.walign(16);
            }
        }
    }
    if n_prims > 0 {
        c.at(prims, false, |c| c.n8(4 * n_prims))?;
        c.walign(16);
    }
    if n_shared_idx > 0 {
        c.at(shared_idx, false, |c| {
            for _ in 0..n_shared_idx {
                c.u16()?;
            }
            Ok(())
        })?;
        c.walign(16);
    }
    if n_packed > 0 {
        c.at(packed, false, |c| c.n32(n_packed))?;
        c.walign(16);
    }
    if n_shared > 0 {
        c.at(shared, false, |c| {
            for _ in 0..n_shared {
                c.u64()?;
            }
            Ok(())
        })?;
        c.walign(16);
    }
    if n_runs > 0 {
        c.at(runs, false, |c| {
            for _ in 0..n_runs {
                c.u32()?;
                c.n8(2)?;
                c.align(4);
            }
            Ok(())
        })?;
        c.walign(16);
    }
    Ok(())
}

fn static_compound_shape(c: &mut Ctx) -> Result<()> {
    bv_tree(c)?;
    c.walign(16);
    c.ptr()?;
    c.n8(2)?;
    c.align(4);
    c.u32()?;
    let (instances, n_inst) = c.array()?;
    let (extra, n_extra) = c.array()?;
    c.ptr()?;
    c.u32()?;
    c.walign(16);
    let (nodes, n_nodes) = c.array()?;
    c.align(16);
    aabb(c)?;
    if n_inst > 0 {
        c.at(instances, false, |c| {
            for _ in 0..n_inst {
                c.vec4(3)?;
                c.ptr()?;
                c.n32(2)?;
                c.word()?;
                c.walign(16);
            }
            Ok(())
        })?;
    }
    if n_extra > 0 {
        c.at(extra, false, |c| {
            for _ in 0..n_extra {
                c.u16()?;
            }
            Ok(())
        })?;
    }
    if n_nodes > 0 {
        c.at(nodes, false, |c| {
            for _ in 0..n_nodes {
                c.n8(4)?;
                c.u16()?;
            }
            Ok(())
        })?;
    }
    c.walign(16);
    Ok(())
}

fn rigid_body(c: &mut Ctx) -> Result<()> {
    referenced(c)?;
    c.walign(16);
    c.ptr()?;
    c.word()?;
    c.ptr()?;
    c.u32()?;
    c.walign(16);
    c.ptr()?;
    c.ptr()?;
    c.u8()?;
    c.u8()?;
    c.u16()?;
    c.u32()?;
    c.n8(3)?;
    c.align(4);
    c.u32()?;
    c.n32(3)?;
    c.n8(4)?;
    c.n32(3)?;
    c.n8(4)?;
    c.u16()?;
    c.u16()?;
    c.walign(8);
    c.ptr()?;
    c.ptr()?;
    c.u32()?;
    c.align(16);
    let (_, n_entries) = c.array()?;
    if n_entries != 0 {
        bail!("rigid bodies with collision entries aren't supported");
    }
    c.n32(2)?;
    c.u16()?;
    c.u16()?;
    c.walign(16);
    let name = c.ptr()?;
    let (_, n_props) = c.array()?;
    if n_props != 0 {
        bail!("rigid bodies with properties aren't supported");
    }
    c.u8()?;
    c.align(2);
    c.u16()?;
    c.n32(2)?;
    c.walign(8);
    c.ptr()?;
    c.u32()?;
    c.walign(8);
    c.ptr()?;
    c.u32()?;
    c.u16()?;
    c.u16()?;
    c.ptr()?;
    c.u16()?;
    c.u16()?;
    c.walign(8);
    for _ in 0..2 {
        let (_, n) = c.array()?;
        if n != 0 {
            bail!("rigid bodies with constraints aren't supported");
        }
    }
    c.ptr()?;
    c.n8(3)?;
    c.align(2);
    c.u32()?;
    c.ptr()?;
    c.u16()?;
    c.n8(2)?;
    c.wpad4();
    c.align(16);
    referenced(c)?;
    c.n8(2)?;
    c.u16()?;
    c.u16()?;
    c.align(16);
    c.vec4(4)?;
    c.vec4(5)?;
    c.vec4(1)?;
    c.u32()?;
    c.u16()?;
    c.u16()?;
    c.u16()?;
    c.n8(3)?;
    c.align(16);
    c.vec4(3)?;
    c.vec4(2)?;
    c.u32()?;
    c.align(8);
    c.ptr()?;
    c.u16()?;
    c.u16()?;
    c.walign(16);
    for _ in 0..2 {
        c.ptr()?;
        c.u16()?;
        c.u16()?;
        c.walign(8);
    }
    c.ptr()?;
    c.ptr()?;
    c.u32()?;
    c.align(16);
    c.at(name, true, |c| c.cstr())?;
    c.walign(16);
    Ok(())
}

fn object(c: &mut Ctx, class: &str) -> Result<()> {
    match class {
        "hkRootLevelContainer" => {
            let (variants, n) = c.array()?;
            c.walign(16);
            let mut names = Vec::new();
            c.at(variants, true, |c| {
                for _ in 0..n {
                    let name = c.ptr()?;
                    let class = c.ptr()?;
                    c.ptr()?;
                    names.push((name, class));
                }
                Ok(())
            })?;
            for (name, class) in names {
                c.at(name, true, |c| c.cstr())?;
                c.walign(16);
                c.at(class, true, |c| c.cstr())?;
                c.walign(16);
            }
            Ok(())
        }
        "hkpPhysicsData" => {
            referenced(c)?;
            c.walign(16);
            c.ptr()?;
            let (systems, n) = c.array()?;
            c.align(16);
            c.at(systems, true, |c| {
                for _ in 0..n {
                    c.ptr()?;
                }
                Ok(())
            })?;
            c.walign(16);
            Ok(())
        }
        "hkpPhysicsSystem" => {
            referenced(c)?;
            c.walign(16);
            let (bodies, n) = c.array()?;
            for _ in 0..3 {
                let (_, k) = c.array()?;
                if k != 0 {
                    bail!("physics systems with constraints, actions or phantoms aren't supported");
                }
            }
            let name = c.ptr()?;
            c.word()?;
            c.u8()?;
            c.align(16);
            if n > 0 {
                c.at(bodies, false, |c| {
                    for _ in 0..n {
                        c.ptr()?;
                    }
                    Ok(())
                })?;
                c.walign(16);
            }
            c.at(name, true, |c| c.cstr())?;
            c.walign(16);
            Ok(())
        }
        "hkpRigidBody" => rigid_body(c),
        "StaticCompoundInfo" => {
            c.take(4)?;
            c.eof_at = Some(c.w.len());
            c.w.extend([0; 8]);
            let (actors, n_actors) = c.array()?;
            let (shapes, n_shapes) = c.array()?;
            c.align(16);
            c.at(actors, true, |c| c.n32(4 * n_actors))?;
            c.walign(16);
            c.at(shapes, true, |c| {
                for _ in 0..n_shapes {
                    c.n32(2)?;
                    c.n8(4)?;
                }
                Ok(())
            })?;
            c.walign(16);
            Ok(())
        }
        "hkaiNavMesh" => nav_mesh(c),
        "hkaiDirectedGraphExplicitCost" => nav_graph(c),
        "hkaiStaticTreeNavMeshQueryMediator" => {
            referenced(c)?;
            c.walign(16);
            c.ptr()?;
            c.ptr()?;
            Ok(())
        }
        "hkcdStaticAabbTree" => {
            referenced(c)?;
            c.u8()?;
            c.align(4);
            c.ptr()?;
            c.align(16);
            Ok(())
        }
        "hkcdStaticTreeDefaultTreeStorage6" => {
            let (nodes, n) = c.array()?;
            c.align(16);
            aabb(c)?;
            if n > 0 {
                c.at(nodes, false, |c| {
                    for _ in 0..n {
                        c.n8(4)?;
                        c.u16()?;
                    }
                    Ok(())
                })?;
                c.walign(16);
            }
            Ok(())
        }
        k if k.starts_with("hkp") => shape_object(c, k),
        k => bail!("Havok class {k} isn't supported"),
    }
}

fn recipes() -> &'static HashMap<String, Vec<String>> {
    static R: std::sync::OnceLock<HashMap<String, Vec<String>>> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let text = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/havok/classes.txt"));
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        let mut cur: Option<String> = None;
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("class ") {
                cur = Some(name.to_string());
                out.insert(name.to_string(), vec![]);
            } else if let Some(c) = &cur
                && let Some(lines) = out.get_mut(c) {
                    lines.push(line.trim().to_string());
                }
        }
        out
    })
}

struct Pending {
    p: (usize, usize),
    kind: String,
    n: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Both,
    Switch,
    WiiU,
}

fn prim_size(op: &str) -> Option<(usize, usize)> {
    Some(match op {
        "U8" | "I8" | "BOOL" => (1, 1),
        "U16" | "I16" => (1, 2),
        "U32" | "I32" | "F32" => (1, 4),
        "U64" | "I64" => (1, 8),
        "V4" => (4, 4),
        "M3" | "QS" => (12, 4),
        "M4" => (16, 4),
        _ => return None,
    })
}

fn prim(c: &mut Ctx, op: &str, side: Side) -> Result<bool> {
    let Some((n, size)) = prim_size(op) else { return Ok(false) };
    for _ in 0..n {
        match side {
            Side::Both => match size {
                1 => {
                    c.u8()?;
                }
                2 => {
                    c.u16()?;
                }
                4 => {
                    c.u32()?;
                }
                _ => {
                    c.u64()?;
                }
            },
            Side::Switch => c.w.extend(std::iter::repeat_n(0, size)),
            Side::WiiU => {
                c.take(size)?;
            }
        }
    }
    Ok(true)
}

fn run_class(c: &mut Ctx, class: &str, pending: &mut Vec<Pending>, depth: usize) -> Result<()> {
    if depth > 32 {
        bail!("Havok classes nested too deep");
    }
    let ops = recipes().get(class).with_context(|| format!("Havok class {class} isn't known"))?;
    let mut sides = vec![Side::Both];
    for op in ops {
        let side = *sides.last().unwrap_or(&Side::Both);
        let mut parts = op.split(' ');
        let head = parts.next().unwrap_or_default();
        let arg = parts.next().unwrap_or_default();
        let arg2 = parts.next().unwrap_or_default();
        if side == Side::Both && matches!(head, "STRING" | "EMPTYPTR" | "POINTER" | "ARRAY" | "USIZE") {
            c.ralign(4);
            c.walign(8);
        }
        match head {
            "IF8" => sides.push(if side == Side::WiiU { Side::WiiU } else { Side::Switch }),
            "IF4" => sides.push(if side == Side::Switch { Side::Switch } else { Side::WiiU }),
            "END" => {
                sides.pop();
            }
            "PAD" => {
                let n: usize = arg.parse()?;
                match side {
                    Side::Both => c.align(n),
                    Side::Switch => c.walign(n),
                    Side::WiiU => c.ralign(n),
                }
            }
            "SEEK" => {
                let n: i64 = arg.parse()?;
                match side {
                    Side::Switch => {
                        let len = (c.w.len() as i64 + n) as usize;
                        c.w.truncate(len);
                    }
                    Side::WiiU => c.rp = (c.rp as i64 + n) as usize,
                    Side::Both => bail!("{class}: a seek on both sides"),
                }
            }
            "PARENT" | "STRUCT" => run_class(c, arg, pending, depth + 1)?,
            "USIZE" => match side {
                Side::Both => c.word()?,
                Side::Switch => c.w.extend([0; 8]),
                Side::WiiU => {
                    c.take(4)?;
                }
            },
            "BYTES" => {
                let n: usize = arg.parse()?;
                let b = c.take(n)?.to_vec();
                c.w.extend(b);
            }
            "STRING" => {
                let p = c.ptr()?;
                pending.push(Pending { p, kind: "CSTR".into(), n: 1 });
            }
            "EMPTYPTR" | "POINTER" => {
                c.ptr()?;
            }
            "ARRAY" => {
                let (p, n) = c.array()?;
                let kind = if arg2.is_empty() { arg.to_string() } else { format!("{arg} {arg2}") };
                pending.push(Pending { p, kind, n });
            }
            h => {
                if !prim(c, h, side)? {
                    bail!("{class}: Havok op {op} isn't supported");
                }
            }
        }
    }
    Ok(())
}

fn generic_object(c: &mut Ctx, class: &str) -> Result<()> {
    let mut pending = Vec::new();
    run_class(c, class, &mut pending, 0)?;
    if matches!(class, "hclObjectSpaceSkinPOperator" | "hclBoneSpaceSkinPOperator") {
        let n = pending.len();
        if n >= 2 && pending[n - 2].n == 0 && pending[n - 1].n > 0 {
            let (Some(unpacked), Some(packed)) = (pending.pop(), pending.pop()) else {
                bail!("{class}: its packed arrays are missing");
            };
            let count = unpacked.n as u32;
            let (ps, us) = (packed.p.1, unpacked.p.1);
            c.w[ps + 8..ps + 12].copy_from_slice(&count.to_le_bytes());
            c.w[ps + 12..ps + 16].copy_from_slice(&(0x8000_0000 | count).to_le_bytes());
            c.w[us + 8..us + 12].copy_from_slice(&0u32.to_le_bytes());
            c.w[us + 12..us + 16].copy_from_slice(&0x8000_0000u32.to_le_bytes());
            let kind = if class == "hclObjectSpaceSkinPOperator" { "PACKP" } else { "STRUCT hclBoneSpaceDeformerLocalBlockP" };
            pending.push(Pending { p: (unpacked.p.0, ps), kind: kind.into(), n: unpacked.n });
        }
    }
    if class == "hclClothData" {
        let at = c.w.len() - 8;
        c.w[at..at + 4].copy_from_slice(&[0; 4]);
    }
    c.walign(16);
    loop {
        let next = pending.iter().enumerate().filter_map(|(i, p)| c.local.get(&p.p.0).map(|&d| (d, i))).min();
        let Some((dst, i)) = next else { break };
        let p = pending.swap_remove(i);
        c.walign(16);
        c.slocal.push((p.p.1, c.w.len()));
        c.rp = dst;
        let mut more = Vec::new();
        let mut parts = p.kind.split(' ');
        let k0 = parts.next().unwrap_or_default();
        let k1 = parts.next().unwrap_or_default();
        for _ in 0..p.n {
            match k0 {
                "CSTR" => c.cstr()?,
                "POINTER" => {
                    c.ptr()?;
                }
                "STR" => {
                    let q = c.ptr()?;
                    more.push(Pending { p: q, kind: "CSTR".into(), n: 1 });
                }
                "STRUCT" => run_class(c, k1, &mut more, 0)?,
                "EMPTY" => {}
                "PACKP" => {
                    for _ in 0..16 {
                        let mut v = [0f32; 4];
                        for x in &mut v {
                            *x = f32::from_bits(u32::from_be_bytes(c.take(4)?.try_into()?));
                        }
                        let m = v[..3].iter().fold(0f32, |a, x| a.max(x.abs()));
                        let (vals, w) = if m == 0.0 {
                            ([0i16; 3], 0x1980u16)
                        } else {
                            let e = m.log2().floor() as i32;
                            let e = if 2f32.powi(e) > m { e - 1 } else if 2f32.powi(e + 1) <= m { e + 1 } else { e };
                            let scale = 2f64.powi(14 - e);
                            let vals = [0, 1, 2].map(|k| (v[k] as f64 * scale + 0.5).floor() as i16);
                            (vals, ((2f32.powi(e - 30)).to_bits() >> 16) as u16)
                        };
                        for x in vals {
                            c.w.extend(x.to_le_bytes());
                        }
                        c.w.extend(w.to_le_bytes());
                    }
                }
                k => {
                    if !prim(c, k, Side::Both)? {
                        bail!("{class}: an array of {k} isn't supported");
                    }
                }
            }
        }
        if p.kind == "CSTR" {
            c.walign(2);
        }
        c.walign(16);
        pending.extend(more);
    }
    Ok(())
}

fn put32(w: &mut [u8], o: usize, v: usize) {
    w[o..o + 4].copy_from_slice(&(v as u32).to_le_bytes());
}

fn ff16(w: &mut Vec<u8>) {
    while !w.len().is_multiple_of(16) {
        w.push(0xff);
    }
}

fn convert_packfile(p: &Packfile) -> Result<(Vec<u8>, Option<usize>)> {
    let mut out = Vec::new();
    let h = p.header;
    out.extend_from_slice(&h[..8]);
    for o in [8, 0xc] {
        out.extend((be32(h, o)? as u32).to_le_bytes());
    }
    out.extend([8, 1, 1, 1]);
    for o in [0x14, 0x18, 0x1c, 0x20, 0x24] {
        out.extend((be32(h, o)? as u32).to_le_bytes());
    }
    out.extend_from_slice(&h[0x28..0x38]);
    out.extend((be32(h, 0x38)? as u32).to_le_bytes());
    out.extend(u16::from_be_bytes([h[0x3c], h[0x3d]]).to_le_bytes());
    out.extend(u16::from_be_bytes([h[0x3e], h[0x3f]]).to_le_bytes());
    if h.len() > 0x40 {
        out.extend(u16::from_be_bytes([h[0x40], h[0x41]]).to_le_bytes());
        out.extend(u16::from_be_bytes([h[0x42], h[0x43]]).to_le_bytes());
        for o in [0x44, 0x48, 0x4c] {
            out.extend((be32(h, o)? as u32).to_le_bytes());
        }
    }
    let sections_at = out.len();
    for tag in ["__classnames__", "__types__", "__data__"] {
        let mut t = [0u8; 20];
        t[..tag.len()].copy_from_slice(tag.as_bytes());
        t[19] = 0xff;
        out.extend(t);
        out.extend([0; 28]);
        out.extend([0xff; 16]);
    }
    let fill = |out: &mut Vec<u8>, i: usize, v: [usize; 7]| {
        for (k, x) in v.iter().enumerate() {
            put32(out, sections_at + 0x40 * i + 20 + 4 * k, *x);
        }
    };
    let names_at = out.len();
    let mut class_offset = HashMap::new();
    for (sig, name) in &p.classes {
        out.extend(sig.to_le_bytes());
        out.push(9);
        class_offset.insert(name.clone(), out.len() - names_at);
        out.extend(name.as_bytes());
        out.push(0);
    }
    ff16(&mut out);
    let n = out.len() - names_at;
    fill(&mut out, 0, [names_at, n, n, n, n, n, n]);
    let types_at = out.len();
    fill(&mut out, 1, [types_at, 0, 0, 0, 0, 0, 0]);
    let data_at = out.len();
    let mut placed = Vec::with_capacity(p.objects.len());
    let mut pointers = Vec::with_capacity(p.objects.len());
    let mut locals = Vec::new();
    let mut eof_at = None;
    for o in &p.objects {
        let mut c = Ctx {
            r: o.bytes,
            rp: 0,
            local: &o.local,
            w: Vec::new(),
            slocal: Vec::new(),
            pointers: HashMap::new(),
            eof_at: None,
            positions: HashMap::new(),
        };
        if let Err(e) = object(&mut c, &o.class) {
            c = Ctx {
                r: o.bytes,
                rp: 0,
                local: &o.local,
                w: Vec::new(),
                slocal: Vec::new(),
                pointers: HashMap::new(),
                eof_at: None,
                positions: HashMap::new(),
            };
            generic_object(&mut c, &o.class).with_context(|| format!("{}: {e:#}", o.class))?;
        }
        let (end_r, end_w) = (c.rp.max(o.size), c.w.len());
        c.positions.entry(end_r).or_insert(end_w);
        let at = out.len() - data_at;
        placed.push(at);
        locals.extend(c.slocal.iter().map(|(s, d)| (at + s, at + d)));
        if let Some(e) = c.eof_at {
            eof_at = Some(data_at + at + e);
        }
        pointers.push((c.pointers, c.positions));
        out.extend(c.w);
    }
    let mut globals = Vec::new();
    for &(s, so, t, to) in &p.globals {
        let src = pointers[s].0.get(&so).with_context(|| format!("{}: a pointer SWMM didn't read", p.objects[s].class))?;
        let dst = if to == 0 {
            0
        } else {
            *pointers[t].1.get(&to).with_context(|| format!("{}: a pointer into it at {to:#x}", p.objects[t].class))?
        };
        globals.push((placed[s] + src, placed[t] + dst));
    }
    locals.sort_by_key(|&(s, d)| (d, s));
    let local_at = out.len() - data_at;
    for (s, d) in &locals {
        out.extend((*s as u32).to_le_bytes());
        out.extend((*d as u32).to_le_bytes());
    }
    ff16(&mut out);
    let global_at = out.len() - data_at;
    for (s, d) in &globals {
        out.extend((*s as u32).to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend((*d as u32).to_le_bytes());
    }
    ff16(&mut out);
    let virtual_at = out.len() - data_at;
    for (k, o) in p.objects.iter().enumerate() {
        out.extend((placed[k] as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend((class_offset[&o.class] as u32).to_le_bytes());
    }
    ff16(&mut out);
    let end = out.len() - data_at;
    fill(&mut out, 2, [data_at, local_at, global_at, virtual_at, end, end, end]);
    Ok((out, eof_at))
}

pub fn to_switch(raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = raw;
    let mut fills = Vec::new();
    while rest.starts_with(&[0x57, 0xe0, 0xe0, 0x57]) {
        let p = read_packfile(rest)?;
        let (bytes, eof) = convert_packfile(&p)?;
        let start = out.len();
        out.extend(bytes);
        if let Some(e) = eof {
            fills.push(start + e);
        }
        rest = rest.get(p.len..).unwrap_or_default();
        let end = out.len();
        for f in fills.drain(..) {
            put32(&mut out, f, end);
        }
    }
    Ok(out)
}

fn streaming_sets(c: &mut Ctx, sets: (usize, usize), n: usize) -> Result<()> {
    let mut parts = Vec::new();
    c.at(sets, false, |c| {
        for _ in 0..n {
            c.n32(2)?;
            parts.push([c.array()?, c.array()?, c.array()?]);
        }
        Ok(())
    })?;
    c.walign(16);
    for [mesh, graph, volume] in parts {
        if mesh.1 > 0 {
            c.at(mesh.0, false, |c| c.n32(4 * mesh.1))?;
            c.walign(16);
        }
        if graph.1 > 0 {
            c.at(graph.0, false, |c| {
                for _ in 0..graph.1 {
                    c.n32(3)?;
                    c.u16()?;
                    c.align(16);
                }
                Ok(())
            })?;
            c.walign(16);
        }
        if volume.1 > 0 {
            c.at(volume.0, false, |c| c.n32(2 * volume.1))?;
            c.walign(16);
        }
    }
    Ok(())
}

fn nav_mesh(c: &mut Ctx) -> Result<()> {
    referenced(c)?;
    c.walign(16);
    let (faces, n_faces) = c.array()?;
    let (edges, n_edges) = c.array()?;
    let (verts, n_verts) = c.array()?;
    let (sets, n_sets) = c.array()?;
    let (face_data, n_face_data) = c.array()?;
    let (edge_data, n_edge_data) = c.array()?;
    c.n32(2)?;
    c.u8()?;
    c.align(16);
    aabb(c)?;
    c.u64()?;
    c.u64()?;
    if n_faces > 0 {
        c.at(faces, false, |c| {
            for _ in 0..n_faces {
                c.n32(2)?;
                for _ in 0..4 {
                    c.u16()?;
                }
            }
            Ok(())
        })?;
        c.walign(16);
    }
    if n_edges > 0 {
        c.at(edges, false, |c| {
            for _ in 0..n_edges {
                c.n32(4)?;
                c.n8(2)?;
                c.u16()?;
            }
            Ok(())
        })?;
        c.walign(16);
    }
    if n_verts > 0 {
        c.at(verts, false, |c| c.vec4(n_verts))?;
        c.walign(16);
    }
    if n_sets > 0 {
        streaming_sets(c, sets, n_sets)?;
    }
    if n_face_data > 0 {
        c.at(face_data, false, |c| c.n32(n_face_data))?;
        c.walign(16);
    }
    if n_edge_data > 0 {
        c.at(edge_data, false, |c| c.n32(n_edge_data))?;
        c.walign(16);
    }
    Ok(())
}

fn nav_graph(c: &mut Ctx) -> Result<()> {
    referenced(c)?;
    c.walign(16);
    let (positions, n_pos) = c.array()?;
    let (nodes, n_nodes) = c.array()?;
    let (edges, n_edges) = c.array()?;
    let (node_data, n_node_data) = c.array()?;
    let (edge_data, n_edge_data) = c.array()?;
    c.n32(2)?;
    let (sets, n_sets) = c.array()?;
    c.align(16);
    if n_pos > 0 {
        c.at(positions, false, |c| c.vec4(n_pos))?;
        c.walign(16);
    }
    if n_nodes > 0 {
        c.at(nodes, false, |c| c.n32(2 * n_nodes))?;
        c.walign(16);
    }
    if n_edges > 0 {
        c.at(edges, false, |c| {
            for _ in 0..n_edges {
                c.u16()?;
                c.u16()?;
                c.u32()?;
            }
            Ok(())
        })?;
        c.walign(16);
    }
    if n_node_data > 0 {
        c.at(node_data, false, |c| c.n32(n_node_data))?;
        c.walign(16);
    }
    if n_edge_data > 0 {
        c.at(edge_data, false, |c| c.n32(n_edge_data))?;
        c.walign(16);
    }
    if n_sets > 0 {
        streaming_sets(c, sets, n_sets)?;
    }
    Ok(())
}
