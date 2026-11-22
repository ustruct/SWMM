use anyhow_ext::{Context, Result, bail};

fn be16(d: &[u8], o: usize) -> Result<usize> {
    Ok(u16::from_be_bytes(d.get(o..o + 2).context("the layout is cut short")?.try_into()?) as usize)
}

fn be32(d: &[u8], o: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("the layout is cut short")?.try_into()?) as usize)
}

fn sw16(d: &mut [u8], o: usize) -> Result<()> {
    d.get_mut(o..o + 2).context("the layout is cut short")?.reverse();
    Ok(())
}

fn sw32(d: &mut [u8], o: usize) -> Result<()> {
    d.get_mut(o..o + 4).context("the layout is cut short")?.reverse();
    Ok(())
}

fn sw32s(d: &mut [u8], from: usize, to: usize) -> Result<()> {
    let mut o = from;
    while o + 4 <= to {
        sw32(d, o)?;
        o += 4;
    }
    Ok(())
}

fn put32(d: &mut [u8], o: usize, v: usize) {
    d[o..o + 4].copy_from_slice(&(v as u32).to_le_bytes());
}

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

fn cstr(d: &[u8], o: usize) -> Result<&[u8]> {
    let s = d.get(o..).context("the layout is cut short")?;
    Ok(&s[..s.iter().position(|&b| b == 0).context("a name without its end")?])
}

fn sized(mut d: Vec<u8>) -> Vec<u8> {
    let n = d.len();
    put32(&mut d, 4, n);
    d
}

pub fn to_switch(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < 0x14 || !matches!(&raw[..4], b"FLYT" | b"FLAN") || raw[4..6] != [0xfe, 0xff] {
        bail!("not a Wii U layout");
    }
    let header = be16(raw, 6)?;
    let count = be16(raw, 0x10)?;
    let mut out = raw.get(..header).context("the layout is cut short")?.to_vec();
    out[4..6].copy_from_slice(&[0xff, 0xfe]);
    out[6..8].reverse();
    out[8..12].copy_from_slice(&0x0800_0000u32.to_le_bytes());
    out[0x10..0x12].reverse();
    let mut o = header;
    for _ in 0..count {
        let size = be32(raw, o + 4)?;
        let sec = raw.get(o..o + size).context("a section is cut short")?;
        out.extend(section(sec).with_context(|| format!("section {}", String::from_utf8_lossy(&sec[..4])))?);
        o += size;
    }
    let n = out.len();
    put32(&mut out, 0xc, n);
    Ok(out)
}

fn section(sec: &[u8]) -> Result<Vec<u8>> {
    let mut d = sec.to_vec();
    sw32(&mut d, 4)?;
    match &sec[..4] {
        b"pan1" | b"bnd1" => pane(&mut d)?,
        b"pic1" => {
            pane(&mut d)?;
            sw16(&mut d, 0x64)?;
            let n = d.len();
            sw32s(&mut d, 0x68, n)?;
        }
        b"wnd1" => window(&mut d, sec)?,
        b"txt1" => return text(sec),
        b"prt1" => return part(sec),
        b"mat1" => return materials(sec),
        b"txl1" => return names(sec, true),
        b"fnl1" => return names(sec, false),
        b"lyt1" => {
            sw16(&mut d, 0xa)?;
            sw32s(&mut d, 0xc, 0x1c)?;
        }
        b"grp1" => sw16(&mut d, 0x2a)?,
        b"usd1" => user_data(&mut d, sec)?,
        b"cnt1" => control(&mut d, sec)?,
        b"pas1" | b"pae1" | b"grs1" | b"gre1" => {}
        b"pat1" => return anim_tag(sec),
        b"pai1" => return anim_info(sec),
        m => bail!("layout section {} isn't known", String::from_utf8_lossy(m)),
    }
    Ok(d)
}

fn pane(d: &mut [u8]) -> Result<()> {
    sw32s(d, 0x2c, 0x54)
}

