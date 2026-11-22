use std::collections::HashMap;

use anyhow_ext::{Context, Result, bail};

const NONE: u32 = 0xffff_ffff;

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn u32(&self, o: usize) -> Result<u32> {
        Ok(u32::from_be_bytes(self.0.get(o..o + 4).context("the link table is cut short")?.try_into()?))
    }
    fn u16(&self, o: usize) -> Result<u16> {
        Ok(u16::from_be_bytes(self.0.get(o..o + 2).context("the link table is cut short")?.try_into()?))
    }
    fn u8(&self, o: usize) -> Result<u8> {
        self.0.get(o).copied().context("the link table is cut short")
    }
    fn u64(&self, o: usize) -> Result<u64> {
        Ok(u64::from_be_bytes(self.0.get(o..o + 8).context("the link table is cut short")?.try_into()?))
    }
}

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes());
    }
    fn pad(&mut self, n: usize) {
        self.0.resize(self.0.len() + n, 0);
    }
    fn align(&mut self, base: usize, a: usize) {
        while !(self.0.len() - base).is_multiple_of(a) {
            self.0.push(0);
        }
    }
    fn put64(&mut self, at: usize, v: u64) {
        self.0[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub fn to_switch(raw: &[u8]) -> Result<Vec<u8>> {
    let w = In(raw);
    if raw.get(..4) != Some(b"XLNK") {
        bail!("not a link table");
    }
    let h = (0..17).map(|i| w.u32(4 + 4 * i)).collect::<Result<Vec<_>>>()?;
    let (version, n_asset, n_trig_ow, trig_ow_pos, lp_pos) = (h[1], h[3] as usize, h[4] as usize, h[5], h[6]);
    let (n_lp, n_lpe, n_direct, n_random, n_curve, n_point) =
        (h[7] as usize, h[8] as usize, h[9] as usize, h[10] as usize, h[11] as usize, h[12] as usize);
    let (ex_pos, n_user, cond_pos, name_pos) = (h[13] as usize, h[14] as usize, h[15] as usize, h[16] as usize);
    if !matches!(version, 0x1c | 0x1e) {
        bail!("link table version {version:#x} isn't known");
    }
    let mut o = Out::default();
    o.0.extend_from_slice(b"XLNK");
    for v in [0, version, h[2], h[3], h[4]] {
        o.u32(v);
    }
    let hdr_ptr = o.len();
    o.pad(16);
    for v in &h[7..13] {
        o.u32(*v);
    }
    let hdr_ex = o.len();
    o.pad(8);
    o.u32(n_user as u32);
    o.pad(4);
    let hdr_cond = o.len();
    o.pad(16);
    let mut p = 0x48;
    for i in 0..n_user {
        o.u32(w.u32(p + 4 * i)?);
    }
    p += 4 * n_user;
    let users = (0..n_user).map(|i| w.u32(p + 4 * i).map(|v| v as usize)).collect::<Result<Vec<_>>>()?;
    p += 4 * n_user;
    o.align(0, 8);
    let user_ptrs = o.len();
    o.pad(8 * n_user);

    let (pdt_size, pu, pa, punk, pt) =
        (w.u32(p)? as usize, w.u32(p + 4)?, w.u32(p + 8)?, w.u32(p + 0xc)?, w.u32(p + 0x10)?);
    let pdt_at = o.len();
    for v in [0, pu, pa, punk, pt] {
        o.u32(v);
    }
    o.pad(4);
    let defs = (pu + pa + pt) as usize;
    for i in 0..defs {
        let e = p + 0x14 + 12 * i;
        let (name, kind, value) = (w.u32(e)?, w.u32(e + 4)?, w.u32(e + 8)?);
        o.u64(name as u64);
        o.u32(kind);
        o.pad(4);
        if kind == 1 {
            o.0.extend((f32::from_bits(value) as f64).to_le_bytes());
        } else if kind == 0 {
            o.u64(value as i32 as i64 as u64);
        } else {
            o.u64(value as u64);
        }
    }
    o.0.extend_from_slice(raw.get(p + 0x14 + 12 * defs..p + pdt_size).context("the parameter names are cut short")?);
    let n = (o.len() - pdt_at) as u32;
    o.0[pdt_at..pdt_at + 4].copy_from_slice(&n.to_le_bytes());
    p += pdt_size;

    let mut kind4 = vec![];
    for _ in 0..n_asset {
        let mask = w.u64(p)?;
        o.u64(mask);
        p += 8;
        for _ in 0..mask.count_ones() {
            let v = w.u32(p)?;
            if v >> 24 == 4 {
                kind4.push(o.len());
            }
            o.u32(v);
            p += 4;
        }
    }
    if p != trig_ow_pos as usize {
        bail!("the asset parameters don't end where the header says");
    }
    let new_trig_ow = o.len();
    for _ in 0..n_trig_ow {
        let mask = w.u32(p)?;
        o.u32(mask);
        p += 4;
        for _ in 0..mask.count_ones() {
            let v = w.u32(p)?;
            if v >> 24 == 4 {
                kind4.push(o.len());
            }
            o.u32(v);
            p += 4;
        }
    }
    if p != lp_pos as usize {
        bail!("the trigger overwrite parameters don't end where the header says");
    }
    let new_lp = o.len();
    for _ in 0..n_lp + n_lpe {
        o.u64(w.u32(p)? as u64);
        p += 4;
    }
    for _ in 0..n_direct + 2 * n_random {
        o.u32(w.u32(p)?);
        p += 4;
    }
    for _ in 0..n_curve {
        for k in 0..4 {
            o.u16(w.u16(p + 2 * k)?);
        }
        o.u64(w.u32(p + 8)? as u64);
        o.u32(w.u32(p + 12)?);
        o.u16(w.u16(p + 16)?);
        o.u16(w.u16(p + 18)?);
        p += 20;
    }
    for _ in 0..2 * n_point {
        o.u32(w.u32(p)?);
        p += 4;
    }
    if p != ex_pos {
        bail!("the curves don't end where the header says");
    }
    let ex_at = o.len();
    let first_user = users.iter().copied().min().unwrap_or(cond_pos);
    let mut exmap: HashMap<u32, u32> = HashMap::new();
    while p < first_user {
        exmap.insert((p - ex_pos) as u32, (o.len() - ex_at) as u32);
        let n = w.u32(p)? as usize;
        o.u32(n as u32);
        p += 4;
        for _ in 0..n {
            o.u64(w.u32(p)? as u64);
            o.0.extend_from_slice(raw.get(p + 4..p + 8).context("the link table is cut short")?);
            o.pad(4);
            p += 8;
        }
    }
    if p != first_user {
        bail!("the arrange group parameters don't end where the users start");
    }
    let arrange = |v: u32| -> Result<u32> {
        if v >> 24 != 4 {
            return Ok(v);
        }
        Ok(4 << 24 | *exmap.get(&(v & 0xff_ffff)).context("a parameter to no arrange group parameters")?)
    };
    for at in kind4 {
        let v = arrange(u32::from_le_bytes(o.0[at..at + 4].try_into()?))?;
        o.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    let mut cond = Out::default();
    let mut cmap: HashMap<u32, u64> = HashMap::new();
    let mut c = cond_pos;
    while c < name_pos {
        cmap.insert((c - cond_pos) as u32, cond.len() as u64);
        match w.u32(c)? {
            0 => {
                let (kind, compare, value) = (w.u32(c + 4)?, w.u32(c + 8)?, w.u32(c + 12)?);
                let enum_index = w.u16(c + 16)? as i16;
                cond.u32(0);
                cond.u8(kind as u8);
                cond.u8(compare as u8);
                cond.u8(w.u8(c + 18)?);
                cond.u8(w.u8(c + 19)?);
                cond.u32(enum_index as i32 as u32);
                if kind == 0 {
                    cond.pad(4);
                    cond.u64(value as u64);
                } else {
                    cond.u32(value);
                }
                c += 20;
            }
            t @ (1 | 2) => {
                cond.u32(t);
                cond.u32(w.u32(c + 4)?);
                c += 8;
            }
            t => bail!("condition type {t} isn't known"),
        }
    }
    cmap.insert((c - cond_pos) as u32, cond.len() as u64);
    let condition = |v: u32| -> Result<u64> {
        if v == NONE { Ok(NONE as u64) } else { cmap.get(&v).copied().context("an offset to no condition") }
    };

    let mut new_users = Vec::with_capacity(n_user);
    for (ui, &u) in users.iter().enumerate() {
        let end = users.get(ui + 1).copied().unwrap_or(cond_pos);
        let us = o.len();
        new_users.push(us);
        let cnt = (0..11).map(|k| w.u32(u + 4 * k)).collect::<Result<Vec<_>>>()?;
        let [_, n_local, n_call, _, _, n_slot, n_action, n_atrig, n_prop, n_ptrig, n_always] =
            cnt.iter().map(|&v| v as usize).collect::<Vec<_>>()[..]
        else {
            unreachable!()
        };
        let trig = w.u32(u + 0x2c)? as usize;
        for v in &cnt {
            o.u32(*v);
        }
        o.pad(4);
        let trig_at = o.len();
        o.pad(8);
        let mut q = u + 0x30;
        for _ in 0..n_local {
            o.u64(w.u32(q)? as u64);
            q += 4;
        }
        for _ in 0..pu {
            o.u32(arrange(w.u32(q)?)?);
            q += 4;
        }
        for _ in 0..n_call {
            o.u16(w.u16(q)?);
            q += 2;
        }
        q = u + (q - u).div_ceil(4) * 4;
        o.align(us, 4);
        let calls = q;
        let cont = calls + 32 * n_call;
        let mut cpos: HashMap<u32, u64> = HashMap::new();
        let mut blocks = Vec::new();
        let (mut st, mut pos) = (0usize, 0u64);
        while cont + st < u + trig {
            let t = w.u32(cont + st)?;
            cpos.insert(st as u32, pos);
            blocks.push((st, t));
            st += if t == 0 { 24 } else { 12 };
            pos += if t == 0 { 32 } else { 16 };
        }
        if cont + st != u + trig {
            bail!("user {ui}: the containers don't end where its triggers start");
        }
        for i in 0..n_call {
            let e = calls + 32 * i;
            let (flag, start, cp) = (w.u16(e + 6)?, w.u32(e + 0x18)?, w.u32(e + 0x1c)?);
            o.u64(w.u32(e)? as u64);
            o.u16(w.u16(e + 4)?);
            o.u16(flag);
            for k in [8, 0xc, 0x10, 0x14] {
                o.u32(w.u32(e + k)?);
            }
            o.pad(4);
            o.u64(if flag & 1 != 0 { *cpos.get(&start).context("a call to no container")? } else { start as u64 });
            o.u64(condition(cp)?);
        }
        for &(st, t) in &blocks {
            let e = cont + st;
            o.u32(t);
            o.u32(w.u32(e + 4)?);
            o.u32(w.u32(e + 8)?);
            o.pad(4);
            if t == 0 {
                o.u64(w.u32(e + 12)? as u64);
                o.u32(w.u32(e + 16)?);
                o.u16(w.u16(e + 20)?);
                o.u8(w.u8(e + 22)?);
                o.u8(w.u8(e + 23)?);
            }
        }
        let at = (o.len() - us) as u64;
        o.put64(trig_at, at);
        let ctb = |v: u32| if v == NONE { NONE as u64 } else { v as u64 / 32 * 48 };
        let mut q = u + trig;
        for _ in 0..n_slot {
            o.u64(w.u32(q)? as u64);
            o.u16(w.u16(q + 4)?);
            o.u16(w.u16(q + 6)?);
            o.pad(4);
            q += 8;
        }
        for _ in 0..n_action {
            o.u64(w.u32(q)? as u64);
            o.u32(w.u32(q + 4)?);
            o.u32(w.u32(q + 8)?);
            q += 12;
        }
        for _ in 0..n_atrig {
            o.u32(w.u32(q)?);
            o.pad(4);
            o.u64(ctb(w.u32(q + 4)?));
            o.u32(w.u32(q + 8)?);
            o.pad(4);
            o.u32(w.u32(q + 12)?);
            o.u16(w.u16(q + 16)?);
            o.u16(w.u16(q + 18)?);
            o.u64(w.u32(q + 20)? as u64);
            q += 24;
        }
        for _ in 0..n_prop {
            o.u64(w.u32(q)? as u64);
            for k in [4, 8, 12] {
                o.u32(w.u32(q + k)?);
            }
            o.pad(4);
            q += 16;
        }
        for _ in 0..n_ptrig {
            o.u32(w.u32(q)?);
            o.u16(w.u16(q + 12)?);
            o.u16(w.u16(q + 14)?);
            o.u64(ctb(w.u32(q + 4)?));
            o.u64(condition(w.u32(q + 8)?)?);
            o.u64(w.u32(q + 16)? as u64);
            q += 20;
        }
        for _ in 0..n_always {
            o.u32(w.u32(q)?);
            o.u16(w.u16(q + 8)?);
            o.u16(w.u16(q + 10)?);
            o.u64(ctb(w.u32(q + 4)?));
            o.u64(w.u32(q + 12)? as u64);
            q += 16;
        }
        if q != end {
            bail!("user {ui} doesn't end where the next starts");
        }
    }
    let new_cond = o.len();
    o.0.extend(cond.0);
    let new_name = o.len();
    o.0.extend_from_slice(raw.get(name_pos..).context("the names are cut short")?);
    let size = o.len() as u32;
    o.0[4..8].copy_from_slice(&size.to_le_bytes());
    o.put64(hdr_ptr, new_trig_ow as u64);
    o.put64(hdr_ptr + 8, new_lp as u64);
    o.put64(hdr_ex, ex_at as u64);
    o.put64(hdr_cond, new_cond as u64);
    o.put64(hdr_cond + 8, new_name as u64);
    for (i, &x) in new_users.iter().enumerate() {
        o.put64(user_ptrs + 8 * i, x as u64);
    }
    Ok(o.0)
}

struct Sw<'a> {
    b: &'a [u8],
    n_asset: u32,
    n_tow: u32,
    pdt: std::ops::Range<usize>,
    asset_at: usize,
    tow_at: usize,
    lp_at: usize,
    direct_at: usize,
    random_at: usize,
    curve_at: usize,
    point_at: usize,
    ex_at: usize,
    users_at: usize,
    cond_at: usize,
    name_at: usize,
    counts: [u32; 6],
    pu: usize,
    users: Vec<(u32, usize, usize)>,
}

fn le32(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(o..o + 4).context("the link table is cut short")?.try_into()?))
}
fn le16(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(b.get(o..o + 2).context("the link table is cut short")?.try_into()?))
}
fn le64(b: &[u8], o: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(b.get(o..o + 8).context("the link table is cut short")?.try_into()?))
}
fn bytes<const N: usize>(b: &[u8], o: usize) -> Result<[u8; N]> {
    Ok(b.get(o..o + N).context("the link table is cut short")?.try_into()?)
}

