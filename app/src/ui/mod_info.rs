use std::{
    hash::{Hash, Hasher},
    io::{BufReader, Read},
    sync::{Arc, LazyLock},
};

use anyhow::Result;
use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHasher};
use swmm_i18n::string_ext::LocString;
use swmm_core::mods::Mod;
use uk_mod::Manifest;
#[allow(deprecated)]
use swmm_widgets::egui_extras::RetainedImage;
use swmm_widgets::{
    egui::{self, Ui},
    PathNode,
};

#[repr(transparent)]
pub struct ModInfo<'a>(pub &'a Mod);

impl ModInfo<'_> {
    #[allow(deprecated)]
    pub fn preview(&self) -> Option<Arc<RetainedImage>> {
        fn load_preview(mod_: &Mod) -> Result<Option<Arc<RetainedImage>>> {
            let mut zip = zip::ZipArchive::new(BufReader::new(std::fs::File::open(&mod_.path)?))?;
            for ext in ["jpg", "jpeg", "png", "svg"] {
                if let Ok(mut file) = zip.by_name(&format!("thumb.{}", ext)) {
                    let mut vec = vec![0; file.size() as usize];
                    file.read_exact(&mut vec)?;
                    return Ok(Some(Arc::new(
                        RetainedImage::from_image_bytes(mod_.meta.name.as_str(), &vec)
                            .map_err(|e| anyhow::anyhow!("{}", e))?,
                    )));
                }
            }
            Ok(None)
        }
        static PREVIEW: LazyLock<RwLock<FxHashMap<usize, Option<Arc<RetainedImage>>>>> =
            LazyLock::new(|| RwLock::new(FxHashMap::default()));
        let mut preview = PREVIEW.write();
        preview
            .entry(self.0.hash())
            .or_insert_with(|| {
                match load_preview(self.0) {
                    Ok(pre) => pre,
                    Err(e) => {
                        log::error!("Error loading mod preview: {}", e);
                        None
                    }
                }
            })
            .clone()
    }
}

pub static ROOTS: LazyLock<RwLock<FxHashMap<u64, PathNode>>> =
    LazyLock::new(|| RwLock::new(FxHashMap::default()));

pub fn render_manifest(manifest: &Manifest, ui: &mut Ui) {
    ui.scope(|ui| {
        ui.style_mut().override_text_style = Some(egui::TextStyle::Body);
        ui.spacing_mut().item_spacing.y = 4.;
        if !manifest.content_files.is_empty() {
            let mut hasher = FxHasher::default();
            manifest.content_files.hash(&mut hasher);
            let mut roots = ROOTS.write();
            let content_root = roots.entry(hasher.finish()).or_insert_with(|| {
                let val = "Info_Manifest_BaseFiles".localize();
                let mut root = PathNode::dir(&val);
                manifest.content_files.iter().for_each(|file| {
                    root.build_tree(&file.split('/').map(|s| s.to_owned()).collect(), 0);
                });
                root
            });
            content_root.render_dir(ui);
        }
        if !manifest.aoc_files.is_empty() {
            let mut hasher = FxHasher::default();
            manifest.aoc_files.hash(&mut hasher);
            let mut roots = ROOTS.write();
            let aoc_root = roots.entry(hasher.finish()).or_insert_with(|| {
                let val = "Info_Manifest_DLCFiles".localize();
                let mut root = PathNode::dir(&val);
                manifest.aoc_files.iter().for_each(|file| {
                    root.build_tree(&file.split('/').map(|s| s.to_owned()).collect(), 0);
                });
                root
            });
            aoc_root.render_dir(ui);
        }
    });
}
