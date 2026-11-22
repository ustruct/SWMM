use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{self, Margin, RichText, Stroke},
    icons::{Icon, IconButtonExt},
    visuals,
};

use super::{chrome, App, Message};

impl App {
    pub fn render_crash(&mut self, ctx: &egui::Context) {
        let Some(report) = self.crash.clone() else {
            return;
        };
        let warn = ctx.style().visuals.warn_fg_color;
        let suspects: Vec<(swmm_core::mods::Mod, usize)> = report
            .suspects
            .iter()
            .filter_map(|(hash, files)| {
                self.mods
                    .iter()
                    .find(|m| m.hash() == *hash && m.enabled)
                    .map(|m| (m.clone(), files.len()))
            })
            .collect();
        chrome::dialog(ctx, "crashDialog", Icon::Warning, Some(warn), &"SW_Crash_Title".localize(), 520.0, |ui| {
            let palette = visuals::palette(ui);
            ui.label("SW_Crash_Desc".localize());
            if !report.reason.is_empty() {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(report.reason.as_str()).monospace().size(11.0).color(palette.muted)).wrap());
            }
            ui.add_space(12.0);
            if suspects.is_empty() {
                ui.label(RichText::new("SW_Crash_NoSuspects".localize()).color(palette.muted));
            } else {
                ui.label(RichText::new("SW_Crash_Suspects".localize()).strong());
                ui.add_space(4.0);
                for (mod_, files) in &suspects {
                    ui.horizontal(|ui| {
                        ui.label(Icon::Puzzle.rich(11.0).color(warn));
                        ui.label(mod_.meta.name.as_str());
                        ui.label(RichText::new(format!("({files})")).size(11.0).color(palette.muted));
                    });
                }
            }
            if !report.files.is_empty() {
                ui.add_space(8.0);
                egui::CollapsingHeader::new(RichText::new("SW_Crash_Files".localize()).size(12.0))
                    .id_source("crashFiles")
                    .show(ui, |ui| {
                        for file in report.files.iter().take(30) {
                            ui.label(RichText::new(file.as_str()).monospace().size(11.0));
                        }
                    });
            }
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("SW_Crash_Log".localize()).size(12.0))
                .id_source("crashLog")
                .show(ui, |ui| {
                    egui::Frame::none()
                        .fill(palette.base)
                        .stroke(Stroke::new(1.0_f32, palette.border))
                        .rounding(6.0)
                        .inner_margin(Margin::same(8.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(180.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for line in &report.tail {
                                        ui.label(RichText::new(line.as_str()).monospace().size(10.5).color(palette.muted));
                                    }
                                });
                        });
                });
            chrome::dialog_buttons(ui, |ui| {
                if chrome::fancy_button(ui, Icon::Check, &"Generic_Close".localize(), 86.0, 30.0).clicked() {
                    self.do_update(Message::CloseCrash);
                }
                if !suspects.is_empty()
                    && ui
                        .icon_text_button("SW_Crash_Disable".localize(), Icon::EyeSlash)
                        .on_hover_text("SW_Crash_Disable_Desc".localize())
                        .clicked()
                {
                    let mods = suspects.iter().map(|(m, _)| m.clone()).collect();
                    self.do_update(Message::ToggleMods(Some(mods), false));
                    self.do_update(Message::CloseCrash);
                }
                if let Some(log) = report.log.clone() {
                    if ui.icon_text_button("SW_Crash_OpenLog".localize(), Icon::Article).clicked() {
                        open::that(log).unwrap_or(());
                    }
                }
                if ui.icon_text_button("SW_Crash_Copy".localize(), Icon::Copy).clicked() {
                    ui.ctx().copy_text(format!("{}\n\n{}", report.reason, report.tail.join("\n")));
                }
            });
        });
    }
}