impl<'a> Sw<'a> {
    fn new(b: &'a [u8]) -> Result<Self> {
        if b.get(..4) != Some(b"XLNK") {
            bail!("not a link table");
        }
        let (n_asset, n_tow) = (le32(b, 0x10)?, le32(b, 0x14)?);
        let (tow_at, lp_at) = (le64(b, 0x18)? as usize, le64(b, 0x20)? as usize);
        let mut counts = [0u32; 6];
        for (i, c) in counts.iter_mut().enumerate() {
            *c = le32(b, 0x28 + 4 * i)?;
        }
        let [n_lp, n_lpe, n_direct, n_random, n_curve, n_point] = counts.map(|c| c as usize);
        let ex_at = le64(b, 0x40)? as usize;
        let n_user = le32(b, 0x48)? as usize;
        let (cond_at, name_at) = (le64(b, 0x50)? as usize, le64(b, 0x58)? as usize);
        let ptrs_at = (0x60 + 4 * n_user).div_ceil(8) * 8;
        let pdt_at = ptrs_at + 8 * n_user;
        let pdt_size = le32(b, pdt_at)? as usize;
        let pu = le32(b, pdt_at + 4)? as usize;
        let direct_at = lp_at + 8 * (n_lp + n_lpe);
        let random_at = direct_at + 4 * n_direct;
        let curve_at = random_at + 8 * n_random;
        let point_at = curve_at + 24 * n_curve;
        let asset_at = pdt_at + pdt_size;
        if point_at + 8 * n_point != ex_at
            || name_at > b.len()
            || cond_at > name_at
            || ex_at > cond_at
            || asset_at > tow_at
            || tow_at > lp_at
        {
            bail!("the link table's regions don't add up");
        }
        let starts = (0..n_user)
            .map(|i| Ok((le32(b, 0x60 + 4 * i)?, le64(b, ptrs_at + 8 * i)? as usize)))
            .collect::<Result<Vec<_>>>()?;
        let mut ends: Vec<usize> = starts.iter().map(|u| u.1).collect();
        ends.push(cond_at);
        ends.sort_unstable();
        let mut users = Vec::with_capacity(n_user);
        for (h, s) in starts {
            let e = ends[ends.partition_point(|&x| x <= s)..].first().copied().unwrap_or(cond_at);
            if s < ex_at || e > cond_at {
                bail!("a user outside the users' region");
            }
            users.push((h, s, e));
        }
        users.sort_unstable_by_key(|u| u.0);
        let users_at = users.iter().map(|u| u.1).min().unwrap_or(cond_at);
        Ok(Self {
            b,
            n_asset,
            n_tow,
            pdt: pdt_at..asset_at,
            asset_at,
            tow_at,
            lp_at,
            direct_at,
            random_at,
            curve_at,
            point_at,
            ex_at,
            users_at,
            cond_at,
            name_at,
            counts,
            pu,
            users,
        })
    }
    fn name(&self, off: u64) -> Result<&'a [u8]> {
        let s = self.b.get(self.name_at + off as usize..).with_context(|| format!("a name past the table ({off:#x})"))?;
        Ok(&s[..s.iter().position(|&c| c == 0).context("an unterminated name")?])
    }
    fn user(&self, hash: u32) -> Option<&'a [u8]> {
        let i = self.users.binary_search_by_key(&hash, |u| u.0).ok()?;
        Some(&self.b[self.users[i].1..self.users[i].2])
    }
}

