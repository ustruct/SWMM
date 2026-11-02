pub mod string_ext;

use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

pub static LOCALIZATION: LazyLock<RwLock<Localization>> =
    LazyLock::new(|| RwLock::new(Localization::from(LocLang::English)));

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LocLang {
    English,
    Dutch,
    French,
    German,
    Italian,
    Japanese,
    Korean,
    Russian,
    SimpleChinese,
    Spanish,
}

impl From<LocLang> for &str {
    fn from(value: LocLang) -> Self {
        value.to_str()
    }
}

impl std::fmt::Display for LocLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_str())
    }
}

impl LocLang {
    pub fn iter() -> std::slice::Iter<'static, Self> {
        [
            Self::English,
            Self::Dutch,
            Self::French,
            Self::German,
            Self::Italian,
            Self::Japanese,
            Self::Korean,
            Self::Russian,
            Self::SimpleChinese,
            Self::Spanish,
        ]
        .iter()
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Dutch => "Nederlands",
            Self::French => "Français",
            Self::German => "Deutsch",
            Self::Italian => "Italiano",
            Self::Japanese => "日本語",
            Self::Korean => "한국어",
            Self::Russian => "Русский",
            Self::SimpleChinese => "简体中文",
            Self::Spanish => "Español",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::English => include_str!("../localization/en.json"),
            Self::Dutch => include_str!("../localization/nl.json"),
            Self::French => include_str!("../localization/fr.json"),
            Self::German => include_str!("../localization/de.json"),
            Self::Italian => include_str!("../localization/it.json"),
            Self::Japanese => include_str!("../localization/ja.json"),
            Self::Korean => include_str!("../localization/ko.json"),
            Self::Russian => include_str!("../localization/ru.json"),
            Self::SimpleChinese => include_str!("../localization/zh.json"),
            Self::Spanish => include_str!("../localization/es.json"),
        }
    }

    fn strings(self) -> HashMap<&'static str, Cow<'static, str>> {
        serde_json::from_str(self.source()).unwrap_or_default()
    }
}

pub struct Localization {
    pub language: LocLang,
    strings: HashMap<&'static str, Cow<'static, str>>,
    fallback: HashMap<&'static str, Cow<'static, str>>,
}

impl From<LocLang> for Localization {
    fn from(language: LocLang) -> Self {
        Self {
            language,
            strings: language.strings(),
            fallback: LocLang::English.strings(),
        }
    }
}

impl Localization {
    pub fn get(&self, key: &'static str) -> Cow<'static, str> {
        self.strings
            .get(key)
            .or_else(|| self.fallback.get(key))
            .cloned()
            .unwrap_or(Cow::Borrowed(key))
    }

    pub fn update_language(&mut self, language: &LocLang) {
        self.strings = language.strings();
        self.language = *language;
    }
}
