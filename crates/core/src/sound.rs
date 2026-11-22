use anyhow_ext::{Context, Result, bail};

struct Swap<'a> {
    d: &'a mut [u8],
}

impl Swap<'_> {
    fn n(&mut self, o: usize, size: usize) -> Result<()> {
        self.d.get_mut(o..o + size).context("the stream is cut short")?.reverse();
        Ok(())
    }
    fn u16(&mut self, o: usize) -> Result<()> {
        self.n(o, 2)
    }
    fn u32(&mut self, o: usize) -> Result<()> {
        self.n(o, 4)
    }
    fn reference(&mut self, o: usize) -> Result<Option<usize>> {
        let off = i32::from_be_bytes(self.d.get(o + 4..o + 8).context("short")?.try_into()?);
        self.u16(o)?;
        self.u32(o + 4)?;
        Ok((off >= 0).then_some(off as usize))
    }
    fn be32(&self, o: usize) -> Result<u32> {
        Ok(u32::from_be_bytes(self.d.get(o..o + 4).context("short")?.try_into()?))
    }
}

pub fn bfstm_to_switch(src: &[u8]) -> Result<Vec<u8>> {
    let prefetch = src.get(..4) == Some(b"FSTP");
    if src.get(..4) != Some(b"FSTM") && !prefetch {
        bail!("not a BFSTM stream");
    }
    if src.get(4..6) == Some(&[0xff, 0xfe]) {
        return Ok(src.to_vec());
    }
    let version = u32::from_be_bytes(src.get(8..12).context("the stream is cut short")?.try_into()?);
    if version != 0x0004_0000 && version != 0x0003_0000 && version != 0x0002_0100 {
        bail!("BFSTM version {version:#x} isn't supported yet");
    }
    let mut out = long_layout(src)?;
    let mut s = Swap { d: &mut out };
    let blocks = u16::from_be_bytes([s.d[0x10], s.d[0x11]]) as usize;
    s.d[4] = 0xff;
    s.d[5] = 0xfe;
    s.u16(6)?;
    s.u32(8)?;
    s.u32(0xc)?;
    s.u16(0x10)?;
    let mut info = None;
    let mut seek = None;
    let mut data = None;
    let mut regions = None;
    let mut pdat = None;
    for i in 0..blocks {
        let r = 0x14 + 12 * i;
        let kind = u16::from_be_bytes([s.d[r], s.d[r + 1]]);
        let off = s.be32(r + 4)? as usize;
        match kind {
            0x4000 => info = Some(off),
            0x4001 => seek = Some(off),
            0x4002 => data = Some(off),
            0x4003 => regions = Some(off),
            0x4004 if prefetch => pdat = Some((i, off)),
            0 if off == 0 => {}
            k => bail!("unknown block {k:#x} in the stream"),
        }
        s.u16(r)?;
        s.u32(r + 4)?;
        s.u32(r + 8)?;
    }
    let info = info.context("the stream has no INFO block")?;
    s.u32(info + 4)?;
    let body = info + 8;
    let stream = s.reference(body)?.context("no stream info")? + body;
    let tracks = s.reference(body + 8)?.map(|o| o + body);
    let channels = s.reference(body + 16)?.map(|o| o + body);
    let codec = s.d[stream];
    let region_count = s.d[stream + 3] as usize;
    let region_size = u16::from_be_bytes([s.d[stream + 0x38], s.d[stream + 0x39]]);
    if codec != 0 && codec != 2 {
        bail!("sound codec {codec} isn't supported yet (DSP-ADPCM and 8-bit PCM are)");
    }
    for k in 0..11 {
        s.u32(stream + 4 + 4 * k)?;
    }
    let sample_ref = s.reference(stream + 0x30)?;
    s.u16(stream + 0x38)?;
    s.reference(stream + 0x3c)?;
    s.u32(stream + 0x44)?;
    s.u32(stream + 0x48)?;
    if let Some(t) = tracks {
        let n = s.be32(t)? as usize;
        s.u32(t)?;
        for i in 0..n {
            if let Some(o) = s.reference(t + 4 + 8 * i)? {
                let track = t + o;
                if let Some(o2) = s.reference(track + 4)? {
                    let table = track + o2;
                    s.u32(table)?;
                }
            }
        }
    }
    if let Some(c) = channels {
        let n = s.be32(c)? as usize;
        s.u32(c)?;
        for i in 0..n {
            if let Some(o) = s.reference(c + 4 + 8 * i)? {
                let ch = c + o;
                if let Some(o2) = s.reference(ch)?
                    && codec == 2 {
                        for k in 0..23 {
                            s.u16(ch + o2 + 2 * k)?;
                        }
                    }
            }
        }
    }
    if let Some(sk) = seek {
        let size = s.be32(sk + 4)? as usize;
        s.u32(sk + 4)?;
        let mut o = sk + 8;
        while o + 2 <= sk + size {
            s.u16(o)?;
            o += 2;
        }
    }
    if let Some(r) = regions {
        s.u32(r + 4)?;
        let size = region_size as usize;
        if size < 8 {
            bail!("the stream's regions have no size");
        }
        for k in 0..region_count {
            let e = r + 0x20 + size * k;
            s.u32(e)?;
            s.u32(e + 4)?;
            for h in (8..size - size % 2).step_by(2) {
                s.u16(e + h)?;
            }
        }
    }
    if let Some(d) = data {
        s.u32(d + 4)?;
    }
    let Some((block, p)) = pdat else {
        let data = block_offset(&out, blocks, 0x4002);
        return Ok(match (data, sample_ref) {
            (Some(d), Some(samples)) => align_samples(out, blocks, d, samples),
            _ => out,
        });
    };
    s.u32(p + 4)?;
    let mut n = s.be32(p + 8)? as usize;
    if n == 0 {
        let first = s.be32(p + 0xc + 0x10)? as usize;
        if first >= 0x14 && first.is_multiple_of(0x14) && p + 0xc + first < s.d.len() {
            n = first / 0x14;
        }
    }
    s.d[p + 8..p + 12].copy_from_slice(&(n as u32).to_be_bytes());
    s.u32(p + 8)?;
    let mut entries = Vec::with_capacity(n);
    for k in 0..n {
        let e = p + 0xc + 0x14 * k;
        let size = s.be32(e + 4)? as usize;
        s.u32(e)?;
        s.u32(e + 4)?;
        s.u32(e + 8)?;
        let off = s.reference(e + 0xc)?.context("prefetch data without its place")?;
        entries.push((e, p + 0xc + off, size));
    }
    let table_end = p + 0xc + 0x14 * n;
    let mut new = out[..table_end].to_vec();
    let mut first_moved = None;
    for (e, at, size) in &entries {
        let to = new.len().div_ceil(0x40) * 0x40;
        new.resize(to, 0);
        new.extend_from_slice(out.get(*at..*at + *size).context("prefetch data is cut short")?);
        new[e + 0x10..e + 0x14].copy_from_slice(&((to - (p + 0xc)) as u32).to_le_bytes());
        first_moved.get_or_insert((*at, to));
    }
    let size = new.len() - p;
    new[p + 4..p + 8].copy_from_slice(&(size as u32).to_le_bytes());
    let r = 0x14 + 12 * block;
    new[r + 8..r + 12].copy_from_slice(&(size as u32).to_le_bytes());
    let file = new.len() as u32;
    new[0xc..0x10].copy_from_slice(&file.to_le_bytes());
    if let (Some(_), Some((_, to))) = (sample_ref, first_moved) {
        new[stream + 0x34..stream + 0x38].copy_from_slice(&(to as u32).to_le_bytes());
    }
    Ok(new)
}

