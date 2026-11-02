use std::path::PathBuf;

use fs_err as fs;
use smartstring::alias::String as SmartString;
use swmm_core::mods::Profile as ProfileData;
use swmm_formats::util::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct ProfileManagerState {
    pub dir: PathBuf,
    pub profiles: HashMap<SmartString, ProfileData>,
    pub selected: Option<SmartString>,
    pub rename: Option<String>,
}

impl ProfileManagerState {
    pub fn new(core: &swmm_core::core::Manager) -> Self {
        let settings = core.settings();
        let dir = settings.profiles_dir();
        let profiles = settings
            .profiles()
            .filter_map(|name| -> Option<(SmartString, ProfileData)> {
                let path = dir.join(name.as_str()).join("profile.yml");
                let text = fs::read_to_string(path).ok()?;
                let data = serde_yaml::from_str(&text).ok()?;
                Some((name, data))
            })
            .collect::<_>();
        Self {
            dir,
            profiles,
            selected: None,
            rename: None,
        }
    }

    pub fn reload(&mut self, core: &swmm_core::core::Manager) {
        let settings = core.settings();
        self.profiles = settings
            .profiles()
            .filter_map(|name| -> Option<(SmartString, ProfileData)> {
                let path = self.dir.join(name.as_str()).join("profile.yml");
                let text = fs::read_to_string(path).ok()?;
                let data = serde_yaml::from_str(&text).ok()?;
                Some((name, data))
            })
            .collect::<_>();
    }

    #[inline]
    pub fn all_assigned_mod_hashes(&self) -> HashSet<usize> {
        self.profiles
            .values()
            .map(|m| m.mods()
                .keys()
                .copied()
                .collect::<Vec<_>>()
            )
            .flatten()
            .collect()
    }
}
