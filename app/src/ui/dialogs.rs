use swmm_i18n::string_ext::LocString;
use swmm_modpack::{Meta, ModCategory};
use net::SmartStringWrapper;

use super::*;

#[derive(Debug)]
pub struct MetaInputModal {
    meta:   Option<Meta>,
    path:   Option<PathBuf>,
    sender: Sender<Message>,
}

impl MetaInputModal {
    pub fn new(sender: Sender<Message>) -> Self {
        Self {
            meta: None,
            path: None,
            sender,
        }
    }

    pub fn clear(&mut self) {
        self.meta = None;
        self.path = None;
        self.sender.send(Message::Noop).expect("Broken channel");
    }

    pub fn take(&mut self) -> Option<Meta> {
        self.meta.take()
    }

    pub fn is_open(&self) -> bool {
        self.meta.is_some()
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        let mut should_clear = false;
        if let Some(meta) = self.meta.as_mut() {
            egui::Window::new("Info_Provide_Label".localize())
                .collapsible(false)
                .anchor(Align2::CENTER_CENTER, Vec2::default())
                .frame(Frame::window(&ctx.style()).inner_margin(8.))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label("Info_Provide_Message".localize());
                    ui.label("Info_Name".localize());
                    ui.text_edit_singleline(&mut SmartStringWrapper(&mut meta.name));
                    egui::ComboBox::new("mod-meta-cat", "Info_Category".localize())
                        .selected_text(meta.category.to_loc_str().localize())
                        .show_ui(ui, |ui| {
                            ModCategory::iter().for_each(|cat| {
                                ui.selectable_value(
                                    &mut meta.category,
                                    *cat,
                                    cat.to_loc_str().localize()
                                );
                            });
                        });
                    ui.label("Info_Description".localize());
                    ui.small("Generic_MarkdownSupported".localize());
                    let string = ui.create_temp_string(
                        "mod-meta-desc",
                        Some(meta.description.as_str().into()),
                    );
                    if egui::TextEdit::multiline(string.write().deref_mut())
                        .desired_width(f32::INFINITY)
                        .show(ui)
                        .response
                        .changed()
                    {
                        meta.description = string.read().as_str().into();
                    }
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), ui.min_size().y),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                if ui.button("Generic_OK".localize()).clicked() {
                                    self.sender
                                        .send(Message::OpenMod(
                                            self.path
                                                .take()
                                                .expect("There should be a mod path here"),
                                        ))
                                        .expect("Broken channel");
                                }
                                if ui.button("Generic_Close".localize()).clicked() {
                                    should_clear = true;
                                }
                                ui.shrink_width_to_current();
                            },
                        );
                    });
                });
            if should_clear {
                self.clear();
            }
        }
    }
}