fn long_layout(src: &[u8]) -> Result<Vec<u8>> {
    let be32 = |b: &[u8], o: usize| -> Result<u32> {
        Ok(u32::from_be_bytes(b.get(o..o + 4).context("the stream is cut short")?.try_into()?))
    };
    let be16 = |b: &[u8], o: usize| -> Result<u16> {
        Ok(u16::from_be_bytes(b.get(o..o + 2).context("the stream is cut short")?.try_into()?))
    };
    let mut out = src.to_vec();
    let mut blocks = be16(&out, 0x10)? as usize;
    while blocks > 0 && be16(&out, 0x14 + 12 * (blocks - 1))? == 0 && be32(&out, 0x18 + 12 * (blocks - 1))? == 0 {
        blocks -= 1;
    }
    out[0x10..0x12].copy_from_slice(&(blocks as u16).to_be_bytes());
    let info = (0..blocks)
        .map(|i| 0x14 + 12 * i)
        .find(|&r| be16(src, r).ok() == Some(0x4000))
        .map(|r| be32(src, r + 4))
        .transpose()?
        .context("the stream has no INFO block")? as usize;
    let info_size = be32(&out, info + 4)? as usize;
    let body = info + 8;
    let stream_off = be32(&out, body + 4)? as usize;
    let stream = body + stream_off;
    let refs: Vec<usize> = [body + 8, body + 16]
        .into_iter()
        .filter(|&r| be32(&out, r + 4).is_ok_and(|o| o != u32::MAX))
        .collect();
    let next = refs.iter().map(|&r| be32(&out, r + 4).map(|o| o as usize)).collect::<Result<Vec<_>>>()?;
    let short = next.iter().min().is_some_and(|&n| n == stream_off + 0x44);
    if be16(&out, stream + 0x38)? == 0 && out[stream + 3] == 0 {
        out[stream + 0x38..stream + 0x3a].copy_from_slice(&0x100u16.to_be_bytes());
        out[stream + 0x3c..stream + 0x40].copy_from_slice(&[0; 4]);
        out[stream + 0x40..stream + 0x44].copy_from_slice(&u32::MAX.to_be_bytes());
    }
    if !short {
        return Ok(out);
    }
    let at = stream + 0x44;
    let (loop_start, frames) = (be32(&out, stream + 8)?, be32(&out, stream + 0xc)?);
    let end = info + info_size;
    let slack = out.get(at..end).context("the stream is cut short")?.iter().rev().take_while(|&&b| b == 0).count();
    let grow = if slack >= 8 { 0 } else { 0x20 };
    let mut extra = loop_start.to_be_bytes().to_vec();
    extra.extend(frames.to_be_bytes());
    out.splice(at..at, extra);
    if grow == 0 {
        out.drain(end..end + 8);
    } else {
        out.splice(end + 8..end + 8, std::iter::repeat_n(0u8, grow - 8));
    }
    for r in refs {
        let o = be32(&out, r + 4)? as usize;
        if body + o >= at {
            out[r + 4..r + 8].copy_from_slice(&((o + 8) as u32).to_be_bytes());
        }
    }
    if grow > 0 {
        let new_size = (info_size + grow) as u32;
        out[info + 4..info + 8].copy_from_slice(&new_size.to_be_bytes());
        for r in (0..blocks).map(|i| 0x14 + 12 * i) {
            let o = be32(&out, r + 4)? as usize;
            if o == info {
                out[r + 8..r + 12].copy_from_slice(&new_size.to_be_bytes());
            } else if o > info && o != 0 {
                out[r + 4..r + 8].copy_from_slice(&((o + grow) as u32).to_be_bytes());
            }
        }
        let file = out.len() as u32;
        out[0xc..0x10].copy_from_slice(&file.to_be_bytes());
    }
    if &out[..4] == b"FSTM" && be32(&out, 8)? < 0x0004_0000 {
        out[8..12].copy_from_slice(&0x0004_0000u32.to_be_bytes());
    }
    Ok(out)
}

