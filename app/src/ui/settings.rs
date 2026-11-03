use std::{ops::Deref, path::PathBuf};

use swmm_core::{savage_wind, settings::Settings};
use uk_content::constants::Language;
use swmm_i18n::{string_ext::LocString, LocLang};
use swmm_widgets::{
    egui::{
        self, pos2, vec2, Align, Color32, Id, Layout, Margin, Rect, RichText, Rounding, Sense, Stroke, TextStyle,
        TextWrapMode, Ui, Vec2, WidgetText,
    },
    icons::{icon_text_job, Icon, IconButtonExt},
    visuals::{self, Theme},
};

use super::{chrome, App, Message};

const MAX_WIDTH: f32 = 760.0;
const ROW_PADDING: f32 = 16.0;

struct Group<'a> {
    ui: &'a mut Ui,
    rows: usize,
}

impl Group<'_> {
    fn separator(&mut self) {
        if self.rows > 0 {
            let palette = visuals::palette(self.ui);
            let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), 1.0), Sense::hover());
            self.ui.painter().hline(
                (rect.left() + ROW_PADDING)..=(rect.right() - ROW_PADDING),
                rect.center().y,
                Stroke::new(1.0_f32, palette.border),
            );
        }
        self.rows += 1;
    }

    fn row(&mut self, label: &str, hint: &str, control_width: f32, add: impl FnOnce(&mut Ui)) {
        self.separator();
        let ui = &mut *self.ui;
        let palette = visuals::palette(ui);
        let width = ui.available_width();
        let inner_width = width - ROW_PADDING * 2.0;
        let control_width = control_width.min(inner_width * 0.6);
        let text_width = (inner_width - control_width - 16.0).max(60.0);
        let title = WidgetText::from(RichText::new(label).color(palette.text)).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            text_width,
            TextStyle::Body,
        );
        let hint = (!hint.is_empty()).then(|| {
            WidgetText::from(RichText::new(hint).size(12.0).color(palette.muted)).into_galley(
                ui,
                Some(TextWrapMode::Wrap),
                text_width,
                TextStyle::Body,
            )
        });
        let text_height = title.size().y + hint.as_ref().map(|h| h.size().y + 2.0).unwrap_or(0.0);
        let height = (text_height + 22.0).max(50.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let mut y = rect.center().y - text_height / 2.0;
        let x = rect.left() + ROW_PADDING;
        let title_height = title.size().y;
        ui.painter().galley(pos2(x, y), title, palette.text);
        y += title_height + 2.0;
        if let Some(hint) = hint {
            ui.painter().galley(pos2(x, y), hint, palette.muted);
        }
        let control = Rect::from_min_max(
            pos2(rect.right() - ROW_PADDING - control_width, rect.top()),
            pos2(rect.right() - ROW_PADDING, rect.bottom()),
        );
        ui.allocate_ui_at_rect(control, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), add);
        });
    }

    fn block(&mut self, add: impl FnOnce(&mut Ui)) {
        self.separator();
        egui::Frame::none()
            .inner_margin(Margin::symmetric(ROW_PADDING, 12.0))
            .show(self.ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
    }
}

fn group(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Group)) {
    let palette = visuals::palette(ui);
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(13.0).strong().color(palette.muted));
    ui.add_space(6.0);
    egui::Frame::none()
        .fill(palette.raised)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .rounding(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut group = Group { ui, rows: 0 };
            add(&mut group);
        });
    ui.add_space(20.0);
}

fn path_field(ui: &mut Ui, value: &mut Option<PathBuf>, folder: bool, hint: &str) {
    let mut text = value.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 40.0).max(80.0);
        if ui
            .add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(width).margin(vec2(8.0, 5.0)))
            .changed()
        {
            *value = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
        }
        if ui.icon_button(Icon::FolderOpen).on_hover_text("Settings_Browse".localize()).clicked() {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = value.as_ref().and_then(|p| if folder { Some(p.as_path()) } else { p.parent() }) {
                dialog = dialog.set_directory(dir);
            }
            let picked = if folder { dialog.pick_folder() } else { dialog.pick_file() };
            if let Some(path) = picked {
                *value = Some(path);
            }
        }
    });
}

fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(40.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let palette = visuals::palette(ui);
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.18);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
    let track = chrome::mix(palette.base, visuals::ACCENT, t);
    let outline = chrome::mix(palette.muted, visuals::ACCENT, t);
    ui.painter().rect(rect, Rounding::same(10.0), track, Stroke::new(1.0_f32, outline));
    let knob = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), chrome::ease(t));
    let radius = 5.0 + hover + if response.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    ui.painter().circle_filled(
        pos2(knob, rect.center().y),
        radius,
        chrome::mix(palette.muted, Color32::WHITE, t),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn percent_slider(ui: &mut Ui, id: Id, value: &mut u32, min: u32, max: u32, step: u32, default: u32) -> egui::Response {
    let palette = visuals::palette(ui);
    let pill_width = 58.0;
    let height = 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut response = ui.interact(rect, id, Sense::click_and_drag());
    let response_id = id;
    let track = Rect::from_min_max(
        pos2(rect.left() + 9.0, rect.center().y - 3.0),
        pos2(rect.right() - pill_width - 18.0, rect.center().y + 3.0),
    );
    let (lo, hi) = (min as f32, max as f32);
    let old = *value;
    if response.double_clicked() {
        *value = default;
    } else if response.dragged() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let t = ((pointer.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
            let raw = lo + t * (hi - lo);
            *value = ((raw / step as f32).round() as u32 * step).clamp(min, max);
        }
    }
    if response.hovered() {
        let (left, right) = ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight)));
        if left {
            *value = value.saturating_sub(step).max(min);
        }
        if right {
            *value = (*value + step).min(max);
        }
    }
    if *value != old {
        response.mark_changed();
    }
    let shown = ui.ctx().animate_value_with_time(response_id, *value as f32, 0.08);
    let t = ((shown - lo) / (hi - lo)).clamp(0.0, 1.0);
    let knob_x = track.left() + t * track.width();
    let hover = ui.ctx().animate_bool_with_time(response_id.with("hover"), response.hovered() || response.dragged(), 0.12);
    let rounding = Rounding::same(3.0);
    ui.painter().rect_filled(track, rounding, palette.base);
    ui.painter().rect_stroke(track, rounding, Stroke::new(1.0_f32, palette.border));
    let filled = Rect::from_min_max(track.min, pos2(knob_x, track.max.y));
    ui.painter().rect_filled(filled, rounding, visuals::ACCENT);
    let default_x = track.left() + ((default as f32 - lo) / (hi - lo)).clamp(0.0, 1.0) * track.width();
    ui.painter().line_segment(
        [pos2(default_x, track.top() - 5.0), pos2(default_x, track.bottom() + 5.0)],
        Stroke::new(1.5_f32, chrome::mix(palette.border, palette.muted, 0.5)),
    );
    let center = pos2(knob_x, track.center().y);
    let radius = 8.0 + hover * 1.5;
    ui.painter().circle_filled(center + vec2(0.0, 1.0), radius + 1.0, Color32::from_black_alpha(60));
    ui.painter().circle(center, radius, Color32::WHITE, Stroke::new(2.0_f32, visuals::ACCENT));
    let pill = Rect::from_min_size(pos2(rect.right() - pill_width, rect.center().y - 13.0), vec2(pill_width, 26.0));
    let changed_glow = chrome::mix(palette.border, visuals::ACCENT, hover * 0.6);
    ui.painter().rect(pill, Rounding::same(13.0), palette.base, Stroke::new(1.0_f32, changed_glow));
    ui.painter().text(
        pill.center(),
        egui::Align2::CENTER_CENTER,
        format!("{}%", *value),
        egui::FontId::proportional(13.0),
        if *value == default { palette.muted } else { palette.text },
    );
    response.on_hover_cursor(egui::CursorIcon::Grab)
}

fn segmented<T: PartialEq + Copy>(ui: &mut Ui, id: Id, options: &[(T, Icon, String)], current: T) -> Option<T> {
    let palette = visuals::palette(ui);
    let segment = 92.0;
    let height = 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(segment * options.len() as f32 + 4.0, height), Sense::hover());
    ui.painter().rect(rect, Rounding::same(8.0), palette.base, Stroke::new(1.0_f32, palette.border));
    let index = options.iter().position(|(v, ..)| *v == current).unwrap_or(0) as f32;
    let x = ui.ctx().animate_value_with_time(id, index, 0.22);
    let highlight = Rect::from_min_size(rect.min + vec2(2.0 + x * segment, 2.0), vec2(segment, height - 4.0));
    ui.painter().rect_filled(highlight, Rounding::same(6.0), palette.hover);
    let mut picked = None;
    for (i, (value, icon, label)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + vec2(2.0 + i as f32 * segment, 2.0), vec2(segment, height - 4.0));
        let response = ui
            .interact(cell, id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let closeness = 1.0 - (x - i as f32).abs().min(1.0);
        let color = chrome::mix(palette.muted, palette.text, closeness);
        let job = icon_text_job(ui, *icon, label, Some(color));
        let galley = ui.fonts(|f| f.layout_job(job));
        ui.painter().galley(cell.center() - galley.size() / 2.0, galley, color);
        if response.clicked() && *value != current {
            picked = Some(*value);
        }
    }
    picked
}