fn window(d: &mut [u8], sec: &[u8]) -> Result<()> {
    pane(d)?;
    for o in (0x54..0x64).step_by(2) {
        sw16(d, o)?;
    }
    let (content, frames, count) = (be32(sec, 0x68)?, be32(sec, 0x6c)?, sec[0x64] as usize);
    sw32(d, 0x68)?;
    sw32(d, 0x6c)?;
    sw16(d, content + 0x10)?;
    let coords = *sec.get(content + 0x12).context("the layout is cut short")? as usize;
    sw32s(d, content + 0x14, content + 0x14 + 32 * coords)?;
    for i in 0..count {
        let f = be32(sec, frames + 4 * i)?;
        sw32(d, frames + 4 * i)?;
        sw16(d, f)?;
    }
    Ok(())
}

fn text(sec: &[u8]) -> Result<Vec<u8>> {
    let mut d = sec.to_vec();
    sw32(&mut d, 4)?;
    pane(&mut d)?;
    for o in [0x54, 0x56, 0x58, 0x5a] {
        sw16(&mut d, o)?;
    }
    d[0x5e] |= 0x20;
    sw32(&mut d, 0x60)?;
    sw32s(&mut d, 0x70, 0x80)?;
    sw32s(&mut d, 0x84, 0x94)?;
    sw32(&mut d, 0x9c)?;
    let (buffer, text_at, id_at) = (be16(sec, 0x54)?, be32(sec, 0x64)?, be32(sec, 0x80)?);
    let header = if text_at == 0xa0 || sec.len() <= 0xa0 { 0xa0 } else { 0xa4 };
    let chars_at = if header == 0xa4 { be32(sec, 0xa0)? } else { 0 };
    let shift = 0xa8 - header;
    let mut len = 0;
    while sec.get(text_at + len..text_at + len + 2).is_some_and(|c| c != [0, 0]) {
        len += 2;
    }
    let next = [id_at, chars_at].into_iter().filter(|&o| o != 0 && o >= text_at).min().unwrap_or(sec.len());
    let len = buffer.max(len + 2).min(next.saturating_sub(text_at) & !1);
    for i in (0..len).step_by(2) {
        sw16(&mut d, text_at + i)?;
    }
    if chars_at != 0 {
        sw32(&mut d, chars_at)?;
        sw32(&mut d, chars_at + 4)?;
        if sec.get(chars_at + 0xa).copied().unwrap_or(0) != 0 {
            curves(&mut d, sec, chars_at + 0xc)?;
        }
    }
    let mut out = d[..0xa0].to_vec();
    out.extend([0; 4]);
    out.extend((if chars_at != 0 { (chars_at + shift) as u32 } else { 0 }).to_le_bytes());
    out.extend_from_slice(d.get(header..).unwrap_or_default());
    if text_at != 0 {
        put32(&mut out, 0x64, text_at + shift);
    }
    if id_at != 0 {
        put32(&mut out, 0x80, id_at + shift);
    }
    Ok(sized(out))
}

fn curves(d: &mut [u8], sec: &[u8], g: usize) -> Result<()> {
    let n = *sec.get(g + 4).context("the layout is cut short")? as usize;
    for k in 0..n {
        let a = g + be32(sec, g + 8 + 4 * k)?;
        sw32(d, g + 8 + 4 * k)?;
        let (kind, keys) = (*sec.get(a + 2).context("the layout is cut short")?, be16(sec, a + 4)?);
        sw16(d, a + 4)?;
        sw16(d, a + 6)?;
        let at = a + be32(sec, a + 8)?;
        sw32(d, a + 8)?;
        match kind {
            2 => sw32s(d, at, at + 12 * keys)?,
            1 => {
                for i in 0..keys {
                    sw32(d, at + 8 * i)?;
                    sw16(d, at + 8 * i + 4)?;
                }
            }
            k => bail!("animation curve type {k} isn't known"),
        }
    }
    Ok(())
}