impl App {
    pub fn render_error(&mut self, ctx: &egui::Context) {
        if let Some(err) = self.error.as_ref() {
            let mut chain = err.chain();
            let label = match chain.len() {
                0 | 1 => chrome::clean_error(err),
                _ => format!(
                    "{}: {}",
                    chain.next().map(chrome::clean_error).unwrap_or_default(),
                    chain.next_back().map(chrome::clean_error).unwrap_or_default()
                ),
            };
            let error_color = ctx.style().visuals.error_fg_color;
            chrome::dialog(ctx, "errorDialog", Icon::Warning, Some(error_color), &"Error_Label".localize(), 480.0, |ui| {
                ui.label(label);
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Error_Details".localize()).show(ui, |ui| {
                    egui::ScrollArea::vertical().drag_to_scroll(false).max_height(180.0).show(ui, |ui| {
                        err.chain().enumerate().for_each(|(i, e)| {
                            ui.label(RichText::new(format!("{i}. {}", chrome::clean_error(e))).code());
                        });
                    });
                });
                if let Some(context) = err.chain().find_map(|e| {
                    e.downcast_ref::<swmm_formats::FormatError>()
                        .and_then(|e| e.context_data())
                }) {
                    egui::CollapsingHeader::new("Error_Context".localize()).show(ui, |ui| {
                        egui::ScrollArea::vertical().drag_to_scroll(false).max_height(180.0).show(ui, |ui| {
                            ui.label(format!("{:#?}", context));
                        });
                    });
                }
                chrome::dialog_buttons(ui, |ui| {
                    if chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 86.0, 30.0).clicked() {
                        self.do_update(Message::CloseError);
                    }
                    if ui.icon_text_button("Generic_Copy".localize(), Icon::Copy).clicked() {
                        ui.output_mut(|o| o.copied_text = format!("{:?}", &err));
                        self.do_update(Message::Toast("Generic_Copied".localize().into()));
                    }
                });
            });
        }
    }

    pub fn render_confirm(&mut self, ctx: &egui::Context) {
        let Some((_, text)) = self.confirm.as_ref() else {
            return;
        };
        let text = text.clone();
        chrome::dialog(ctx, "confirmDialog", Icon::Help, None, &"Generic_Confirm".localize(), 420.0, |ui| {
            ui.label(text);
            chrome::dialog_buttons(ui, |ui| {
                if chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 86.0, 30.0).clicked() {
                    if let Some((msg, _)) = self.confirm.take() {
                        self.do_update(msg);
                    }
                    self.do_update(Message::CloseConfirm);
                }
                if ui.add(egui::Button::new("Generic_Cancel".localize()).min_size(Vec2::new(80.0, 30.0))).clicked() {
                    self.do_update(Message::CloseConfirm);
                }
            });
        });
    }

    pub fn render_new_profile(&mut self, ctx: &egui::Context) {
        if self.new_profile.is_none() {
            return;
        }
        let mut close = false;
        chrome::dialog(ctx, "newProfileDialog", Icon::People, None, &"Profile_New".localize(), 380.0, |ui| {
            ui.label("Profile_New_Label".localize());
            ui.add_space(6.0);
            let name = self.new_profile.as_mut().unwrap();
            let response = ui.add(
                egui::TextEdit::singleline(name)
                    .desired_width(f32::INFINITY)
                    .margin(Vec2::new(8.0, 5.0)),
            );
            response.request_focus();
            let valid = !name.trim().is_empty();
            let submit = valid && response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            chrome::dialog_buttons(ui, |ui| {
                let ok = ui
                    .add_enabled_ui(valid, |ui| chrome::fancy_button(ui, Icon::Add, &"Generic_OK".localize(), 86.0, 30.0))
                    .inner;
                if ok.clicked() || submit {
                    self.do_update(Message::AddProfile);
                }
                if ui.add(egui::Button::new("Generic_Cancel".localize()).min_size(Vec2::new(80.0, 30.0))).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    close = true;
                }
            });
        });
        if close {
            self.new_profile = None;
        }
    }

    pub fn render_busy(&self, ctx: &egui::Context, _frame: &eframe::Frame) {
        if !self.busy.get() {
            return;
        }
        let palette = visuals::ctx_palette(ctx);
        let style = ctx.style();
        let t = chrome::window_in(ctx, Id::new("busyDialog"), 0.24);
        egui::Window::new("Busy_Working".localize())
            .id(Id::new("busyDialog"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - t) * 16.0))
            .min_width(340.0)
            .max_width(340.0)
            .frame(Frame::window(&style).inner_margin(Margin::same(20.0)).rounding(12.0))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.horizontal(|ui| {
                    chrome::progress_ring(ui, 34.0, palette.accent_text);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new("Busy_Processing".localize())
                                .family(egui::FontFamily::Name("Bold".into()))
                                .size(15.0)
                                .color(palette.text),
                        );
                        let progress = crate::logger::LOGGER
                            .get_progress()
                            .unwrap_or_else(|| "Busy_Working".localize().into_owned());
                        ui.add(
                            Label::new(RichText::new(progress).size(12.0).color(palette.muted))
                                .wrap_mode(egui::TextWrapMode::Truncate),
                        );
                    });
                });
                ui.add_space(14.0);
                let (track, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), egui::Sense::hover());
                ui.painter().rect_filled(track, 1.5, palette.border);
                chrome::indeterminate_bar(ui.painter(), track, ui.input(|i| i.time), visuals::ACCENT);
            });
    }

    pub fn render_about(&self, ctx: &egui::Context) {
        if !self.show_about {
            chrome::appear(ctx, Id::new("aboutLogo"), false, 0.5);
            return;
        }
        let palette = visuals::ctx_palette(ctx);
        let style = ctx.style();
        let t_in = chrome::window_in(ctx, Id::new("aboutDialog"), 0.24);
        egui::Window::new("Menu_Help_About".localize())
            .id(Id::new("aboutDialog"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - t_in) * 16.0))
            .min_width(420.0)
            .max_width(420.0)
            .frame(Frame::window(&style).inner_margin(Margin::same(24.0)).rounding(12.0))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.vertical_centered(|ui| {
                    let t = chrome::appear(ctx, Id::new("aboutLogo"), true, 0.5);
                    ui.add_space((1.0 - t) * 10.0);
                    super::frame::logo(ui, 64.0 + 8.0 * t);
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new("SWMM")
                                .family(egui::FontFamily::Name(swmm_widgets::TITLE_FAMILY.into()))
                                .size(34.0)
                                .color(palette.text),
                        )
                        .selectable(false),
                    );
                    ui.add_space(2.0);
                    ui.label(RichText::new("Savage Wind Mod Manager").color(palette.muted));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .size(11.5)
                            .color(chrome::mix(palette.muted, palette.panel, 0.35)),
                    );
                });
                ui.add_space(14.0);
                egui::Grid::new("aboutLinks").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    for (name, text, url) in [
                        ("Savage Wind", "github.com/Revivedx/SavageWindRecomp", "https://github.com/Revivedx/SavageWindRecomp"),
                        ("Mods", "gamebanana.com", "https://gamebanana.com/games/6386"),
                    ] {
                        ui.label(RichText::new(name).color(palette.muted));
                        if ui.link(text).clicked() {
                            open::that(url).unwrap_or(());
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new("Free software under the GNU GPL v3  ·").size(11.5).color(palette.muted));
                    if ui
                        .add(egui::Label::new(RichText::new("Licenses").size(11.5).color(palette.accent_text)).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.do_update(Message::ShowLicenses);
                    }
                });
                chrome::dialog_buttons(ui, |ui| {
                    if chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 86.0, 30.0).clicked() {
                        self.do_update(Message::CloseAbout);
                    }
                });
            });
    }

    pub fn render_licenses(&self, ctx: &egui::Context) {
        if !self.show_licenses {
            return;
        }
        chrome::dialog(ctx, "licensesDialog", Icon::Info, None, "Licenses", 500.0, |ui| {
            let palette = visuals::palette(ui);
            let heading = |ui: &mut Ui, text: &str| {
                ui.label(
                    RichText::new(text)
                        .family(egui::FontFamily::Name("Bold".into()))
                        .size(14.0)
                        .color(palette.text),
                );
            };
            let body = |ui: &mut Ui, text: &str| {
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(palette.muted)).wrap());
            };
            egui::ScrollArea::vertical().drag_to_scroll(false).max_height(360.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                heading(ui, "SWMM");
                body(ui, crate::COPYRIGHT);
                body(
                    ui,
                    "SWMM is based on U-King Mod Manager (UKMM) by Caleb Smith and Ginger Chody, used under the GNU \
                     General Public License v3.",
                );
                ui.add_space(8.0);
                heading(ui, "GNU General Public License v3");
                body(
                    ui,
                    "SWMM is free software: you can redistribute it and/or modify it under the terms of the GNU General \
                     Public License as published by the Free Software Foundation, either version 3 of the License, or \
                     (at your option) any later version. It is distributed without any warranty.",
                );
                if ui.link("Read the full license").clicked() {
                    open::that("https://www.gnu.org/licenses/gpl-3.0.html").unwrap_or(());
                }
                ui.add_space(8.0);
                heading(ui, "Third-party assets");
                body(ui, "Icons: Font Awesome Free, SIL Open Font License 1.1.");
                if ui.link("fontawesome.com").clicked() {
                    open::that("https://fontawesome.com").unwrap_or(());
                }
                body(ui, "Title font: The Wild Breath of Zelda by Chequered Ink (freeware).");
            });
            chrome::dialog_buttons(ui, |ui| {
                if chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 86.0, 30.0).clicked() {
                    self.do_update(Message::CloseLicenses);
                }
            });
        });
    }

    pub fn render_changelog(&self, ctx: &egui::Context) {
        let Some(text) = self.changelog.as_ref() else {
            return;
        };
        chrome::dialog(ctx, "welcomeDialog", Icon::Wind, None, &"SW_Welcome_Title".localize(), 560.0, |ui| {
            egui::ScrollArea::vertical().drag_to_scroll(false).max_height(380.0).show(ui, |ui| {
                chrome::rich_text(ui, text);
            });
            chrome::dialog_buttons(ui, |ui| {
                if chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 96.0, 30.0).clicked() {
                    self.do_update(Message::CloseChangelog);
                }
            });
        });
    }
}