#[derive(Default)]
struct Sink {
    assets: Vec<u8>,
    add_assets: u32,
    add_values: u32,
    tow: Vec<u8>,
    add_tow: u32,
    direct: Vec<u32>,
    random: Vec<[u8; 8]>,
    curves: Vec<[u8; 24]>,
    points: Vec<[u8; 8]>,
    cond: Vec<u8>,
    ex: Vec<u8>,
    ex_map: HashMap<usize, u32>,
    names: Vec<u8>,
    name_map: HashMap<Vec<u8>, u64>,
    direct_map: HashMap<u32, u32>,
    random_map: HashMap<[u8; 8], u32>,
    asset_map: HashMap<usize, u64>,
    tow_map: HashMap<usize, u64>,
    cond_map: HashMap<usize, u64>,
    curve_map: HashMap<usize, u32>,
}

impl Sink {
    fn of(t: &Sw) -> Result<Self> {
        let b = t.b;
        let mut s = Sink {
            assets: b[t.asset_at..t.tow_at].to_vec(),
            tow: b[t.tow_at..t.lp_at].to_vec(),
            cond: b[t.cond_at..t.name_at].to_vec(),
            ex: b[t.ex_at..t.users_at].to_vec(),
            names: b[t.name_at..].to_vec(),
            ..Default::default()
        };
        for i in 0..t.counts[2] as usize {
            let v = le32(b, t.direct_at + 4 * i)?;
            s.direct.push(v);
            s.direct_map.entry(v).or_insert(i as u32);
        }
        for i in 0..t.counts[3] as usize {
            let v = bytes::<8>(b, t.random_at + 8 * i)?;
            s.random.push(v);
            s.random_map.entry(v).or_insert(i as u32);
        }
        for i in 0..t.counts[4] as usize {
            s.curves.push(bytes(b, t.curve_at + 24 * i)?);
        }
        for i in 0..t.counts[5] as usize {
            s.points.push(bytes(b, t.point_at + 8 * i)?);
        }
        let mut at = 0;
        for n in s.names.split(|&c| c == 0) {
            s.name_map.entry(n.to_vec()).or_insert(at as u64);
            at += n.len() + 1;
        }
        Ok(s)
    }