fn names(sec: &[u8], strip: bool) -> Result<Vec<u8>> {
    let n = be16(sec, 8)?;
    let mut list = Vec::with_capacity(n);
    for i in 0..n {
        let s = cstr(sec, 0xc + be32(sec, 0xc + 4 * i)?)?;
        list.push(if strip { s.strip_suffix(b".bflim").unwrap_or(s) } else { s });
    }
    let mut out = sec[..0xc].to_vec();
    out[8..10].reverse();
    let mut strings = Vec::new();
    for s in &list {
        out.extend(((4 * n + strings.len()) as u32).to_le_bytes());
        strings.extend_from_slice(s);
        strings.push(0);
    }
    out.extend(strings);
    pad4(&mut out);
    Ok(sized(out))
}

fn materials(sec: &[u8]) -> Result<Vec<u8>> {
    let n = be16(sec, 8)?;
    let offsets = (0..n).map(|i| be32(sec, 0xc + 4 * i)).collect::<Result<Vec<_>>>()?;
    let mut out = sec[..0xc].to_vec();
    out[8..10].reverse();
    out.resize(0xc + 4 * n, 0);
    for i in 0..n {
        let end = offsets.get(i + 1).copied().unwrap_or(sec.len());
        let m = sec.get(offsets[i]..end).context("a material is cut short")?;
        let at = out.len();
        put32(&mut out, 0xc + 4 * i, at);
        out.extend(material(m)?);
    }
    Ok(sized(out))
}

fn material_size(flags: usize) -> usize {
    let count = |shift: u32, bits: u32| (flags >> shift) & ((1 << bits) - 1);
    0x28 + 4 * count(0, 2)
        + 20 * count(2, 2)
        + 8 * count(4, 2)
        + 4 * count(6, 3)
        + 8 * count(9, 1)
        + 4 * count(10, 1)
        + 4 * count(12, 1)
        + 12 * count(14, 1)
        + 20 * count(15, 2)
        + 8 * count(17, 1)
}

fn tool_flags(flags: usize) -> usize {
    (flags & !0x3c0) | (((flags >> 6) & 3) << 6) | (((flags >> 8) & 1) << 9)
}

fn material(m: &[u8]) -> Result<Vec<u8>> {
    let mut flags = be32(m, 0x24)?;
    if material_size(flags) > m.len() && flags & 0x200 == 0 && material_size(tool_flags(flags)) <= m.len() {
        log::warn!(
            "material {}: its flags use another layout tool's order ({flags:#x}); read as {:#x}",
            String::from_utf8_lossy(m.get(..0x1c).unwrap_or_default().split(|b| *b == 0).next().unwrap_or_default()),
            tool_flags(flags)
        );
        flags = tool_flags(flags);
    }
    let mut out = m.get(..0x1c).context("a material is cut short")?.to_vec();
    out.extend((flags as u32).to_le_bytes());
    out.extend([0, 2, 4, 8]);
    out.extend_from_slice(&m[0x1c..0x24]);
    let mut p = 0x28;
    let mut part = |out: &mut Vec<u8>, size: usize, f: &dyn Fn(&mut [u8]) -> Result<()>| -> Result<()> {
        let mut b = m.get(p..p + size).context("a material is cut short")?.to_vec();
        f(&mut b)?;
        out.extend(b);
        p += size;
        Ok(())
    };
    let count = |shift: u32, bits: u32| (flags >> shift) & ((1 << bits) - 1);
    for _ in 0..count(0, 2) {
        part(&mut out, 4, &|b| sw16(b, 0))?;
    }
    for _ in 0..count(2, 2) {
        part(&mut out, 20, &|b| sw32s(b, 0, 20))?;
    }
    for _ in 0..count(4, 2) {
        part(&mut out, 8, &|b| sw32(b, 4))?;
        out.extend([0; 8]);
    }
    for _ in 0..count(6, 3) {
        part(&mut out, 4, &|b| {
            if b[0] >= 9 {
                b[0] += 2;
            }
            Ok(())
        })?;
    }
    if count(9, 1) == 1 {
        part(&mut out, 8, &|b| sw32(b, 4))?;
    }
    if count(10, 1) == 1 {
        part(&mut out, 4, &|_| Ok(()))?;
    }
    if count(12, 1) == 1 {
        part(&mut out, 4, &|_| Ok(()))?;
    }
    if count(14, 1) == 1 {
        part(&mut out, 12, &|b| sw32s(b, 0, 12))?;
    }
    for _ in 0..count(15, 2) {
        part(&mut out, 20, &|b| sw32s(b, 0, 16))?;
    }
    if count(17, 1) == 1 {
        part(&mut out, 8, &|_| Ok(()))?;
    }
    out.extend_from_slice(&m[p.min(m.len())..]);
    Ok(out)
}

