use std::path::PathBuf;

use crate::{mods::Mod, settings::Settings};

const CRASH_MARKERS: &[&str] = &[
    "[plat] access violation",
    "[plat] exception",
    "unhandled exception",
    "segmentation fault",
    "[crash]",
    "fatal error",
    "panicked at",
    "abort()",
];
const CLEAN_EXIT_MARKERS: &[&str] = &["window closed"];
const TAIL_LINES: usize = 40;

#[derive(Debug, Clone, Default)]
pub struct CrashReport {
    pub log: Option<PathBuf>,
    pub reason: String,
    pub tail: Vec<String>,
    pub files: Vec<String>,
    pub suspects: Vec<(usize, Vec<String>)>,
}

pub fn log_path(settings: &Settings) -> Option<PathBuf> {
    #[cfg(windows)]
    let dir = settings
        .savage_wind
        .executable
        .as_ref()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    #[cfg(not(windows))]
    let dir = crate::savage_wind::data_root(&settings.savage_wind);
    dir.map(|d| d.join("savagewind.log")).filter(|p| p.is_file())
}

fn crash_line(text: &str) -> Option<&str> {
    text.lines().find(|line| {
        let lower = line.to_ascii_lowercase();
        CRASH_MARKERS.iter().any(|m| lower.contains(m))
    })
}

fn exited_cleanly(text: &str) -> bool {
    text.lines()
        .rev()
        .take(TAIL_LINES)
        .any(|line| CLEAN_EXIT_MARKERS.iter().any(|m| line.trim() == *m))
}

fn game_paths(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '(' | ')' | '[' | ']' | ':'))
        .filter(|w| w.contains('/') && w.rsplit('/').next().is_some_and(|n| n.contains('.')))
        .map(|w| {
            let w = w.replace('\\', "/");
            for root in ["/romfs/", "/content/", "/overlay/"] {
                if let Some(i) = w.rfind(root) {
                    return w[i + root.len()..].to_string();
                }
            }
            w.trim_start_matches('/').to_string()
        })
}

pub fn analyze(text: &str, exit_ok: bool, mods: &[Mod]) -> Option<CrashReport> {
    let marker = crash_line(text);
    if marker.is_none() && (exit_ok || exited_cleanly(text)) {
        return None;
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("[vk] t=")).collect();
    let tail: Vec<String> = lines
        .iter()
        .skip(lines.len().saturating_sub(TAIL_LINES))
        .map(|l| l.to_string())
        .collect();
    let mut files: Vec<String> = Vec::new();
    for line in lines.iter().rev().take(400) {
        let lower = line.to_ascii_lowercase();
        if !(lower.contains("fail") || lower.contains("cannot") || lower.contains("error") || lower.contains("missing")
            || lower.contains("not found") || lower.contains("invalid"))
        {
            continue;
        }
        for path in game_paths(line) {
            if !files.contains(&path) {
                files.push(path);
            }
        }
    }
    let mut suspects = Vec::new();
    for mod_ in mods.iter().filter(|m| m.enabled) {
        let Ok(manifest) = mod_.manifest() else {
            continue;
        };
        let hits: Vec<String> = files
            .iter()
            .filter(|f| {
                manifest.content_files.iter().chain(manifest.aoc_files.iter()).any(|own| {
                    let own = own.as_str();
                    f.ends_with(own) || own.ends_with(f.as_str())
                })
            })
            .cloned()
            .collect();
        if !hits.is_empty() {
            suspects.push((mod_.hash(), hits));
        }
    }
    suspects.sort_by_key(|(_, hits)| std::cmp::Reverse(hits.len()));
    Some(CrashReport {
        log: None,
        reason: marker.map(|m| m.trim().to_string()).unwrap_or_default(),
        tail,
        files,
        suspects,
    })
}

pub fn check(settings: &Settings, exit_ok: bool, mods: &[Mod]) -> Option<CrashReport> {
    let path = log_path(settings)?;
    let text = std::fs::read(&path).ok()?;
    let text = String::from_utf8_lossy(&text);
    analyze(&text, exit_ok, mods).map(|mut report| {
        report.log = Some(path);
        report
    })
}