    fn name(&mut self, t: &Sw, off: u64) -> Result<u64> {
        if off == NONE as u64 {
            return Ok(off);
        }
        let n = t.name(off)?;
        if let Some(&o) = self.name_map.get(n) {
            return Ok(o);
        }
        let o = self.names.len() as u64;
        self.names.extend_from_slice(n);
        self.names.push(0);
        self.name_map.insert(n.to_vec(), o);
        Ok(o)
    }

    fn param(&mut self, t: &Sw, raw: u32) -> Result<u32> {
        let (kind, v) = (raw >> 24, (raw & 0xff_ffff) as usize);
        let new = match kind {
            0 | 5 => {
                if v >= t.counts[2] as usize {
                    bail!("a value past the table");
                }
                let x = le32(t.b, t.direct_at + 4 * v)?;
                match self.direct_map.get(&x) {
                    Some(&i) => i,
                    None => {
                        self.direct.push(x);
                        let i = self.direct.len() as u32 - 1;
                        self.direct_map.insert(x, i);
                        i
                    }
                }
            }
            1 => self.name(t, v as u64)? as u32,
            2 => self.curve(t, v)?,
            4 => self.arrange(t, v)?,
            3 | 6..=17 => {
                if v >= t.counts[3] as usize {
                    bail!("a random range past the table");
                }
                let x = bytes::<8>(t.b, t.random_at + 8 * v)?;
                match self.random_map.get(&x) {
                    Some(&i) => i,
                    None => {
                        self.random.push(x);
                        let i = self.random.len() as u32 - 1;
                        self.random_map.insert(x, i);
                        i
                    }
                }
            }
            k => bail!("parameter value kind {k} isn't known"),
        };
        if new > 0xff_ffff {
            bail!("too many values");
        }
        Ok(kind << 24 | new)
    }