fn block_offset(out: &[u8], blocks: usize, kind: u16) -> Option<usize> {
    (0..blocks).map(|i| 0x14 + 12 * i).find_map(|r| {
        (u16::from_le_bytes([out[r], out[r + 1]]) == kind)
            .then(|| u32::from_le_bytes([out[r + 4], out[r + 5], out[r + 6], out[r + 7]]) as usize)
    })
}

fn align_samples(mut out: Vec<u8>, blocks: usize, d: usize, samples: usize) -> Vec<u8> {
    let start = d + 8 + samples;
    let pad = (0x40 - start % 0x40) % 0x40;
    if pad == 0 {
        return out;
    }
    let le32 = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let Some(prev) = (0..blocks)
        .map(|i| 0x14 + 12 * i)
        .filter(|&r| (le32(&out, r + 4) as usize) < d)
        .max_by_key(|&r| le32(&out, r + 4))
    else {
        return out;
    };
    let prev_at = le32(&out, prev + 4) as usize;
    out.splice(d..d, std::iter::repeat_n(0u8, pad));
    for r in (0..blocks).map(|i| 0x14 + 12 * i) {
        if le32(&out, r + 4) as usize == d {
            out[r + 4..r + 8].copy_from_slice(&((d + pad) as u32).to_le_bytes());
        }
    }
    let grown = le32(&out, prev + 8) + pad as u32;
    out[prev + 8..prev + 12].copy_from_slice(&grown.to_le_bytes());
    out[prev_at + 4..prev_at + 8].copy_from_slice(&grown.to_le_bytes());
    let file = out.len() as u32;
    out[0xc..0x10].copy_from_slice(&file.to_le_bytes());
    out
}

