use std::path::{Path, PathBuf};

use anyhow_ext::{Context, Result};

enum Node {
    Map(Vec<(Vec<u8>, Node)>),
    Str(Vec<u8>),
    Raw(u8, Vec<u8>),
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).context("Unexpected end of shortcuts.vdf")?;
        self.pos += 1;
        Ok(b)
    }

    fn cstr(&mut self) -> Result<Vec<u8>> {
        let rest = &self.data[self.pos.min(self.data.len())..];
        let end = rest.iter().position(|b| *b == 0).context("Unterminated string in shortcuts.vdf")?;
        let s = rest[..end].to_vec();
        self.pos += end + 1;
        Ok(s)
    }

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        let s = self.data.get(self.pos..self.pos + n).context("Unexpected end of shortcuts.vdf")?.to_vec();
        self.pos += n;
        Ok(s)
    }

    fn map(&mut self) -> Result<Vec<(Vec<u8>, Node)>> {
        let mut items = Vec::new();
        loop {
            let kind = self.byte()?;
            if kind == 0x08 {
                return Ok(items);
            }
            let key = self.cstr()?;
            let node = match kind {
                0x00 => Node::Map(self.map()?),
                0x01 => Node::Str(self.cstr()?),
                0x02 | 0x03 | 0x04 | 0x06 => Node::Raw(kind, self.bytes(4)?),
                0x07 | 0x0a => Node::Raw(kind, self.bytes(8)?),
                other => anyhow_ext::bail!("Unknown value type {other} in shortcuts.vdf"),
            };
            items.push((key, node));
        }
    }
}

fn write_map(items: &[(Vec<u8>, Node)], out: &mut Vec<u8>) {
    for (key, node) in items {
        let kind = match node {
            Node::Map(_) => 0x00,
            Node::Str(_) => 0x01,
            Node::Raw(k, _) => *k,
        };
        out.push(kind);
        out.extend_from_slice(key);
        out.push(0);
        match node {
            Node::Map(inner) => {
                write_map(inner, out);
                out.push(0x08);
            }
            Node::Str(s) => {
                out.extend_from_slice(s);
                out.push(0);
            }
            Node::Raw(_, b) => out.extend_from_slice(b),
        }
    }
}

fn field<'a>(items: &'a [(Vec<u8>, Node)], name: &str) -> Option<&'a [u8]> {
    items.iter().find_map(|(k, v)| match v {
        Node::Str(s) if k.eq_ignore_ascii_case(name.as_bytes()) => Some(s.as_slice()),
        _ => None,
    })
}

fn strip_var(options: &str, var: &str) -> String {
    let needle = format!("{var}=");
    let mut out = options.to_string();
    while let Some(start) = out.find(&needle) {
        let value = start + needle.len();
        let end = if out[value..].starts_with('"') {
            out[value + 1..].find('"').map(|i| value + i + 2).unwrap_or(out.len())
        } else {
            out[value..].find(char::is_whitespace).map(|i| value + i).unwrap_or(out.len())
        };
        out.replace_range(start..end, "");
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out == "%command%" { String::new() } else { out }
}

pub fn launch_options(options: &str, var: &str, value: Option<&Path>) -> String {
    let rest = strip_var(options, var);
    let Some(path) = value else { return rest };
    let set = format!("{var}=\"{}\"", path.display());
    if rest.contains("%command%") {
        format!("{set} {rest}")
    } else if rest.is_empty() {
        format!("{set} %command%")
    } else {
        format!("{set} %command% {rest}")
    }
}

fn steam_roots() -> Vec<PathBuf> {
    let Some(home) = dirs2::home_dir() else { return Vec::new() };
    let mut roots: Vec<PathBuf> = Vec::new();
    for candidate in [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ] {
        if let Ok(real) = candidate.canonicalize() {
            if real.is_dir() && !roots.contains(&real) {
                roots.push(real);
            }
        }
    }
    roots
}

fn shortcut_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in steam_roots() {
        let Ok(users) = std::fs::read_dir(root.join("userdata")) else { continue };
        for user in users.flatten() {
            let file = user.path().join("config").join("shortcuts.vdf");
            if file.is_file() {
                files.push(file);
            }
        }
    }
    files
}

pub fn update_shortcuts(var: &str, value: Option<&Path>, is_game: impl Fn(&Path) -> bool) -> Result<usize> {
    let mut changed = 0;
    for file in shortcut_files() {
        let data = std::fs::read(&file)?;
        let mut reader = Reader { data: &data, pos: 0 };
        let mut root = reader.map().with_context(|| format!("Couldn't read {}", file.display()))?;
        let mut touched = false;
        for (_, list) in root.iter_mut() {
            let Node::Map(shortcuts) = list else { continue };
            for (_, shortcut) in shortcuts.iter_mut() {
                let Node::Map(items) = shortcut else { continue };
                let exe = field(items, "Exe").map(|e| String::from_utf8_lossy(e).trim_matches('"').to_string());
                let Some(exe) = exe.filter(|e| is_game(Path::new(e))) else { continue };
                let old = field(items, "LaunchOptions").map(|o| String::from_utf8_lossy(o).into_owned()).unwrap_or_default();
                let new = launch_options(&old, var, value);
                if new == old {
                    continue;
                }
                match items.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(b"LaunchOptions")) {
                    Some((_, node)) => *node = Node::Str(new.into_bytes()),
                    None => items.push((b"LaunchOptions".to_vec(), Node::Str(new.into_bytes()))),
                }
                log::info!("Updated the Steam shortcut for {exe}");
                touched = true;
                changed += 1;
            }
        }
        if touched {
            let mut out = Vec::with_capacity(data.len() + 128);
            write_map(&root, &mut out);
            out.push(0x08);
            let _ = std::fs::copy(&file, file.with_extension("vdf.swmm-backup"));
            std::fs::write(&file, out)?;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options_round_trip() {
        let var = "SAVAGEWIND_ROMFS_OVERLAY";
        let p = Path::new("/home/deck/mods");
        assert_eq!(launch_options("", var, Some(p)), "SAVAGEWIND_ROMFS_OVERLAY=\"/home/deck/mods\" %command%");
        assert_eq!(launch_options("-fullscreen", var, Some(p)), "SAVAGEWIND_ROMFS_OVERLAY=\"/home/deck/mods\" %command% -fullscreen");
        assert_eq!(launch_options("gamemoderun %command%", var, Some(p)), "SAVAGEWIND_ROMFS_OVERLAY=\"/home/deck/mods\" gamemoderun %command%");
        let set = launch_options("", var, Some(p));
        assert_eq!(launch_options(&set, var, None), "");
        assert_eq!(launch_options(&set, var, Some(Path::new("/x y"))), "SAVAGEWIND_ROMFS_OVERLAY=\"/x y\" %command%");
    }

    #[test]
    fn vdf_round_trip() {
        let mut data = Vec::new();
        data.extend_from_slice(b"\x00shortcuts\x00\x000\x00\x02appid\x00\x01\x02\x03\x04\x01AppName\x00Savage Wind\x00");
        data.extend_from_slice(b"\x01Exe\x00\"/games/Savage_Wind.AppImage\"\x00\x01LaunchOptions\x00\x00\x00tags\x00\x08\x08\x08\x08");
        let mut reader = Reader { data: &data, pos: 0 };
        let root = reader.map().expect("parse");
        let mut out = Vec::new();
        write_map(&root, &mut out);
        out.push(0x08);
        assert_eq!(out, data);
    }
}