    fn arrange(&mut self, t: &Sw, pos: usize) -> Result<u32> {
        if let Some(&n) = self.ex_map.get(&pos) {
            return Ok(n);
        }
        let at = t.ex_at + pos;
        let n = le32(t.b, at)? as usize;
        if at + 4 + 16 * n > t.users_at {
            bail!("arrange group parameters past their region");
        }
        let new = self.ex.len() as u32;
        self.ex.extend((n as u32).to_le_bytes());
        for k in 0..n {
            let e = at + 4 + 16 * k;
            let name = self.name(t, le64(t.b, e)?)?;
            self.ex.extend(name.to_le_bytes());
            self.ex.extend(bytes::<8>(t.b, e + 8)?);
        }
        self.ex_map.insert(pos, new);
        Ok(new)
    }

    fn curve(&mut self, t: &Sw, i: usize) -> Result<u32> {
        if let Some(&n) = self.curve_map.get(&i) {
            return Ok(n);
        }
        if i >= t.counts[4] as usize {
            bail!("a curve past the table");
        }
        let mut c = bytes::<24>(t.b, t.curve_at + 24 * i)?;
        let (start, n) = (u16::from_le_bytes([c[0], c[1]]) as usize, u16::from_le_bytes([c[2], c[3]]) as usize);
        if start + n > t.counts[5] as usize {
            bail!("curve points past the table");
        }
        let new_start = u16::try_from(self.points.len()).context("too many curve points")?;
        for k in start..start + n {
            self.points.push(bytes(t.b, t.point_at + 8 * k)?);
        }
        c[..2].copy_from_slice(&new_start.to_le_bytes());
        let name = self.name(t, u64::from_le_bytes(c[8..16].try_into()?))?;
        c[8..16].copy_from_slice(&name.to_le_bytes());
        self.curves.push(c);
        let n = self.curves.len() as u32 - 1;
        self.curve_map.insert(i, n);
        Ok(n)
    }

