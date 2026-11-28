use anyhow_ext::{Result, bail};

use super::bntx::Texture;

fn cubic(t: f32) -> f32 {
    let t = t.abs();
    if t < 1.0 {
        1.5 * t * t * t - 2.5 * t * t + 1.0
    } else if t < 2.0 {
        -0.5 * t * t * t + 2.5 * t * t - 4.0 * t + 2.0
    } else {
        0.0
    }
}

fn weights(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f32 / dst as f32;
    let support = scale.max(1.0);
    (0..dst)
        .map(|i| {
            let centre = (i as f32 + 0.5) * scale - 0.5;
            let lo = (centre - 2.0 * support).floor() as isize;
            let hi = (centre + 2.0 * support).ceil() as isize;
            let mut w: Vec<(usize, f32)> = (lo..=hi)
                .map(|k| (k.clamp(0, src as isize - 1) as usize, cubic((k as f32 - centre) / support)))
                .filter(|(_, w)| *w != 0.0)
                .collect();
            let sum: f32 = w.iter().map(|(_, w)| w).sum();
            if sum != 0.0 {
                w.iter_mut().for_each(|(_, x)| *x /= sum);
            }
            w
        })
        .collect()
}

pub fn resize_rgba(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let wx = weights(w, nw);
    let wy = weights(h, nh);
    let mut mid = vec![0f32; nw * h * 4];
    for y in 0..h {
        for (x, ws) in wx.iter().enumerate() {
            for c in 0..4 {
                mid[(y * nw + x) * 4 + c] = ws.iter().map(|&(k, f)| src[(y * w + k) * 4 + c] as f32 * f).sum();
            }
        }
    }
    let mut out = vec![0u8; nw * nh * 4];
    for (y, ws) in wy.iter().enumerate() {
        for x in 0..nw {
            for c in 0..4 {
                let v: f32 = ws.iter().map(|&(k, f)| mid[(k * nw + x) * 4 + c] * f).sum();
                out[(y * nw + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn to565(c: [f32; 3]) -> u16 {
    let r = (c[0].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    let g = (c[1].clamp(0.0, 255.0) * 63.0 / 255.0).round() as u16;
    let b = (c[2].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    (r << 11) | (g << 5) | b
}

fn from565(c: u16) -> [f32; 3] {
    let r = ((c >> 11) & 31) as f32;
    let g = ((c >> 5) & 63) as f32;
    let b = (c & 31) as f32;
    [r * 255.0 / 31.0, g * 255.0 / 63.0, b * 255.0 / 31.0]
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn palette4(c0: u16, c1: u16) -> [[f32; 3]; 4] {
    let (a, b) = (from565(c0), from565(c1));
    let mix = |x: f32, y: f32| [0, 1, 2].map(|i| (a[i] * x + b[i] * y) / (x + y));
    [a, b, mix(2.0, 1.0), mix(1.0, 2.0)]
}

fn block_error(px: &[[f32; 3]], pal: &[[f32; 3]]) -> f32 {
    px.iter().map(|c| pal.iter().map(|p| dist(*c, *p)).fold(f32::MAX, f32::min)).sum()
}

fn quantise_pair(px: &[[f32; 3]], a: [f32; 3], b: [f32; 3]) -> (u16, u16, f32) {
    let steps = [31.0, 63.0, 31.0];
    let cands = |c: [f32; 3]| -> [[u16; 2]; 3] {
        [0, 1, 2].map(|i| {
            let v = c[i].clamp(0.0, 255.0) * steps[i] / 255.0;
            [v.floor() as u16, (v.ceil() as u16).min(steps[i] as u16)]
        })
    };
    let (ca, cb) = (cands(a), cands(b));
    let pack = |r: u16, g: u16, b: u16| (r << 11) | (g << 5) | b;
    let mut best = (0u16, 0u16, f32::MAX);
    for m in 0..64u32 {
        let pick = |c: &[[u16; 2]; 3], bits: u32| pack(c[0][(bits & 1) as usize], c[1][((bits >> 1) & 1) as usize], c[2][((bits >> 2) & 1) as usize]);
        let (mut c0, mut c1) = (pick(&ca, m & 7), pick(&cb, m >> 3));
        if c0 == c1 {
            continue;
        }
        if c0 < c1 {
            std::mem::swap(&mut c0, &mut c1);
        }
        let e = block_error(px, &palette4(c0, c1));
        if e < best.2 {
            best = (c0, c1, e);
        }
    }
    if best.2 == f32::MAX {
        let c = to565(a);
        let (c0, c1) = if c == 0 { (1, 0) } else { (c, c - 1) };
        let e = block_error(px, &palette4(c0, c1));
        best = (c0, c1, e);
    }
    best
}

fn refine(px: &[[f32; 3]], mut a: [f32; 3], mut b: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let weights = [(1.0f32, 0.0f32), (0.0, 1.0), (2.0 / 3.0, 1.0 / 3.0), (1.0 / 3.0, 2.0 / 3.0)];
    for _ in 0..2 {
        let mix = |x: f32, y: f32| [0, 1, 2].map(|i| a[i] * x + b[i] * y);
        let pal: Vec<[f32; 3]> = weights.iter().map(|&(x, y)| mix(x, y)).collect();
        let (mut aa, mut bb, mut ab) = (0f32, 0f32, 0f32);
        let (mut ax, mut bx) = ([0f32; 3], [0f32; 3]);
        for c in px {
            let k = (0..4).min_by(|&i, &j| dist(*c, pal[i]).total_cmp(&dist(*c, pal[j]))).unwrap_or(0);
            let (wa, wb) = weights[k];
            aa += wa * wa;
            bb += wb * wb;
            ab += wa * wb;
            for i in 0..3 {
                ax[i] += wa * c[i];
                bx[i] += wb * c[i];
            }
        }
        let det = aa * bb - ab * ab;
        if det.abs() < 1e-6 {
            break;
        }
        a = [0, 1, 2].map(|i| (ax[i] * bb - bx[i] * ab) / det);
        b = [0, 1, 2].map(|i| (bx[i] * aa - ax[i] * ab) / det);
    }
    (a, b)
}

fn encode_block(px: &[[u8; 4]; 16]) -> [u8; 8] {
    let transparent = px.iter().any(|p| p[3] < 128);
    let opaque: Vec<[f32; 3]> =
        px.iter().filter(|p| p[3] >= 128).map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    if opaque.is_empty() {
        return [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
    }
    let n = opaque.len() as f32;
    let mean = [0, 1, 2].map(|c| opaque.iter().map(|p| p[c]).sum::<f32>() / n);
    let mut cov = [[0f32; 3]; 3];
    for p in &opaque {
        let d = [p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let mut axis = [0.577f32, 0.577, 0.577];
    for _ in 0..8 {
        let next = [0, 1, 2].map(|i| cov[i][0] * axis[0] + cov[i][1] * axis[1] + cov[i][2] * axis[2]);
        let len = (next[0] * next[0] + next[1] * next[1] + next[2] * next[2]).sqrt();
        if len < 1e-6 {
            break;
        }
        axis = next.map(|v| v / len);
    }
    let proj = |p: &[f32; 3]| (p[0] - mean[0]) * axis[0] + (p[1] - mean[1]) * axis[1] + (p[2] - mean[2]) * axis[2];
    let (lo, hi) = opaque.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(proj(p)), hi.max(proj(p))));
    let end = |t: f32| [mean[0] + axis[0] * t, mean[1] + axis[1] * t, mean[2] + axis[2] * t];
    let (c0, c1, palette): (u16, u16, Vec<[f32; 3]>) = if transparent {
        let (mut c0, mut c1) = (to565(end(hi)), to565(end(lo)));
        if c0 > c1 {
            std::mem::swap(&mut c0, &mut c1);
        }
        let (a, b) = (from565(c0), from565(c1));
        (c0, c1, vec![a, b, [0, 1, 2].map(|i| (a[i] + b[i]) / 2.0)])
    } else {
        let (a, b) = refine(&opaque, end(hi), end(lo));
        let (c0, c1, _) = quantise_pair(&opaque, a, b);
        (c0, c1, palette4(c0, c1).to_vec())
    };
    let mut bits = 0u32;
    for (i, p) in px.iter().enumerate() {
        let index = if transparent && p[3] < 128 {
            3
        } else {
            let c = [p[0] as f32, p[1] as f32, p[2] as f32];
            (0..palette.len()).min_by(|&x, &y| dist(c, palette[x]).total_cmp(&dist(c, palette[y]))).unwrap_or(0) as u32
        };
        bits |= index << (2 * i);
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&bits.to_le_bytes());
    out
}

pub fn encode_bc1(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut out = Vec::with_capacity(bw * bh * 8);
    for by in 0..bh {
        for bx in 0..bw {
            let mut px = [[0u8; 4]; 16];
            for (i, p) in px.iter_mut().enumerate() {
                let x = (bx * 4 + i % 4).min(w - 1);
                let y = (by * 4 + i / 4).min(h - 1);
                p.copy_from_slice(&rgba[(y * w + x) * 4..][..4]);
            }
            out.extend(encode_block(&px));
        }
    }
    out
}

fn is_rgba8(format: u32) -> bool {
    format >> 8 == 0x0b
}

fn is_bc1(format: u32) -> bool {
    format >> 8 == 0x1a
}

pub fn resize(t: &mut Texture, width: u32, height: u32) -> Result<()> {
    let (w, h) = (t.width as usize, t.height as usize);
    let (nw, nh) = (width as usize, height as usize);
    let Some(top) = t.levels.first() else { bail!("{} has no image data", t.name) };
    let rgba = if is_rgba8(t.format) {
        top.get(..w * h * 4).map(<[u8]>::to_vec)
    } else if is_bc1(t.format) {
        crate::map::decode_bc1(top, w, h).ok()
    } else {
        None
    };
    let Some(rgba) = rgba else { bail!("{}: format {:#x} can't be resized", t.name, t.format) };
    if t.layers != 1 || t.levels.len() != 1 {
        bail!("{}: only single images can be resized", t.name);
    }
    let scaled = resize_rgba(&rgba, w, h, nw, nh);
    t.levels = vec![if is_rgba8(t.format) { scaled } else { encode_bc1(&scaled, nw, nh) }];
    t.width = width;
    t.height = height;
    Ok(())
}

pub fn match_sizes(ours: &mut [Texture], theirs: &[Texture]) -> usize {
    let mut n = 0;
    for t in ours.iter_mut() {
        let Some(s) = theirs.iter().find(|s| s.name == t.name) else { continue };
        if (s.width, s.height) == (t.width, t.height) || s.format != t.format || s.levels.len() != 1 {
            continue;
        }
        if s.width as u64 * t.height as u64 != s.height as u64 * t.width as u64 {
            continue;
        }
        match resize(t, s.width, s.height) {
            Ok(()) => n += 1,
            Err(e) => log::warn!("{e:#}"),
        }
    }
    n
}