fn user_data(d: &mut [u8], sec: &[u8]) -> Result<()> {
    let n = be16(sec, 8)?;
    sw16(d, 8)?;
    for i in 0..n {
        let e = 0xc + 12 * i;
        let (data, count, kind) = (be32(sec, e + 4)?, be16(sec, e + 8)?, *sec.get(e + 0xa).context("cut short")?);
        sw32(d, e)?;
        sw32(d, e + 4)?;
        sw16(d, e + 8)?;
        match kind {
            0 => {}
            1 | 2 => sw32s(d, e + data, e + data + 4 * count)?,
            k => bail!("user data type {k} isn't known"),
        }
    }
    Ok(())
}

fn control(d: &mut [u8], sec: &[u8]) -> Result<()> {
    let (panes_at, panes, anims, pane_params, anim_params) =
        (be32(sec, 0xc)?, be16(sec, 0x10)?, be16(sec, 0x12)?, be32(sec, 0x14)?, be32(sec, 0x18)?);
    for o in [8, 0xc, 0x14, 0x18] {
        sw32(d, o)?;
    }
    sw16(d, 0x10)?;
    sw16(d, 0x12)?;
    let anims_at = panes_at + 24 * panes;
    sw32s(d, anims_at, anims_at + 4 * anims)?;
    if pane_params != 0 {
        sw32s(d, pane_params, pane_params + 4 * panes)?;
    }
    if anim_params != 0 {
        sw32s(d, anim_params, anim_params + 4 * anims)?;
    }
    Ok(())
}

fn part(sec: &[u8]) -> Result<Vec<u8>> {
    let n = be32(sec, 0x54)?;
    let table_end = 0x60 + 0x28 * n;
    let mut items = Vec::new();
    for i in 0..n {
        for f in 0..3 {
            let o = be32(sec, 0x60 + 0x28 * i + 0x1c + 4 * f)?;
            if o != 0 {
                items.push((o, f));
            }
        }
    }
    items.sort();
    items.dedup();
    let first = items.first().map_or(sec.len(), |i| i.0);
    let mut out = sec.get(..first).context("the part is cut short")?.to_vec();
    sw32(&mut out, 4)?;
    pane(&mut out)?;
    sw32s(&mut out, 0x54, 0x60)?;
    for i in 0..n {
        sw32s(&mut out, 0x60 + 0x28 * i + 0x1c, 0x60 + 0x28 * i + 0x28)?;
    }
    let _ = table_end;
    let mut moved = std::collections::HashMap::new();
    let mut cursor = first;
    for (k, &(o, f)) in items.iter().enumerate() {
        if o < cursor {
            bail!("part properties overlap");
        }
        out.extend_from_slice(&sec[cursor..o]);
        moved.insert(o, out.len());
        let next = items.get(k + 1).map_or(sec.len(), |i| i.0);
        let size = match f {
            0 | 1 => be32(sec, o + 4)?,
            _ => next - o,
        };
        let item = sec.get(o..o + size).context("a part property is cut short")?;
        match f {
            0 | 1 => out.extend(section(item)?),
            _ => {
                let mut b = item.to_vec();
                let end = 0x30.min(b.len());
                sw32s(&mut b, 8, end)?;
                out.extend(b);
            }
        }
        cursor = o + size;
    }
    out.extend_from_slice(&sec[cursor.min(sec.len())..]);
    for i in 0..n {
        for f in 0..3 {
            let at = 0x60 + 0x28 * i + 0x1c + 4 * f;
            let o = be32(sec, at)?;
            if o != 0 {
                put32(&mut out, at, moved[&o]);
            }
        }
    }
    Ok(sized(out))
}

