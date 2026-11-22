use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::Arc,
};

use rustc_hash::FxHasher;
use swmm_core::{
    conflicts::{self, Overlap},
    mods::Mod,
};
use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{self, Color32, RichText, Ui},
    icons::Icon,
    visuals,
};

use super::{App, Message, Sort};

#[derive(Default)]
pub struct ConflictCache {
    key: Option<u64>,
    overlaps: Arc<Vec<Overlap>>,
}

impl App {
    pub fn overlaps(&self) -> Arc<Vec<Overlap>> {
        let mods: Vec<Mod> = self.core.mod_manager().all_mods().collect();
        let mut hasher = FxHasher::default();
        for mod_ in &mods {
            mod_.hash().hash(&mut hasher);
            mod_.enabled.hash(&mut hasher);
            for option in &mod_.enabled_options {
                option.path.hash(&mut hasher);
            }
        }
        let key = hasher.finish();
        let mut cache = self.conflicts.borrow_mut();
        if cache.key != Some(key) {
            cache.overlaps = Arc::new(conflicts::overlaps(&mods));
            cache.key = Some(key);
        }
        cache.overlaps.clone()
    }

    pub fn overridden_mods(&self) -> HashSet<usize> {
        self.overlaps()
            .iter()
            .filter(|o| o.replaced() > 0)
            .map(|o| o.loser)
            .collect()
    }

    fn mod_name(&self, hash: usize) -> String {
        self.mods
            .iter()
            .find(|m| (*m).hash() == hash)
            .map(|m| m.meta.name.to_string())
            .unwrap_or_else(|| "?".into())
    }

    pub fn render_conflicts(&self, ui: &mut Ui, mod_: &Mod) {
        let palette = visuals::palette(ui);
        let overlaps = self.overlaps();
        let hash = mod_.hash();
        let loses: Vec<&Overlap> = overlaps.iter().filter(|o| o.loser == hash).collect();
        let wins: Vec<&Overlap> = overlaps.iter().filter(|o| o.winner == hash).collect();
        ui.add_space(10.0);
        ui.label(RichText::new("SW_Conflicts".localize()).size(13.0).strong().color(palette.muted));
        ui.add_space(4.0);
        if !mod_.enabled || (loses.is_empty() && wins.is_empty()) {
            ui.horizontal(|ui| {
                ui.label(Icon::Check.rich(11.0).color(visuals::GREEN));
                ui.label(RichText::new("SW_Conflicts_None".localize()).size(12.0).color(palette.muted));
            });
            return;
        }
        let can_move = self.list_state.sort == Sort::Priority
            && !self.list_state.descending
            && self.selected.len() == 1
            && self.mod_list_filter.is_empty();
        let warn = ui.visuals().warn_fg_color;
        for overlap in loses {
            let other = overlap.winner;
            let color = if overlap.replaced() > 0 { warn } else { palette.muted };
            self.conflict_row(ui, overlap, Icon::ArrowDown, color, "SW_Conflicts_LosesTo", other, |app, ui| {
                if can_move
                    && overlap.replaced() > 0
                    && ui
                        .small_button("SW_Conflicts_MoveBelow".localize())
                        .on_hover_text("SW_Conflicts_MoveBelow_Desc".localize())
                        .clicked()
                {
                    if let Some(index) = app.mods.iter().position(|m| m.hash() == other) {
                        app.do_update(Message::MoveSelected(index));
                    }
                }
            });
        }
        for overlap in wins {
            self.conflict_row(ui, overlap, Icon::ArrowUp, visuals::GREEN, "SW_Conflicts_WinsOver", overlap.loser, |_, _| {});
        }
        ui.add_space(2.0);
        ui.label(RichText::new("SW_Conflicts_Hint".localize()).size(11.0).color(palette.muted));
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_row(
        &self,
        ui: &mut Ui,
        overlap: &Overlap,
        icon: Icon,
        color: Color32,
        label: &'static str,
        other: usize,
        extra: impl FnOnce(&Self, &mut Ui),
    ) {
        let palette = visuals::palette(ui);
        let id = ui.make_persistent_id(("conflict", overlap.winner, overlap.loser));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.label(icon.rich(10.0).color(color));
                ui.label(RichText::new(format!("{} {}", label.localize(), self.mod_name(other))).size(13.0));
                ui.label(
                    RichText::new(format!(
                        "{} · {} {}",
                        overlap.files.len(),
                        overlap.replaced(),
                        "SW_Conflicts_Replaced".localize()
                    ))
                    .size(11.0)
                    .color(palette.muted),
                );
                extra(self, ui);
            })
            .body(|ui| {
                for file in overlap.files.iter().take(200) {
                    ui.horizontal(|ui| {
                        let (tag, tag_color) = if file.merged {
                            ("SW_Conflicts_Merged".localize(), palette.muted)
                        } else {
                            ("SW_Conflicts_Replaced".localize(), ui.visuals().warn_fg_color)
                        };
                        ui.label(RichText::new(tag).size(10.0).color(tag_color));
                        ui.add(
                            egui::Label::new(RichText::new(file.path.as_str()).size(11.0).monospace().color(palette.text))
                                .truncate(),
                        );
                    });
                }
                if overlap.files.len() > 200 {
                    ui.label(RichText::new(format!("+{}", overlap.files.len() - 200)).size(11.0).color(palette.muted));
                }
            });
    }
}