impl App {
    pub fn render_settings(&mut self, ui: &mut Ui) {
        let mut theme_change: Option<Theme> = None;
        let mut lang_change: Option<LocLang> = None;
        let changed = self.temp_settings.ne(self.core.settings().deref());
        let page = ui.available_rect_before_wrap();
        egui::ScrollArea::vertical().drag_to_scroll(false)
            .id_source("settingsScroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width().min(MAX_WIDTH);
                let left = ((ui.available_width() - width) / 2.0).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(left);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        self.settings_game(ui);
                        self.settings_general(ui, &mut theme_change, &mut lang_change);
                        self.settings_mods(ui);
                        self.settings_misc(ui);
                        self.settings_storage(ui);
                        ui.add_space(56.0);
                    });
                });
            });
        self.settings_save_bar(ui, page, changed);
        if let Some(theme) = theme_change {
            self.do_update(Message::SetTheme(theme));
        }
        if let Some(lang) = lang_change {
            self.do_update(Message::SetLanguage(lang));
        }
    }

    fn settings_save_bar(&mut self, ui: &mut Ui, page: Rect, changed: bool) {
        let t = ui.ctx().animate_bool_with_time(Id::new("settingsSaveBar"), changed, 0.25);
        if t <= 0.0 {
            return;
        }
        let palette = visuals::palette(ui);
        let width = page.width().min(520.0);
        let offset = (1.0 - chrome::ease(t)) * 70.0;
        let bar = Rect::from_center_size(pos2(page.center().x, page.bottom() - 26.0 + offset), vec2(width, 48.0));
        let shadow = egui::epaint::Shadow {
            offset: vec2(0.0, 6.0),
            blur: 22.0,
            spread: 0.0,
            color: Color32::from_black_alpha((120.0 * t) as u8),
        };
        ui.painter().add(shadow.as_shape(bar, Rounding::same(12.0)));
        ui.painter().rect(bar, Rounding::same(12.0), palette.raised, Stroke::new(1.0_f32, palette.border));
        ui.allocate_ui_at_rect(bar.shrink2(vec2(16.0, 0.0)), |ui| {
            ui.set_opacity(t);
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("Settings_Unsaved".localize()).color(palette.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_enabled_ui(changed, |ui| {
                        if chrome::fancy_button(ui, Icon::Check, &"Generic_Save".localize(), 88.0, 30.0).clicked() {
                            self.do_update(Message::SaveSettings);
                        }
                        if ui
                            .add(egui::Button::new("Generic_Reset".localize()).min_size(vec2(76.0, 30.0)))
                            .clicked()
                        {
                            self.do_update(Message::SetLanguage(self.core.settings().lang));
                            self.do_update(Message::ResetSettings);
                        }
                    });
                });
            });
        });
    }

    fn settings_game(&mut self, ui: &mut Ui) {
        let status_ok = self.game_status.ok;
        let status = self.game_status.summary.clone();
        let detail = self.game_status.detail.clone();
        let sender = self.channel.0.clone();
        let config = &mut self.temp_settings.savage_wind;
        group(ui, &"SW_Settings_Section".localize(), |group| {
            let location = detail.lines().last().unwrap_or_default().to_string();
            group.row(&status, &location, 190.0, |ui| {
                let logo = super::frame::logo_image(ui, 14.0);
                if ui
                    .add(egui::Button::image_and_text(logo, "SW_Menu_AutoDetect".localize()))
                    .on_hover_text("SW_Settings_AutoDetect_Desc".localize())
                    .clicked()
                {
                    sender.send(Message::SwAutoDetect).unwrap_or(());
                }
                let color = if status_ok { visuals::GREEN } else { ui.visuals().warn_fg_color };
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, color);
            });
            group.block(|ui| {
                let palette = visuals::palette(ui);
                ui.label(RichText::new("SW_Settings_Exe".localize()).color(palette.text));
                ui.add_space(6.0);
                path_field(ui, &mut config.executable, false, "savagewind.exe");
            });
        });
    }

    fn settings_mods(&mut self, ui: &mut Ui) {
        let settings = &mut self.temp_settings;
        group(ui, &"Settings_Mods".localize(), |group| {
            group.row(
                &"SW_Settings_Browse".localize(),
                &"SW_Settings_Browse_Desc".localize(),
                48.0,
                |ui| {
                    toggle(ui, &mut settings.savage_wind.browse_gamebanana);
                },
            );
            if let Some(config) = settings.switch_config.as_mut() {
                group.row(
                    &"Settings_GameLanguage".localize(),
                    &"Settings_GameLanguage_Desc".localize(),
                    150.0,
                    |ui| {
                        egui::ComboBox::from_id_source("gameLanguage")
                            .width(140.0)
                            .selected_text(config.language.to_str())
                            .show_ui(ui, |ui| {
                                for lang in Language::iter() {
                                    ui.selectable_value(&mut config.language, *lang, lang.to_str());
                                }
                            });
                    },
                );
            }
        });
    }

    fn settings_misc(&mut self, ui: &mut Ui) {
        let sw = &mut self.temp_settings.savage_wind;
        group(ui, &"SW_Settings_Misc".localize(), |group| {
            group.block(|ui| {
                let palette = visuals::palette(ui);
                ui.label(RichText::new("SW_Settings_Camera".localize()).color(palette.text));
                ui.label(RichText::new("SW_Settings_Camera_Desc".localize()).size(12.0).color(palette.muted));
                ui.add_space(10.0);
                percent_slider(ui, Id::new("cameraSensitivity"), &mut sw.camera_sensitivity, 25, 300, 5, 100);
            });
            group.row(&"SW_Settings_Cemuhook".localize(), &"SW_Settings_Cemuhook_Desc".localize(), 48.0, |ui| {
                toggle(ui, &mut sw.cemuhook);
            });
            if sw.cemuhook {
                group.row(
                    &"SW_Settings_CemuhookServer".localize(),
                    &"SW_Settings_CemuhookServer_Desc".localize(),
                    170.0,
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut sw.cemuhook_server)
                                .hint_text("127.0.0.1:26760")
                                .desired_width(160.0)
                                .margin(vec2(8.0, 5.0)),
                        );
                    },
                );
                group.row(
                    &"SW_Settings_CemuhookPrefer".localize(),
                    &"SW_Settings_CemuhookPrefer_Desc".localize(),
                    48.0,
                    |ui| {
                        toggle(ui, &mut sw.cemuhook_prefer);
                    },
                );
            }
        });
    }

    fn settings_general(&mut self, ui: &mut Ui, theme_change: &mut Option<Theme>, lang_change: &mut Option<LocLang>) {
        let current_theme = self.theme;
        let settings = &mut self.temp_settings;
        group(ui, &"Settings_Appearance".localize(), |group| {
            let options = [
                (Theme::Dark, Icon::Moon, "Settings_Theme_Dark".localize().into_owned()),
                (Theme::Light, Icon::Sun, "Settings_Theme_Light".localize().into_owned()),
            ];
            group.row(&"Settings_Theme".localize(), "", 190.0, |ui| {
                if let Some(theme) = segmented(ui, Id::new("themeSwitch"), &options, current_theme) {
                    *theme_change = Some(theme);
                }
            });
            group.row(&"Settings_Language".localize(), "", 150.0, |ui| {
                egui::ComboBox::from_id_source("appLanguage")
                    .width(140.0)
                    .selected_text(settings.lang.to_str())
                    .show_ui(ui, |ui| {
                        for lang in LocLang::iter() {
                            if ui.selectable_value(&mut settings.lang, *lang, lang.to_str()).changed() {
                                *lang_change = Some(*lang);
                            }
                        }
                    });
            });
        });
    }

    fn settings_storage(&mut self, ui: &mut Ui) {
        let settings: &mut Settings = &mut self.temp_settings;
        group(ui, &"Settings_Storage".localize(), |group| {
            group.block(|ui| {
                let palette = visuals::palette(ui);
                ui.label(RichText::new("Settings_StorageFolder".localize()).color(palette.text));
                ui.label(RichText::new("Settings_Storage_Desc".localize()).size(12.0).color(palette.muted));
                ui.add_space(6.0);
                let mut path = Some(settings.storage_dir.clone());
                path_field(ui, &mut path, true, "");
                if let Some(path) = path {
                    settings.storage_dir = path;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.icon_text_button("Menu_Tools_StorageFolder".localize(), Icon::FolderOpen).clicked() {
                        std::fs::create_dir_all(&settings.storage_dir).unwrap_or(());
                        open::that(&settings.storage_dir).unwrap_or(());
                    }
                    if let Some(overlay) = savage_wind::overlay_dir(settings) {
                        if ui
                            .icon_text_button("SW_Menu_OverlayFolder".localize(), Icon::Layers)
                            .on_hover_text(overlay.display().to_string())
                            .clicked()
                        {
                            std::fs::create_dir_all(&overlay).unwrap_or(());
                            open::that(&overlay).unwrap_or(());
                        }
                    }
                });
            });
        });
    }
}