    fn asset(&mut self, t: &Sw, pos: u64) -> Result<u64> {
        if pos == NONE as u64 {
            return Ok(pos);
        }
        let pos = pos as usize;
        if let Some(&n) = self.asset_map.get(&pos) {
            return Ok(n);
        }
        let at = t.asset_at + pos;
        if at >= t.tow_at {
            bail!("asset parameters past the table");
        }
        let mask = le64(t.b, at)?;
        let new = self.assets.len() as u64;
        let mut e = mask.to_le_bytes().to_vec();
        for k in 0..mask.count_ones() as usize {
            e.extend(self.param(t, le32(t.b, at + 8 + 4 * k)?)?.to_le_bytes());
        }
        self.assets.extend(e);
        self.add_assets += 1;
        self.add_values += mask.count_ones();
        self.asset_map.insert(pos, new);
        Ok(new)
    }

    fn tow(&mut self, t: &Sw, pos: u64) -> Result<u64> {
        if pos == NONE as u64 {
            return Ok(pos);
        }
        let pos = pos as usize;
        if let Some(&n) = self.tow_map.get(&pos) {
            return Ok(n);
        }
        let at = t.tow_at + pos;
        if at >= t.lp_at {
            bail!("trigger parameters past the table");
        }
        let mask = le32(t.b, at)?;
        let new = self.tow.len() as u64;
        let mut e = mask.to_le_bytes().to_vec();
        for k in 0..mask.count_ones() as usize {
            e.extend(self.param(t, le32(t.b, at + 4 + 4 * k)?)?.to_le_bytes());
        }
        self.tow.extend(e);
        self.add_tow += 1;
        self.tow_map.insert(pos, new);
        Ok(new)
    }

    fn cond(&mut self, t: &Sw, off: u64) -> Result<u64> {
        if off == NONE as u64 {
            return Ok(off);
        }
        let off = off as usize;
        if let Some(&n) = self.cond_map.get(&off) {
            return Ok(n);
        }
        let at = t.cond_at + off;
        if at >= t.name_at {
            bail!("a condition past the table");
        }
        let new = self.cond.len() as u64;
        match le32(t.b, at)? {
            0 if t.b.get(at + 4) == Some(&0) => {
                let mut e = bytes::<24>(t.b, at)?;
                let n = self.name(t, le64(t.b, at + 16)?)?;
                e[16..24].copy_from_slice(&n.to_le_bytes());
                self.cond.extend(e);
            }
            0 => self.cond.extend(bytes::<16>(t.b, at)?),
            1 | 2 => self.cond.extend(bytes::<8>(t.b, at)?),
            k => bail!("condition type {k} isn't known"),
        }
        self.cond_map.insert(off, new);
        Ok(new)
    }