fn anim_tag(sec: &[u8]) -> Result<Vec<u8>> {
    let mut d = sec.to_vec();
    sw32(&mut d, 4)?;
    for o in [8, 0xa, 0x14, 0x16] {
        sw16(&mut d, o)?;
    }
    let (name, groups) = (be32(sec, 0xc)?, be32(sec, 0x10)?);
    let mut out = d[..0x14].to_vec();
    out.extend([0; 4]);
    out.extend_from_slice(&d[0x14..]);
    put32(&mut out, 0xc, if name != 0 { name + 4 } else { 0 });
    put32(&mut out, 0x10, if groups != 0 { groups + 4 } else { 0 });
    Ok(sized(out))
}

fn anim_info(sec: &[u8]) -> Result<Vec<u8>> {
    let mut d = sec.to_vec();
    sw32(&mut d, 4)?;
    sw16(&mut d, 8)?;
    let (textures, entries, table) = (be16(sec, 0xc)?, be16(sec, 0xe)?, be32(sec, 0x10)?);
    sw16(&mut d, 0xc)?;
    sw16(&mut d, 0xe)?;
    for i in 0..entries {
        let e = be32(sec, table + 4 * i)?;
        let tags = *sec.get(e + 0x1c).context("the animation is cut short")? as usize;
        for t in 0..tags {
            let g = e + be32(sec, e + 0x20 + 4 * t)?;
            sw32(&mut d, e + 0x20 + 4 * t)?;
            curves(&mut d, sec, g)?;
        }
    }
    let mut region = Vec::new();
    let mut strings = Vec::new();
    for i in 0..textures {
        let s = cstr(sec, 0x14 + be32(sec, 0x14 + 4 * i)?)?;
        region.extend(((4 * textures + strings.len()) as u32).to_le_bytes());
        strings.extend_from_slice(s.strip_suffix(b".bflim").unwrap_or(s));
        strings.push(0);
    }
    region.extend(strings);
    let mut out = d[..0x14].to_vec();
    out.extend(region);
    pad4(&mut out);
    let new_table = out.len();
    out.extend_from_slice(d.get(table..).context("the animation is cut short")?);
    put32(&mut out, 0x10, new_table);
    for i in 0..entries {
        let e = be32(sec, table + 4 * i)?;
        put32(&mut out, new_table + 4 * i, e + new_table - table);
    }
    Ok(sized(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_tools_material_flags_are_read_in_the_games_order() {
        let mut m = vec![0u8; 0x28];
        m[..4].copy_from_slice(b"W_Se");
        m[0x24..0x28].copy_from_slice(&0x140u32.to_be_bytes());
        m.extend([1, 2, 3, 4]);
        m.extend([6, 0, 0, 0, 0x3f, 0, 0, 0]);
        let out = material(&m).unwrap();
        assert_eq!(u32::from_le_bytes(out[0x1c..0x20].try_into().unwrap()), 0x240);
        assert_eq!(&out[out.len() - 12..out.len() - 8], &[1, 2, 3, 4]);
        assert_eq!(material_size(0x240), m.len());
    }
}
