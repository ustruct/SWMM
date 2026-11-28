
use std::{collections::HashSet, sync::LazyLock};

use anyhow_ext::{Context, Result};
use roead::{
    sarc::{Sarc, SarcWriter},
    yaz0::{compress, decompress_if},
};
use uk_content::message::{Endianness, Entry, Msyt};

static STALE: LazyLock<HashSet<u64>> = LazyLock::new(|| {
    include_bytes!("stale_texts.bin")
        .chunks_exact(8)
        .map(|n| u64::from_le_bytes([n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7]]))
        .collect()
});

pub fn text_hash(language: &str, file: &str, name: &str, entry: &Entry) -> u64 {
    let content = serde_json::to_string(entry).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [language, file, name, content.as_str()] {
        for byte in part.bytes().chain([0]) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

pub fn member(language: &str) -> String {
    format!("Message/Msg_{language}.product.ssarc")
}

pub fn language(file_name: &str) -> Option<&str> {
    let language = file_name.strip_prefix("Bootup_")?.strip_suffix(".pack")?;
    (language.len() == 4 && language != "Graphics").then_some(language)
}

fn repacked(original: &[u8], data: Vec<u8>) -> Vec<u8> {
    if original.starts_with(b"Yaz0") {
        compress(data)
    } else {
        data
    }
}

pub fn language_pack(theirs: &[u8], game: &[u8], language: &str) -> Result<Option<(Vec<u8>, usize)>> {
    language_pack_with(theirs, game, language, &STALE)
}

fn language_pack_with(
    theirs: &[u8],
    game: &[u8],
    language: &str,
    stale: &HashSet<u64>,
) -> Result<Option<(Vec<u8>, usize)>> {
    let (mod_bytes, game_bytes) = (decompress_if(theirs), decompress_if(game));
    let (mod_pack, game_pack) = (Sarc::new(&*mod_bytes)?, Sarc::new(&*game_bytes)?);
    let member = member(language);
    let (Some(mod_member), Some(game_member)) = (mod_pack.get_data(&member), game_pack.get_data(&member)) else {
        return Ok(None);
    };
    let (mod_texts, game_texts) = (decompress_if(mod_member), decompress_if(game_member));
    let (mod_texts, game_texts) = (Sarc::new(&*mod_texts)?, Sarc::new(&*game_texts)?);
    let mut writer = SarcWriter::from_sarc(&mod_texts);
    let mut count = 0;
    for file in mod_texts.files() {
        let Some(name) = file.name().filter(|name| name.ends_with(".msbt")) else { continue };
        let game_file = game_texts.get_data(name);
        if game_file == Some(file.data()) {
            continue;
        }
        let Ok(mut texts) = Msyt::from_msbt_bytes(file.data()) else { continue };
        let old: Vec<String> = texts
            .entries
            .iter()
            .filter(|(key, entry)| stale.contains(&text_hash(language, name, key, entry)))
            .map(|(key, _)| key.clone())
            .collect();
        if old.is_empty() {
            continue;
        }
        let game_texts = match game_file {
            Some(data) => Some(Msyt::from_msbt_bytes(data).with_context(|| format!("The game's {name} can't be read"))?),
            None => None,
        };
        for key in &old {
            match game_texts.as_ref().and_then(|game| game.entries.get(key)) {
                Some(entry) => {
                    texts.entries.insert(key.clone(), entry.clone());
                }
                None => {
                    texts.entries.shift_remove(key);
                }
            }
        }
        count += old.len();
        let new = match (game_file, &game_texts) {
            (Some(data), Some(game)) if game.entries == texts.entries => data.to_vec(),
            _ => {
                let endian = if file.data().get(8..10) == Some(&[0xFE, 0xFF]) { Endianness::Big } else { Endianness::Little };
                texts.into_msbt_bytes(endian).with_context(|| format!("Failed to write {name}"))?
            }
        };
        writer.files.insert(name.into(), new);
    }
    if count == 0 {
        return Ok(None);
    }
    let mut pack = SarcWriter::from_sarc(&mod_pack);
    pack.files.insert(member.as_str().into(), repacked(mod_member, writer.to_binary()));
    Ok(Some((repacked(theirs, pack.to_binary()), count)))
}