pub fn bfwav_to_switch(src: &[u8]) -> Result<Vec<u8>> {
    if src.get(..4) != Some(b"FWAV") {
        bail!("not a BFWAV sound");
    }
    if src.get(4..6) == Some(&[0xff, 0xfe]) {
        return Ok(src.to_vec());
    }
    let mut out = src.to_vec();
    let mut s = Swap { d: &mut out };
    let version = s.be32(8)?;
    if version != 0x0001_0200 && version != 0x0001_0100 {
        bail!("BFWAV version {version:#x} isn't supported yet");
    }
    let blocks = u16::from_be_bytes([s.d[0x10], s.d[0x11]]) as usize;
    s.d[4] = 0xff;
    s.d[5] = 0xfe;
    s.u16(6)?;
    s.u32(8)?;
    s.d[8..12].copy_from_slice(&0x0001_0200u32.to_le_bytes());
    s.u32(0xc)?;
    s.u16(0x10)?;
    let (mut info, mut data) = (None, None);
    for i in 0..blocks {
        let r = 0x14 + 12 * i;
        let kind = u16::from_be_bytes([s.d[r], s.d[r + 1]]);
        let off = s.be32(r + 4)? as usize;
        match kind {
            0x7000 => info = Some(off),
            0x7001 => data = Some((i, off)),
            0 if off == 0 => {}
            k => bail!("unknown block {k:#x} in the sound"),
        }
        s.u16(r)?;
        s.u32(r + 4)?;
        s.u32(r + 8)?;
    }
    let info = info.context("the sound has no INFO block")?;
    let (data_block, data) = data.context("the sound has no DATA block")?;
    s.u32(info + 4)?;
    let body = info + 8;
    let codec = s.d[body];
    if codec > 2 {
        bail!("sound codec {codec} isn't supported yet (PCM and DSP-ADPCM are)");
    }
    let samples = s.be32(body + 0xc)? as usize;
    for k in 0..4 {
        s.u32(body + 4 + 4 * k)?;
    }
    let table = body + 0x14;
    let n = s.be32(table)? as usize;
    s.u32(table)?;
    let mut sample_refs = Vec::with_capacity(n);
    for i in 0..n {
        let ch = table + s.reference(table + 4 + 8 * i)?.context("a channel without its info")?;
        let sample_off = s.reference(ch)?.context("a channel without its samples")?;
        sample_refs.push((ch, sample_off));
        if let Some(o) = s.reference(ch + 8)?
            && codec == 2 {
                for k in 0..23 {
                    s.u16(ch + o + 2 * k)?;
                }
            }
        s.u32(ch + 0x10)?;
    }
    s.u32(data + 4)?;
    let len = match codec {
        0 => samples,
        1 => samples * 2,
        _ => samples.div_ceil(14) * 8,
    };
    let start = data + 8;
    let mut new = out[..start].to_vec();
    for (ch, off) in &sample_refs {
        let at = start + off;
        let mut bytes = out.get(at..at + len).context("the sound's samples are cut short")?.to_vec();
        if codec == 1 {
            for w in bytes.chunks_mut(2) {
                w.reverse();
            }
        }
        let to = new.len().div_ceil(0x40) * 0x40;
        new.resize(to, 0);
        new.extend_from_slice(&bytes);
        new[ch + 4..ch + 8].copy_from_slice(&((to - start) as u32).to_le_bytes());
    }
    let size = new.len() - data;
    new[data + 4..data + 8].copy_from_slice(&(size as u32).to_le_bytes());
    let r = 0x14 + 12 * data_block;
    new[r + 8..r + 12].copy_from_slice(&(size as u32).to_le_bytes());
    let file = new.len() as u32;
    new[0xc..0x10].copy_from_slice(&file.to_le_bytes());
    Ok(new)
}

