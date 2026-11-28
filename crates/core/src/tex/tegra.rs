#[inline]
fn div_up(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

#[inline]
fn round_up(a: u32, b: u32) -> u32 {
    a.div_ceil(b) * b
}

pub fn block_height(height_in_blocks: u32) -> u32 {
    let mut bh = 1;
    while bh < 16 && 3 * height_in_blocks >= 2 * 8 * (bh * 2) {
        bh *= 2;
    }
    bh
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub offset: u32,
    pub size: u32,
    pub width_blocks: u32,
    pub height_blocks: u32,
    pub block_height: u32,
}

pub fn levels(width: u32, height: u32, mips: u32, block_w: u32, bpe: u32, bh0: u32) -> Vec<Level> {
    let mut out = Vec::new();
    let mut bh = bh0;
    let mut offset = 0;
    for m in 0..mips {
        let w = (width >> m).max(1);
        let h = (height >> m).max(1);
        let wb = div_up(w, block_w);
        let hb = div_up(h, block_w);
        if m > 0 {
            while bh > 1 && hb <= bh * 8 / 2 {
                bh /= 2;
            }
        }
        let pitch = round_up(wb * bpe, 64);
        let size = pitch * round_up(hb, bh * 8);
        out.push(Level { offset, size, width_blocks: wb, height_blocks: hb, block_height: bh });
        offset += size;
    }
    out
}

#[inline]
fn address(x: u32, y: u32, width_blocks: u32, bpe: u32, block_height: u32) -> usize {
    let gobs_per_row = div_up(width_blocks * bpe, 64);
    let xb = x * bpe;
    let gob = (y / (8 * block_height)) * 512 * block_height * gobs_per_row
        + (xb / 64) * 512 * block_height
        + (y % (8 * block_height) / 8) * 512;
    (gob + ((xb % 64) / 32) * 256 + ((y % 8) / 2) * 64 + ((xb % 32) / 16) * 32 + (y % 2) * 16 + (xb % 16)) as usize
}

pub fn swizzle(linear: &[u8], level: &Level, bpe: u32, out: &mut [u8]) {
    let (wb, hb) = (level.width_blocks, level.height_blocks);
    for y in 0..hb {
        for x in 0..wb {
            let src = ((y * wb + x) * bpe) as usize;
            let dst = address(x, y, wb, bpe, level.block_height);
            if let (Some(s), Some(d)) = (linear.get(src..src + bpe as usize), out.get_mut(dst..dst + bpe as usize)) {
                d.copy_from_slice(s);
            }
        }
    }
}

pub fn deswizzle(data: &[u8], level: &Level, bpe: u32) -> Vec<u8> {
    let (wb, hb) = (level.width_blocks, level.height_blocks);
    let mut out = vec![0u8; (wb * hb * bpe) as usize];
    for y in 0..hb {
        for x in 0..wb {
            let dst = ((y * wb + x) * bpe) as usize;
            let src = address(x, y, wb, bpe, level.block_height);
            if let Some(s) = data.get(src..src + bpe as usize) {
                out[dst..dst + bpe as usize].copy_from_slice(s);
            }
        }
    }
    out
}
