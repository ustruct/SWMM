use swmm_i18n::string_ext::LocString;
use swmm_modpack::{ModOption, ModOptionGroup, OptionGroup};
use swmm_widgets::{
    egui::{self, pos2, vec2, Color32, Context, Id, Layout, Align, Margin, Rect, RichText, Rounding, Sense, Shape, Stroke, Ui, Vec2},
    icons::Icon,
    visuals,
};

use super::{chrome, App, Message};

fn option_row(ui: &mut Ui, id: Id, option: &ModOption, checked: bool, radio: bool) -> bool {
    let palette = visuals::palette(ui);
    let background = ui.painter().add(Shape::Noop);
    let width = ui.available_width();
    let inner = ui
        .allocate_ui_with_layout(vec2(width, 0.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_width(width);
            ui.add_space(8.0);
            let (indicator, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(option.name.as_str()).color(palette.text)).wrap());
                if !option.description.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(option.description.as_str()).size(12.0).color(palette.muted))
                            .wrap(),
                    );
                }
                ui.add_space(3.0);
            });
            indicator
        });
    let indicator = inner.inner;
    let rect = inner.response.rect;
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui.ctx().animate_bool_with_time(id.with("hover"), response.hovered(), 0.12);
    let on = ui.ctx().animate_bool_with_time(id.with("on"), checked, 0.16);
    if hover > 0.0 {
        ui.painter().set(
            background,
            Shape::rect_filled(rect, Rounding::same(6.0), palette.hover.gamma_multiply(hover * 0.7)),
        );
    }
    let center = pos2(indicator.center().x, indicator.top() + 11.0);
    let stroke = Stroke::new(1.5_f32, chrome::mix(palette.muted, visuals::ACCENT, on));
    if radio {
        ui.painter().circle_stroke(center, 7.5, stroke);
        if on > 0.0 {
            ui.painter().circle_filled(center, 4.0 * on, visuals::ACCENT);
        }
    } else {
        let square = Rect::from_center_size(center, Vec2::splat(15.0));
        ui.painter().rect(square, Rounding::same(4.0), visuals::ACCENT.gamma_multiply(on), stroke);
        if on > 0.0 {
            let points = vec![
                square.left_center() + vec2(3.5, 0.0),
                square.center_bottom() + vec2(-1.5, -4.0),
                square.right_top() + vec2(-3.5, 4.0),
            ];
            ui.painter()
                .add(Shape::line(points, Stroke::new(1.8_f32, Color32::WHITE.gamma_multiply(on))));
        }
    }
    response.clicked()
}

impl App {
    pub fn render_option_picker(&mut self, ctx: &Context) {
        if self.options_mod.is_none() {
            return;
        }
        let palette = visuals::ctx_palette(ctx);
        let max_height = (ctx.screen_rect().height() - 260.0).max(200.0);
        let title = "Options_Select".localize();
        let mut finish: Option<bool> = None;
        chrome::dialog(ctx, "optionsDialog", Icon::Sliders, None, &title, 560.0, |ui| {
            let Some((mod_, _)) = self.options_mod.as_mut() else {
                return;
            };
            ui.label(RichText::new(mod_.meta.name.as_str()).size(13.0).color(palette.muted));
            ui.add_space(10.0);
            let mut done = true;
            egui::ScrollArea::vertical().drag_to_scroll(false)
                .id_source("optionsScroll")
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (group_index, group) in mod_.meta.options.iter().enumerate() {
                        let satisfied = !group.required()
                            || group.options().iter().any(|opt| mod_.enabled_options.contains(opt));
                        done &= satisfied;
                        egui::Frame::none()
                            .fill(palette.raised)
                            .stroke(Stroke::new(1.0_f32, palette.border))
                            .rounding(10.0)
                            .inner_margin(Margin::same(12.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(group.name())
                                            .family(egui::FontFamily::Name("Bold".into()))
                                            .color(palette.text),
                                    );
                                    if !satisfied {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            ui.label(
                                                RichText::new("Options_Required".localize())
                                                    .size(12.0)
                                                    .color(ui.visuals().warn_fg_color),
                                            );
                                        });
                                    }
                                });
                                if !group.description().is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(group.description()).size(12.0).color(palette.muted),
                                        )
                                        .wrap(),
                                    );
                                }
                                ui.add_space(6.0);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                let base = Id::new(("optionGroup", group_index));
                                match group {
                                    OptionGroup::Exclusive(group) => {
                                        if !group.required {
                                            let none = ModOption {
                                                name: "Options_None".localize().as_ref().into(),
                                                description: Default::default(),
                                                path: Default::default(),
                                                requires: Default::default(),
                                            };
                                            let nothing = !group.options.iter().any(|o| mod_.enabled_options.contains(o));
                                            if option_row(ui, base.with("none"), &none, nothing, true) {
                                                mod_.enabled_options.retain(|o| !group.options.contains(o));
                                            }
                                        }
                                        for (i, opt) in group.options.iter().enumerate() {
                                            let checked = mod_.enabled_options.contains(opt);
                                            if option_row(ui, base.with(i), opt, checked, true) && !checked {
                                                mod_.enabled_options.retain(|o| !group.options.contains(o));
                                                mod_.enabled_options.push(opt.clone());
                                            }
                                        }
                                    }
                                    OptionGroup::Multiple(group) => {
                                        for (i, opt) in group.options.iter().enumerate() {
                                            let checked = mod_.enabled_options.contains(opt);
                                            if option_row(ui, base.with(i), opt, checked, false) {
                                                if checked {
                                                    mod_.enabled_options.retain(|o| o != opt);
                                                } else {
                                                    mod_.enabled_options.push(opt.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        ui.add_space(8.0);
                    }
                });
            chrome::dialog_buttons(ui, |ui| {
                let ok = ui
                    .add_enabled_ui(done, |ui| chrome::fancy_button(ui, Icon::Check, &"Generic_OK".localize(), 90.0, 30.0))
                    .inner;
                if ok.clicked() {
                    finish = Some(true);
                }
                if ui
                    .add(egui::Button::new("Generic_Cancel".localize()).min_size(vec2(80.0, 30.0)))
                    .clicked()
                {
                    finish = Some(false);
                }
            });
        });
        match finish {
            Some(true) => {
                if let Some((mod_, update)) = self.options_mod.take() {
                    if update {
                        self.do_update(Message::UpdateOptions(mod_));
                    } else {
                        self.do_update(Message::InstallMod(mod_));
                    }
                }
            }
            Some(false) => self.options_mod = None,
            None => (),
        }
    }
}