pub fn bars_to_switch(src: &[u8], name: &str) -> Result<Vec<u8>> {
    if src.get(..4) != Some(b"BARS") {
        bail!("not a sound archive");
    }
    if src.get(8..10) == Some(&[0xff, 0xfe]) {
        return Ok(src.to_vec());
    }
    let be = |o: usize| -> Result<u32> { Ok(u32::from_be_bytes(src.get(o..o + 4).context("cut short")?.try_into()?)) };
    let n = be(0xc)? as usize;
    let table = 0x10 + 4 * n;
    let entries: Vec<(usize, u32)> = (0..n)
        .map(|i| {
            let (a, at) = (be(table + 8 * i)? as usize, be(table + 8 * i + 4)?);
            if at != u32::MAX && at as usize >= src.len() {
                log::warn!("{name}: sound {i} points past the end of the archive: kept without its sound");
                return Ok((a, u32::MAX));
            }
            Ok((a, at))
        })
        .collect::<Result<_>>()?;
    let keep_amta = name.rsplit('/').next().unwrap_or(name).starts_with("NV_");
    let amta_end = entries.iter().map(|(a, _)| Ok(a + be(a + 8)? as usize)).collect::<Result<Vec<_>>>()?;
    let first_sound = entries.iter().filter(|e| e.1 != u32::MAX).map(|e| e.1 as usize).min();
    let head = first_sound.unwrap_or(src.len()).max(amta_end.iter().copied().max().unwrap_or(table + 8 * n));
    let mut out = src.get(..head).context("cut short")?.to_vec();
    out[8] = 0xff;
    out[9] = 0xfe;
    for o in (4..8).chain(0xc..table + 8 * n).step_by(4) {
        out[o..o + 4].reverse();
    }
    for (i, &(_, at)) in entries.iter().enumerate() {
        if at == u32::MAX {
            out[table + 8 * i + 4..table + 8 * i + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        }
    }
    for &(a, _) in &entries {
        if !keep_amta {
            amta_to_switch(&mut out, a)?;
        }
    }
    let mut order: Vec<(usize, usize)> =
        entries.iter().enumerate().filter(|(_, e)| e.1 != u32::MAX).map(|(i, e)| (e.1 as usize, i)).collect();
    order.sort();
    let mut placed: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (at, i) in order {
        let to = match placed.get(&at) {
            Some(to) => *to,
            None => {
                let size = be(at + 0xc)? as usize;
                let file = src.get(at..at + size).context("a sound is cut short")?;
                let conv = match &file[..4] {
                    b"FWAV" => bfwav_to_switch(file)?,
                    b"FSTP" => bfstm_to_switch(file)?,
                    m => bail!("unknown sound {:?} in the archive", String::from_utf8_lossy(m)),
                };
                let to = out.len().div_ceil(0x40) * 0x40;
                out.resize(to, 0);
                out.extend_from_slice(&conv);
                placed.insert(at, to);
                to
            }
        };
        out[table + 8 * i + 4..table + 8 * i + 8].copy_from_slice(&(to as u32).to_le_bytes());
    }
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

fn amta_to_switch(b: &mut [u8], a: usize) -> Result<()> {
    if b.get(a..a + 4) != Some(b"AMTA") || b.get(a + 4..a + 6) != Some(&[0xfe, 0xff]) {
        bail!("an archive entry isn't AMTA metadata");
    }
    let be = |b: &[u8], o: usize| -> Result<usize> {
        Ok(u32::from_be_bytes(b.get(o..o + 4).context("cut short")?.try_into()?) as usize)
    };
    let sections: Vec<usize> = (0..4).map(|k| be(b, a + 0xc + 4 * k)).collect::<Result<_>>()?;
    b[a + 4] = 0xff;
    b[a + 5] = 0xfe;
    b[a + 6..a + 8].reverse();
    for o in (a + 8..a + 0x1c).step_by(4) {
        b[o..o + 4].reverse();
    }
    for off in sections {
        let at = a + off;
        let magic: [u8; 4] = b.get(at..at + 4).context("cut short")?.try_into()?;
        let size = be(b, at + 4)?;
        b[at + 4..at + 8].reverse();
        let body = at + 8;
        if &magic == b"STRG" {
            continue;
        }
        if !matches!(&magic, b"DATA" | b"MARK" | b"EXT_") {
            bail!("unknown AMTA section {:?}", String::from_utf8_lossy(&magic));
        }
        for k in (0..size - size % 4).step_by(4) {
            if &magic == b"DATA" && k == 8 {
                continue;
            }
            b.get_mut(body + k..body + k + 4).context("cut short")?.reverse();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_past_the_end_are_dropped() {
        let mut b = Vec::new();
        b.extend_from_slice(b"BARS");
        b.extend(0u32.to_be_bytes());
        b.extend([0xfe, 0xff, 1, 1]);
        b.extend(1u32.to_be_bytes());
        b.extend(0u32.to_be_bytes());
        b.extend(0x20u32.to_be_bytes());
        b.extend(0xffff_ecffu32.to_be_bytes());
        b.resize(0x20, 0);
        let mut amta = b"AMTA".to_vec();
        amta.extend([0xfe, 0xff, 0, 4]);
        amta.extend(0x1cu32.to_be_bytes());
        amta.extend([0; 16]);
        b.extend(amta);
        let n = b.len() as u32;
        b[4..8].copy_from_slice(&n.to_be_bytes());
        let out = bars_to_switch(&b, "Voice/USen/NV_Test.bars").unwrap();
        assert_eq!(&out[0x18..0x1c], &u32::MAX.to_le_bytes());
    }
}