    fn user(&mut self, t: &Sw, u: &[u8]) -> Result<Vec<u8>> {
        let mut o = u.to_vec();
        let c = |k: usize| le32(u, 4 * k).map(|v| v as usize);
        let (n_local, n_call) = (c(1)?, c(2)?);
        let (n_slot, n_action, n_atrig, n_prop, n_ptrig, n_always) = (c(5)?, c(6)?, c(7)?, c(8)?, c(9)?, c(10)?);
        let trig = le64(u, 0x30)? as usize;
        let put = |o: &mut Vec<u8>, at: usize, v: u64| o[at..at + 8].copy_from_slice(&v.to_le_bytes());
        let mut q = 0x38;
        for _ in 0..n_local {
            let v = self.name(t, le64(u, q)?).context("local property")?;
            put(&mut o, q, v);
            q += 8;
        }
        for _ in 0..t.pu {
            let v = self.param(t, le32(u, q)?).context("user parameter")?;
            o[q..q + 4].copy_from_slice(&v.to_le_bytes());
            q += 4;
        }
        q = (q + 2 * n_call).div_ceil(4) * 4;
        for _ in 0..n_call {
            let v = self.name(t, le64(u, q)?).context("call")?;
            put(&mut o, q, v);
            if le16(u, q + 10)? & 1 == 0 {
                let v = self.asset(t, le64(u, q + 32)?).context("asset")?;
                put(&mut o, q + 32, v);
            }
            let v = self.cond(t, le64(u, q + 40)?)?;
            put(&mut o, q + 40, v);
            q += 48;
        }
        while q < trig {
            if le32(u, q)? == 0 {
                let v = self.name(t, le64(u, q + 16)?).context("container")?;
                put(&mut o, q + 16, v);
                q += 32;
            } else {
                q += 16;
            }
        }
        if q != trig {
            bail!("a user's containers don't end where its triggers start");
        }
        for _ in 0..n_slot + n_action {
            let v = self.name(t, le64(u, q)?).context("action")?;
            put(&mut o, q, v);
            q += 16;
        }
        for _ in 0..n_atrig {
            let v = self.tow(t, le64(u, q + 32)?)?;
            put(&mut o, q + 32, v);
            q += 40;
        }
        for _ in 0..n_prop {
            let v = self.name(t, le64(u, q)?).context("property")?;
            put(&mut o, q, v);
            q += 24;
        }
        for _ in 0..n_ptrig {
            let v = self.cond(t, le64(u, q + 16)?)?;
            put(&mut o, q + 16, v);
            let v = self.tow(t, le64(u, q + 24)?)?;
            put(&mut o, q + 24, v);
            q += 32;
        }
        for _ in 0..n_always {
            let v = self.tow(t, le64(u, q + 16)?)?;
            put(&mut o, q + 16, v);
            q += 24;
        }
        if q != u.len() {
            bail!("a user doesn't end where the next starts");
        }
        Ok(o)
    }
}

fn fingerprint(t: &Sw, u: &[u8]) -> Result<(u32, u32)> {
    let mut s = Sink::default();
    let mut v = s.user(t, u)?;
    v.extend(&s.assets);
    v.extend(&s.tow);
    v.extend(s.direct.iter().flat_map(|x| x.to_le_bytes()));
    v.extend(s.random.iter().flatten());
    v.extend(s.curves.iter().flatten());
    v.extend(s.points.iter().flatten());
    v.extend(&s.cond);
    v.extend(&s.ex);
    v.extend(&s.names);
    Ok((crate::stock::crc32(&v), v.len() as u32))
}

pub fn fingerprints(switch_layout: &[u8]) -> Result<Vec<(u32, u32, u32)>> {
    let t = Sw::new(switch_layout)?;
    t.users.iter().map(|&(h, s, e)| fingerprint(&t, &t.b[s..e]).map(|(c, n)| (h, c, n))).collect()
}

static WIIU_USERS: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/wiiu/xlink_users.bin"));

fn wiiu_users(sound: bool) -> &'static std::collections::HashSet<(u32, u32, u32)> {
    use std::sync::OnceLock;
    static T: OnceLock<[std::collections::HashSet<(u32, u32, u32)>; 2]> = OnceLock::new();
    let t = T.get_or_init(|| {
        let mut sets: [std::collections::HashSet<_>; 2] = Default::default();
        let mut p = 0;
        for set in sets.iter_mut() {
            let n = le32(WIIU_USERS, p).unwrap_or(0) as usize;
            p += 4;
            for _ in 0..n {
                let g = |k: usize| le32(WIIU_USERS, p + 4 * k).unwrap_or(0);
                set.insert((g(0), g(1), g(2)));
                p += 12;
            }
        }
        sets
    });
    &t[sound as usize]
}

