use anyhow_ext::{Context, Result, bail};

fn be32(d: &[u8], o: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(d.get(o..o + 4).context("the table is cut short")?.try_into()?) as usize)
}

fn words(d: &mut [u8], from: usize, to: usize) -> Result<()> {
    let to = to.min(d.len());
    if from > to {
        bail!("the table is cut short");
    }
    for o in (from..to - (to - from) % 4).step_by(4) {
        d[o..o + 4].reverse();
    }
    Ok(())
}

fn halves(d: &mut [u8], from: usize, to: usize) -> Result<()> {
    let to = to.min(d.len());
    if from > to {
        bail!("the table is cut short");
    }
    for o in (from..to - (to - from) % 2).step_by(2) {
        d.swap(o, o + 1);
    }
    Ok(())
}

pub fn to_switch(kind: &str, raw: &[u8]) -> Result<Vec<u8>> {
    let mut d = raw.to_vec();
    match kind {
        "baacl" | "baroc" | "baatn" | "bagst" | "blal" => {
            if d.get(4..6) != Some(&[0xfe, 0xff]) {
                bail!("not a big endian sound table");
            }
            d[4] = 0xff;
            d[5] = 0xfe;
            if kind != "blal" {
                d[6..8].reverse();
            }
            match kind {
                "baatn" => {
                    let strings = be32(raw, 8)?;
                    words(&mut d, 8, strings)?;
                }
                "bagst" => {
                    let (tree, grp, strg) = (be32(raw, 0xc)?, be32(raw, 0x10)?, be32(raw, 0x14)?);
                    if raw.get(tree..tree + 4) != Some(b"TREE")
                        || raw.get(grp..grp + 4) != Some(b"GRP ")
                        || raw.get(strg..strg + 4) != Some(b"STRG")
                    {
                        bail!("sound group table without its sections");
                    }
                    words(&mut d, 8, tree)?;
                    words(&mut d, tree + 4, grp)?;
                    words(&mut d, grp + 4, strg)?;
                    words(&mut d, strg + 4, strg + 8)?;
                }
                _ => words(&mut d, 8, raw.len())?,
            }
        }
        "fmc" => {
            let n = be32(raw, 4)?;
            let table = 8 + 4 * n;
            if be32(raw, table).ok() == Some(0x100) && be32(raw, table + 4).ok() == Some(0x108) {
                let records = table + 4 * 64 + 12 * 576;
                words(&mut d, 0, records)?;
                halves(&mut d, records, raw.len())?;
            } else {
                words(&mut d, 0, raw.len())?;
            }
        }
        "beco" => {
            if be32(raw, 0)? != 0x0011_2233 {
                bail!("not an ecosystem table");
            }
            let n = be32(raw, 4)?;
            let rows = 0x10 + 4 * n;
            words(&mut d, 0, rows)?;
            halves(&mut d, rows, raw.len())?;
        }
        k => bail!("no table layout for .{k}"),
    }
    Ok(d)
}

pub fn known(kind: &str) -> bool {
    matches!(kind, "baacl" | "baroc" | "baatn" | "bagst" | "blal" | "fmc" | "beco")
}