pub fn wiiu_users_table(effects: &[u8], sounds: &[u8]) -> Result<Vec<u8>> {
    let mut o = vec![];
    for t in [effects, sounds] {
        let f = fingerprints(t)?;
        o.extend((f.len() as u32).to_le_bytes());
        for (h, c, n) in f {
            o.extend(h.to_le_bytes().into_iter().chain(c.to_le_bytes()).chain(n.to_le_bytes()));
        }
    }
    Ok(o)
}

pub fn merge(switch: &[u8], modded: &[u8], sound: bool) -> Result<(Vec<u8>, usize)> {
    let (s, m) = (Sw::new(switch)?, Sw::new(modded)?);
    let props = |t: &Sw| -> Result<Vec<Vec<u8>>> {
        (0..(t.counts[0] + t.counts[1]) as usize).map(|i| Ok(t.name(le64(t.b, t.lp_at + 8 * i)?)?.to_vec())).collect()
    };
    if s.b[8..12] != m.b[8..12] || s.counts[..2] != m.counts[..2] {
        bail!("the mod's table is another version from the game's");
    }
    if s.b[s.pdt.clone()] != m.b[m.pdt.clone()] {
        bail!("the mod's table defines its parameters differently from the game's");
    }
    if props(&s)? != props(&m)? {
        bail!("the mod's table names its local properties differently from the game's");
    }
    let wiiu = wiiu_users(sound);
    let mut sink = Sink::of(&s)?;
    let mut users: Vec<(u32, Vec<u8>)> = s.users.iter().map(|&(h, a, e)| (h, s.b[a..e].to_vec())).collect();
    let mut copied = 0;
    for &(h, a, e) in &m.users {
        let u = &m.b[a..e];
        if let Some(theirs) = s.user(h) {
            let fp = fingerprint(&m, u)?;
            if wiiu.contains(&(h, fp.0, fp.1)) || fingerprint(&s, theirs)? == fp {
                continue;
            }
        }
        let new = sink.user(&m, u)?;
        match users.binary_search_by_key(&h, |x| x.0) {
            Ok(i) => users[i].1 = new,
            Err(i) => users.insert(i, (h, new)),
        }
        copied += 1;
    }

    let mut o = Out::default();
    o.0.extend_from_slice(b"XLNK");
    o.u32(0);
    o.0.extend_from_slice(&s.b[8..12]);
    o.u32(le32(s.b, 0xc)? + sink.add_values);
    o.u32(s.n_asset + sink.add_assets);
    o.u32(s.n_tow + sink.add_tow);
    let hdr_pos = o.len();
    o.pad(16);
    o.u32(s.counts[0]);
    o.u32(s.counts[1]);
    for v in [sink.direct.len(), sink.random.len(), sink.curves.len(), sink.points.len()] {
        o.u32(v as u32);
    }
    let hdr_ex = o.len();
    o.pad(8);
    o.u32(users.len() as u32);
    o.pad(4);
    let hdr_cond = o.len();
    o.pad(16);
    for (h, _) in &users {
        o.u32(*h);
    }
    o.align(0, 8);
    let ptrs = o.len();
    o.pad(8 * users.len());
    o.0.extend_from_slice(&s.b[s.pdt.clone()]);
    o.0.extend(&sink.assets);
    let tow_at = o.len();
    o.0.extend(&sink.tow);
    let lp_at = o.len();
    o.0.extend_from_slice(&s.b[s.lp_at..s.direct_at]);
    o.0.extend(sink.direct.iter().flat_map(|x| x.to_le_bytes()));
    o.0.extend(sink.random.iter().flatten());
    o.0.extend(sink.curves.iter().flatten());
    o.0.extend(sink.points.iter().flatten());
    let ex_at = o.len();
    o.0.extend(&sink.ex);
    for (i, (_, u)) in users.iter().enumerate() {
        let at = o.len() as u64;
        o.put64(ptrs + 8 * i, at);
        o.0.extend(u);
    }
    let cond_at = o.len();
    o.0.extend(&sink.cond);
    let name_at = o.len();
    o.0.extend(&sink.names);
    let size = o.len() as u32;
    o.0[4..8].copy_from_slice(&size.to_le_bytes());
    o.put64(hdr_pos, tow_at as u64);
    o.put64(hdr_pos + 8, lp_at as u64);
    o.put64(hdr_ex, ex_at as u64);
    o.put64(hdr_cond, cond_at as u64);
    o.put64(hdr_cond + 8, name_at as u64);
    Ok((o.0, copied))
}
