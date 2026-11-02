#![allow(deprecated)]

use std::{borrow::Cow, sync::LazyLock};

use strfmt::Format;
use swmm_core::savage_wind;
use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{load::SizedTexture, pos2, vec2, Color32, Rect, Rounding, Sense, Stroke},
    egui_extras::RetainedImage,
    icons::icon_family,
    visuals,
};

use super::{chrome, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Page {
    #[default]
    Home,
    Browse,
    Profiles,
    Settings,
}

impl Page {
    pub const MAIN: [Page; 3] = [Page::Home, Page::Browse, Page::Profiles];

    pub fn icon(self) -> Icon {
        match self {
            Page::Home => Icon::Home,
            Page::Browse => Icon::Globe,
            Page::Profiles => Icon::People,
            Page::Settings => Icon::Gear,
        }
    }

    pub fn title(self) -> Cow<'static, str> {
        match self {
            Page::Home => "SW_Page_Home",
            Page::Browse => "SW_Page_Browse",
            Page::Profiles => "SW_Page_Profiles",
            Page::Settings => "SW_Page_Settings",
        }
        .localize()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameStatus {
    pub ok: bool,
    pub can_launch: bool,
    pub summary: String,
    pub detail: String,
}

impl GameStatus {
    pub fn check(core: &Manager) -> Self {
        let settings = core.settings();
        let exe = settings
            .savage_wind
            .executable
            .clone()
            .filter(|p| !p.as_os_str().is_empty());
        let exe_ok = exe.as_ref().map(|e| e.exists()).unwrap_or(false);
        let exe_text = match exe.as_ref() {
            Some(e) if exe_ok => e.display().to_string(),
            Some(e) => format!("{} ({})", e.display(), "SW_Status_NotFound".localize()),
            None => "SW_Status_ExeNotSet".localize().into_owned(),
        };
        match savage_wind::current_game_files(&settings) {
            Ok(game) => {
                let supported = game.is_supported_version();
                Self {
                    ok: exe_ok && supported,
                    can_launch: exe_ok,
                    summary: if supported {
                        format!("Savage Wind · BOTW {}", game.describe())
                    } else {
                        format!("BOTW {} ({})", game.describe(), "SW_Status_Unsupported".localize())
                    },
                    detail: format!(
                        "{}: {}\n{}: {}",
                        "SW_Settings_Exe".localize(),
                        exe_text,
                        "SW_Status_GameFiles".localize().trim_end_matches([':', '\u{ff1a}', ' ']),
                        game.romfs.display(),
                    ),
                }
            }
            Err(e) => Self {
                ok: false,
                can_launch: exe_ok,
                summary: "SW_Status_NotSetUp".localize().into_owned(),
                detail: format!("{}: {}\n{}", "SW_Settings_Exe".localize(), exe_text, chrome::clean_error(&e)),
            },
        }
    }
}

pub fn auto_configure_on_start(core: &Manager) {
    savage_wind::clear_legacy_environment();
    let result = {
        let mut settings = core.settings_mut();
        match savage_wind::auto_configure(&mut settings) {
            Ok(done) if done.changed() => {
                log::info!("Savage Wind set up automatically: {:?}", done);
                settings.save().map(|_| true)
            }
            Ok(_) => Ok(false),
            Err(e) => {
                log::warn!("Savage Wind isn't fully set up yet: {e:?}");
                Ok(false)
            }
        }
    };
    match result {
        Ok(true) => {
            if let Err(e) = core.reload() {
                log::error!("Failed to reload after setting up Savage Wind: {e:?}");
            }
        }
        Ok(false) => (),
        Err(e) => log::error!("Failed to save the Savage Wind settings: {e:?}"),
    }
    super::jobs::sync_loader(&core.settings());
}

static LOGO: LazyLock<RetainedImage> = LazyLock::new(|| {
    RetainedImage::from_image_bytes("swmm-icon", include_bytes!("../../../assets/icon.png"))
        .expect("Bundled icon is a valid PNG")
});

static BANNER: LazyLock<RetainedImage> = LazyLock::new(|| {
    RetainedImage::from_image_bytes("swmm-banner", include_bytes!("../../../assets/swmm.png"))
        .expect("Bundled banner is a valid PNG")
});

pub fn logo(ui: &mut Ui, size: f32) -> egui::Response {
    let tint = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    ui.add(egui::Image::new(SizedTexture::new(LOGO.texture_id(ui.ctx()), Vec2::splat(size))).tint(tint))
}

pub fn logo_image(ui: &Ui, size: f32) -> egui::Image<'static> {
    let tint = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    egui::Image::new(SizedTexture::new(LOGO.texture_id(ui.ctx()), Vec2::splat(size))).tint(tint)
}

pub fn logo_texture(ctx: &egui::Context) -> egui::TextureId {
    LOGO.texture_id(ctx)
}

pub fn banner(ui: &mut Ui, width: f32, max_height: f32) -> egui::Response {
    let [w, h] = BANNER.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let scale = (width / w).min(max_height / h);
    ui.add(
        egui::Image::new(SizedTexture::new(BANNER.texture_id(ui.ctx()), Vec2::new(w * scale, h * scale)))
            .rounding(10.0),
    )
}

pub fn action_button(ui: &mut Ui, icon: Icon, tip: &'static str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(icon.rich(14.0)).min_size(Vec2::new(32.0, 28.0)))
        .on_hover_text(tip.localize())
        .clicked()
}

pub fn accent_button(ui: &mut Ui, icon: Icon, text: Cow<'static, str>) -> egui::Response {
    chrome::fancy_button(ui, icon, &text, 0.0, 30.0)
}

pub fn chip(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.15))
        .rounding(10.0)
        .inner_margin(Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let palette = visuals::palette(ui);
    egui::Frame::none()
        .fill(palette.raised)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .rounding(8.0)
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn page_header(ui: &mut Ui, page: Page) {
    let palette = visuals::palette(ui);
    ui.label(
        RichText::new(page.title())
            .size(24.0)
            .family(egui::FontFamily::Name("Bold".into()))
            .color(palette.text),
    );
    ui.add_space(14.0);
}

impl App {
    pub(crate) fn current_profile(&self) -> String {
        self.core
            .settings()
            .platform_config()
            .map(|c| c.profile.to_string())
            .unwrap_or_else(|| "Default".to_owned())
    }

    fn status_text(&self) -> (Icon, String) {
        if self.busy.get() {
            let text = crate::logger::LOGGER
                .get_progress()
                .unwrap_or_else(|| "Busy_Processing".localize().into_owned());
            return (Icon::Spinner, text);
        }
        let dirty = self.dirty();
        let pending = dirty.content_files.len() + dirty.aoc_files.len();
        drop(dirty);
        if pending > 0 {
            return (Icon::ListCheck, format!("{} {}", pending, "SW_Status_Pending".localize()));
        }
        if self.core.deploy_manager().pending() {
            return (Icon::Publish, "SW_Status_DeployPending".localize().into_owned());
        }
        if !self.game_status.ok {
            return (Icon::Warning, self.game_status.summary.clone());
        }
        (Icon::Check, "SW_Status_Ready".localize().into_owned())
    }

    pub fn render_status_bar(&self, ctx: &egui::Context) {
        let palette = self.theme.palette();
        egui::TopBottomPanel::bottom("statusBar")
            .exact_height(28.0)
            .show_separator_line(false)
            .frame(
                Frame::none()
                    .fill(palette.base)
                    .rounding(chrome::bottom_rounding(ctx))
                    .inner_margin(Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                let busy = ui.ctx().animate_bool_with_time(Id::new("statusBusy"), self.busy.get(), 0.2);
                if busy > 0.0 {
                    let full = ui.max_rect().expand2(vec2(12.0, 0.0));
                    let track = Rect::from_min_size(full.left_top(), vec2(full.width(), 2.0));
                    let time = ui.input(|i| i.time);
                    chrome::indeterminate_bar(ui.painter(), track, time, visuals::ACCENT.gamma_multiply(busy));
                    ui.ctx().request_repaint();
                }
                ui.horizontal_centered(|ui| {
                    let (icon, text) = self.status_text();
                    if self.busy.get() {
                        chrome::progress_ring(ui, 13.0, palette.accent_text);
                    } else {
                        let color = match icon {
                            Icon::Check => visuals::GREEN,
                            Icon::Warning => ui.visuals().warn_fg_color,
                            _ => palette.accent_text,
                        };
                        ui.label(icon.rich(11.0).color(color));
                    }
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(palette.muted)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(12.0)
                                .color(palette.muted),
                        );
                        ui.label(RichText::new("·").color(palette.muted));
                        let enabled = self.mods.iter().filter(|m| m.enabled).count();
                        ui.label(
                            RichText::new(format!("{} / {} {}", enabled, self.mods.len(), "SW_ModsEnabled".localize()))
                                .size(12.0)
                                .color(palette.muted),
                        );
                        ui.label(Icon::Puzzle.rich(10.0).color(palette.muted));
                        ui.label(RichText::new("·").color(palette.muted));
                        ui.label(RichText::new(self.current_profile()).size(12.0).color(palette.muted));
                        ui.label(Icon::User.rich(10.0).color(palette.muted));
                    });
                });
            });
    }

    fn nav_button(&self, ui: &mut Ui, page: Page) -> egui::Response {
        let selected = self.page == page;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
        let palette = self.theme.palette();
        let ctx = ui.ctx().clone();
        let hover = ctx.animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
        let active = ctx.animate_bool_with_time(response.id.with("active"), selected, 0.2);
        let fill = (hover * 0.6).max(active);
        if fill > 0.0 {
            ui.painter().rect_filled(rect, Rounding::same(7.0), palette.hover.gamma_multiply(fill));
        }
        let color = chrome::mix(chrome::mix(palette.muted, palette.text, hover), palette.text, active);
        ui.painter().text(
            pos2(rect.left() + 20.0, rect.center().y),
            Align2::CENTER_CENTER,
            page.icon().glyph().to_string(),
            egui::FontId::new(14.0, icon_family()),
            chrome::mix(color, palette.accent_text, active),
        );
        ui.painter().text(
            pos2(rect.left() + 40.0, rect.center().y),
            Align2::LEFT_CENTER,
            page.title(),
            egui::FontId::proportional(14.0),
            color,
        );
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    pub fn render_nav(&mut self, ctx: &egui::Context) {
        let palette = self.theme.palette();
        egui::SidePanel::left("navRail")
            .resizable(false)
            .exact_width(188.0)
            .show_separator_line(false)
            .frame(Frame::none().fill(palette.base).inner_margin(Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 10.0,
            }))
            .show(ctx, |ui| {
                ui.add_enabled_ui(!self.modal_open(), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut selected_rect = None;
                    let browse_on = self.core.settings().savage_wind.browse_gamebanana;
                    if !browse_on && self.page == Page::Browse {
                        self.page = Page::Home;
                    }
                    for page in Page::MAIN {
                        if page == Page::Browse && !browse_on {
                            continue;
                        }
                        let response = self.nav_button(ui, page);
                        if self.page == page {
                            selected_rect = Some(response.rect);
                        }
                        if response.clicked() {
                            self.page = page;
                        }
                    }
                    let footer = ui
                        .with_layout(Layout::bottom_up(Align::Min), |ui| {
                            let response = self.nav_button(ui, Page::Settings);
                            if response.clicked() {
                                self.page = Page::Settings;
                            }
                            (self.page == Page::Settings).then_some(response.rect)
                        })
                        .inner;
                    if let Some(rect) = selected_rect.or(footer) {
                        let y = ui.ctx().animate_value_with_time(Id::new("navPill"), rect.center().y, 0.28);
                        let stretch = (y - rect.center().y).abs().min(36.0) * 0.45;
                        let pill = Rect::from_center_size(pos2(rect.left() + 1.5, y), vec2(3.0, 14.0 + stretch));
                        ui.painter().rect_filled(pill, Rounding::same(1.5), palette.accent_text);
                    }
                });
            });
    }

    pub fn render_page(&mut self, ctx: &egui::Context) {
        let page = self.page;
        let palette = self.theme.palette();
        let time = chrome::now(ctx);
        let key = Id::new("pageShown");
        let (shown, since) = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || (page, time)));
        let since = if shown != page {
            ctx.data_mut(|d| d.insert_temp(key, (page, time)));
            time
        } else {
            since
        };
        let t = ((time - since) as f32 / 0.32).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }
        let progress = chrome::ease(t);
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(palette.panel)
                    .inner_margin(Margin::same(24.0)),
            )
            .show(ctx, |ui| {
                ui.set_opacity(progress);
                ui.add_space((1.0 - progress) * 16.0);
                ui.add_enabled_ui(!self.modal_open(), |ui| match page {
                    Page::Home => self.render_home(ui),
                    Page::Browse => {
                        page_header(ui, page);
                        self.render_browser(ui);
                    }
                    Page::Profiles => {
                        page_header(ui, page);
                        self.render_profiles_page(ui);
                    }
                    Page::Settings => {
                        page_header(ui, page);
                        self.render_settings(ui);
                    }
                });
            });
    }

    fn render_home(&mut self, ui: &mut Ui) {
        if self.mods.is_empty() {
            self.render_welcome(ui);
            return;
        }
        egui::SidePanel::right("homeDetails")
            .resizable(false)
            .exact_width(320.0)
            .show_separator_line(false)
            .frame(Frame::none().inner_margin(Margin {
                left: 22.0,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.render_home_info(ui));
        egui::CentralPanel::default()
            .frame(Frame::none())
            .show_inside(ui, |ui| self.render_home_list(ui));
    }

    fn render_welcome(&mut self, ui: &mut Ui) {
        let configured = self.game_status.ok;
        let palette = visuals::palette(ui);
        let time = ui.input(|i| i.time);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        let avail = ui.available_rect_before_wrap();
        let width = avail.width().min(720.0).max(240.0);
        let area = Rect::from_center_size(avail.center(), Vec2::new(width, avail.height().min(660.0)));
        ui.allocate_ui_at_rect(area, |ui| {
            let float = (time * 1.3).sin() as f32 * 4.0;
            ui.add_space(4.0 + float);
            ui.vertical_centered(|ui| {
                banner(ui, width, 240.0);
            });
            ui.add_space(14.0 - float);
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.18);
            let stroke = chrome::mix(palette.border, palette.accent_text, hover);
            ui.painter().rect_filled(rect, Rounding::same(10.0), visuals::ACCENT.gamma_multiply(0.06 * hover));
            let r = rect.shrink(1.0);
            let points = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            let offset = if hover > 0.0 { (time * 24.0) as f32 % 11.0 } else { 0.0 };
            ui.painter().extend(egui::Shape::dashed_line_with_offset(
                &points,
                Stroke::new(1.5_f32, stroke),
                &[6.0],
                &[5.0],
                offset,
            ));
            let lift = hover * 4.0;
            ui.painter().text(
                rect.center() - Vec2::new(0.0, 18.0 + lift),
                Align2::CENTER_CENTER,
                Icon::FileImport.glyph().to_string(),
                egui::FontId::new(34.0, icon_family()),
                stroke,
            );
            ui.painter().text(
                rect.center() + Vec2::new(0.0, 28.0),
                Align2::CENTER_CENTER,
                "SW_Welcome_Drop".localize(),
                egui::FontId::proportional(14.0),
                palette.muted,
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Menu_File_Open".localize())
                .clicked()
            {
                self.do_update(Message::SelectFile);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if self.core.settings().savage_wind.browse_gamebanana
                    && accent_button(ui, Icon::Globe, "SW_Welcome_Browse".localize()).clicked()
                {
                    self.page = Page::Browse;
                }
                if ui.icon_text_button("SW_Welcome_OpenSettings".localize(), Icon::Gear).clicked() {
                    self.page = Page::Settings;
                }
            });
            if !configured {
                ui.add_space(12.0);
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(Icon::Warning.rich(16.0).color(ui.visuals().warn_fg_color));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("SW_Welcome_Setup".localize()).strong());
                            ui.label(
                                RichText::new(self.game_status.detail.as_str())
                                    .size(12.0)
                                    .color(palette.muted),
                            );
                        });
                    });
                });
            }
        });
    }

    fn render_home_list(&mut self, ui: &mut Ui) {
        let palette = visuals::palette(ui);
        let width = ui.available_width();
        ui.horizontal(|ui| {
            let search_width = (width - 150.0 - 36.0 - 2.0 * ui.spacing().item_spacing.x).max(120.0);
            egui::Frame::none()
                .fill(palette.raised)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .rounding(8.0)
                .inner_margin(Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(search_width - 22.0);
                    ui.horizontal(|ui| {
                        ui.label(Icon::Search.rich(12.0).color(palette.muted));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.mod_list_filter)
                                .frame(false)
                                .hint_text("SW_SearchMods".localize())
                                .desired_width(f32::INFINITY),
                        );
                    });
                });
            let current = self.current_profile();
            egui::ComboBox::from_id_source("homeProfiles")
                .width(150.0)
                .selected_text(current.as_str())
                .show_ui(ui, |ui| {
                    for profile in self.core.settings().profiles() {
                        if ui.selectable_label(profile.as_str() == current, profile.as_str()).clicked()
                            && profile.as_str() != current
                        {
                            self.do_update(Message::ChangeProfile(profile.to_string()));
                        }
                    }
                })
                .response
                .on_hover_text("Profile_Select".localize());
            if action_button(ui, Icon::Add, "SW_Tip_Install", true) {
                self.do_update(Message::SelectFile);
            }
        });
        ui.add_space(10.0);
        egui::TopBottomPanel::bottom("homeActions")
            .show_separator_line(false)
            .frame(Frame::none().inner_margin(Margin {
                top: 10.0,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.render_home_actions(ui));
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(palette.raised)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .inner_margin(Margin::same(4.0)),
            )
            .show_inside(ui, |ui| self.render_modlist(ui));
    }

    fn render_home_actions(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let has_selection = !self.selected.is_empty();
            let first = self.mods.iter().position(|m| self.selected.contains(m));
            let last = self.mods.iter().rposition(|m| self.selected.contains(m));
            let last_index = self.mods.len().saturating_sub(1);
            let can_up = first.map(|i| i > 0).unwrap_or(false);
            let can_down = last.map(|i| i < last_index).unwrap_or(false);
            if action_button(ui, Icon::ChevronUp, "SW_Tip_MoveUp", can_up) {
                self.do_update(Message::MoveSelected(first.unwrap_or(0).saturating_sub(1)));
            }
            if action_button(ui, Icon::ChevronDown, "SW_Tip_MoveDown", can_down) {
                self.do_update(Message::MoveSelected(first.unwrap_or(0) + 1));
            }
            let options_mod = self
                .selected
                .first()
                .filter(|m| self.selected.len() == 1 && !m.meta.options.is_empty())
                .cloned();
            if action_button(ui, Icon::Sliders, "SW_Tip_Options", options_mod.is_some()) {
                if let Some(mod_) = options_mod {
                    self.do_update(Message::RequestOptions(mod_, true));
                }
            }
            if action_button(ui, Icon::Delete, "SW_Tip_Uninstall", has_selection) {
                let names = self
                    .selected
                    .iter()
                    .map(|m| m.meta.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.do_update(Message::Confirm(
                    Box::new(Message::UninstallMods(None)),
                    format!("{}\n\n{}", "SW_Confirm_Uninstall".localize(), names),
                ));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.primary_action(ui);
            });
        });
    }

    fn primary_action(&self, ui: &mut Ui) {
        if !self.dirty().is_empty() {
            let response = accent_button(ui, Icon::ListCheck, "Generic_Apply".localize()).on_hover_ui(|ui| {
                ui.label(RichText::new("Pending_Files".localize()).strong());
                mod_info::render_manifest(&self.dirty(), ui);
            });
            if response.clicked() {
                self.do_update(Message::Apply);
            }
            if ui
                .button("Generic_Cancel".localize())
                .on_hover_text("SW_Tip_Discard".localize())
                .clicked()
            {
                self.do_update(Message::ResetMods(None));
            }
        } else if self.core.deploy_manager().pending() {
            if accent_button(ui, Icon::Publish, "Tab_Deploy".localize())
                .on_hover_text("SW_Tip_Deploy".localize())
                .clicked()
            {
                self.do_update(Message::Deploy);
            }
        } else {
            self.play_button(ui);
        }
    }

    fn render_home_info(&mut self, ui: &mut Ui) {
        let palette = visuals::palette(ui);
        let Some(mod_) = self.selected.first().cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Mod_Selected_None".localize()).color(palette.muted));
            });
            return;
        };
        let fade = chrome::content_fade(ui.ctx(), egui::Id::new("homeInfoFade"), mod_.hash() as u64, 0.22);
        ui.multiply_opacity(fade);
        ui.add_space((1.0 - fade) * 8.0);
        let width = ui.available_width();
        let preview = mod_info::ModInfo(&mod_).preview();
        let (texture, [w, h]) = match preview.as_ref() {
            Some(img) => (img.texture_id(ui.ctx()), img.size()),
            None => (BANNER.texture_id(ui.ctx()), BANNER.size()),
        };
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let scale = (width / w).min(210.0 / h);
        let size = Vec2::new(w * scale, h * scale);
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, size.y), Sense::hover());
        let image_rect = Rect::from_center_size(area.center(), size);
        let fade = chrome::appear(ui.ctx(), Id::new(("homePreview", mod_.hash())), true, 0.3);
        egui::Image::new(SizedTexture::new(texture, size))
            .rounding(10.0)
            .tint(Color32::WHITE.gamma_multiply(fade))
            .paint_at(ui, image_rect);
        ui.add_space(14.0);
        ui.add(
            egui::Label::new(
                RichText::new(mod_.meta.name.as_str())
                    .size(18.0)
                    .family(egui::FontFamily::Name("Bold".into()))
                    .color(palette.text),
            )
            .wrap(),
        );
        ui.add_space(2.0);
        let mut meta = format!("v{}", mod_.meta.version);
        if !mod_.meta.author.is_empty() {
            meta.push_str(&format!("  ·  {} {}", "SW_By".localize(), mod_.meta.author));
        }
        ui.add(egui::Label::new(RichText::new(meta).size(13.0).color(palette.muted)).truncate());
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !matches!(
                mod_.meta.category,
                uk_mod::ModCategory::None | uk_mod::ModCategory::Other
            ) {
                let category = mod_.meta.category.to_loc_str().localize();
                chip(ui, category.as_ref(), palette.accent_text);
            }
            if !mod_.enabled {
                let warn = ui.visuals().warn_fg_color;
                chip(ui, "SW_Disabled".localize().as_ref(), warn);
            }
            if !mod_.meta.options.is_empty()
                && ui
                    .add(
                        egui::Button::new(icon_text_job_owned(ui, Icon::Sliders, &"SW_Options".localize()))
                            .min_size(Vec2::new(0.0, 24.0)),
                    )
                    .clicked()
            {
                self.do_update(Message::RequestOptions(mod_.clone(), true));
            }
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical().drag_to_scroll(false)
            .id_source("homeInfoScroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if mod_.meta.description.trim().is_empty() {
                    ui.label(RichText::new("SW_NoDescription".localize()).color(palette.muted));
                } else {
                    let md_cache = ui.data_mut(|d| {
                        d.get_temp_mut_or_default::<Arc<Mutex<egui_commonmark::CommonMarkCache>>>(egui::Id::new(
                            "md_cache",
                        ))
                        .clone()
                    });
                    egui_commonmark::CommonMarkViewer::new("homeModDescription").show(
                        ui,
                        &mut md_cache.lock(),
                        &mod_.meta.description,
                    );
                }
                if !mod_.enabled_options.is_empty() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Info_Options".localize()).size(13.0).strong().color(palette.muted));
                    for opt in mod_.enabled_options.iter() {
                        ui.horizontal(|ui| {
                            ui.label(Icon::Check.rich(11.0).color(palette.accent_text));
                            ui.label(opt.name.as_str());
                        });
                    }
                }
            });
    }

    fn render_profiles_page(&self, ui: &mut Ui) {
        let active = self.current_profile();
        let palette = visuals::palette(ui);
        let mut state = self.profiles_state.borrow_mut();
        let mut names: Vec<String> = state.profiles.keys().map(|k| k.to_string()).collect();
        names.sort_by_key(|n| n.to_lowercase());
        if names.is_empty() {
            names.push(active.clone());
        }
        let selected = state
            .selected
            .as_ref()
            .map(|s| s.to_string())
            .filter(|s| names.contains(s))
            .unwrap_or_else(|| active.clone());
        egui::SidePanel::left("profilesList")
            .resizable(false)
            .exact_width(250.0)
            .show_separator_line(false)
            .frame(Frame::none().inner_margin(Margin {
                right: 20.0,
                ..Default::default()
            }))
            .show_inside(ui, |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::WidgetText::from(icon_text_job_owned(ui, Icon::Add, &"Profile_New".localize())))
                            .min_size(Vec2::new(ui.available_width(), 32.0)),
                    )
                    .clicked()
                {
                    self.do_update(Message::NewProfile);
                }
                ui.add_space(10.0);
                egui::ScrollArea::vertical().drag_to_scroll(false)
                    .id_source("profilesListScroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for name in names.iter() {
                            let (enabled, total) = profile_counts(&state, name);
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
                            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
                            let picked = ui.ctx().animate_bool_with_time(response.id.with("picked"), *name == selected, 0.18);
                            let fill = (hover * 0.6).max(picked);
                            if fill > 0.0 {
                                ui.painter().rect_filled(rect, Rounding::same(8.0), palette.hover.gamma_multiply(fill));
                            }
                            let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(4.0, 0.0)));
                            painter.text(
                                rect.left_top() + Vec2::new(12.0, 8.0),
                                Align2::LEFT_TOP,
                                name,
                                egui::FontId::proportional(14.0),
                                palette.text,
                            );
                            painter.text(
                                rect.left_top() + Vec2::new(12.0, 27.0),
                                Align2::LEFT_TOP,
                                format!("{} / {} {}", enabled, total, "SW_ModsEnabled".localize()),
                                egui::FontId::proportional(12.0),
                                palette.muted,
                            );
                            if *name == active {
                                painter.circle_filled(pos2(rect.right() - 16.0, rect.center().y), 4.0, visuals::GREEN);
                            }
                            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                state.selected = Some(name.as_str().into());
                                state.rename = None;
                            }
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(Frame::none())
            .show_inside(ui, |ui| {
                let fade = chrome::content_fade(
                    ui.ctx(),
                    egui::Id::new("profileFade"),
                    egui::Id::new(&selected).value(),
                    0.22,
                );
                ui.multiply_opacity(fade);
                ui.add_space((1.0 - fade) * 8.0);
                let (enabled, total) = profile_counts(&state, &selected);
                ui.horizontal(|ui| {
                    let rename = state.rename.get_or_insert_with(|| selected.clone());
                    ui.add(
                        egui::TextEdit::singleline(&mut *rename)
                            .font(egui::FontId::new(18.0, egui::FontFamily::Name("Bold".into())))
                            .hint_text("SW_ProfileName".localize())
                            .margin(Vec2::new(10.0, 6.0))
                            .desired_width((ui.available_width() - 100.0).max(120.0)),
                    );
                    let can_rename = !rename.trim().is_empty() && *rename != selected;
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Profile_Rename".localize()).min_size(Vec2::new(90.0, 34.0)))
                        .clicked()
                    {
                        self.do_update(Message::RenameProfile(selected.clone(), rename.trim().to_string()));
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} / {} {}", enabled, total, "SW_ModsEnabled".localize()))
                        .size(13.0)
                        .color(palette.muted),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if selected == active {
                        ui.label(Icon::Check.rich(13.0).color(visuals::GREEN));
                        ui.label(RichText::new("SW_ProfileActive".localize()).color(visuals::GREEN));
                        ui.add_space(8.0);
                    } else if accent_button(ui, Icon::Check, "SW_ProfileActivate".localize()).clicked() {
                        self.do_update(Message::ChangeProfile(selected.clone()));
                    }
                    if ui
                        .add(egui::Button::new(icon_text_job_owned(ui, Icon::Copy, &"Profile_Duplicate".localize())).min_size(Vec2::new(0.0, 30.0)))
                        .clicked()
                    {
                        self.do_update(Message::DuplicateProfile(selected.clone()));
                    }
                    let delete = ui
                        .add_enabled(
                            selected != active,
                            egui::Button::new(icon_text_job_owned(ui, Icon::Delete, &"Generic_Delete".localize()))
                                .min_size(Vec2::new(0.0, 30.0)),
                        )
                        .on_disabled_hover_text("SW_Tip_DeleteProfile".localize());
                    if delete.clicked() {
                        let message = "Profile_Delete_Confirmation".localize();
                        let vars = std::collections::HashMap::from([("profile_name".to_string(), selected.clone())]);
                        self.do_update(Message::Confirm(
                            Message::DeleteProfile(selected.clone()).into(),
                            message.format(&vars).unwrap_or_else(|_| message.to_string()),
                        ));
                    }
                });
                ui.add_space(20.0);
                ui.label(RichText::new("SW_ProfileMods".localize()).size(13.0).strong().color(palette.muted));
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(palette.raised)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .inner_margin(Margin::symmetric(6.0, 6.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().drag_to_scroll(false)
                            .id_source("profileModsScroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let Some(profile) = state.profiles.get(selected.as_str()) else {
                                    return;
                                };
                                let mods = profile.mods();
                                let order = profile.load_order();
                                if order.is_empty() {
                                    ui.add_space(10.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(RichText::new("Profile_NoMods".localize()).color(palette.muted));
                                    });
                                    return;
                                }
                                for (index, hash) in order.iter().enumerate() {
                                    let Some(m) = mods.get(hash) else {
                                        continue;
                                    };
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
                                    if index % 2 == 1 {
                                        ui.painter().rect_filled(rect, Rounding::same(6.0), palette.base.gamma_multiply(0.35));
                                    }
                                    let painter = ui.painter().with_clip_rect(rect);
                                    painter.text(
                                        pos2(rect.left() + 30.0, rect.center().y),
                                        Align2::RIGHT_CENTER,
                                        (index + 1).to_string(),
                                        egui::FontId::proportional(12.0),
                                        palette.muted,
                                    );
                                    let (glyph, color) = if m.enabled {
                                        (Icon::CircleCheck, palette.accent_text)
                                    } else {
                                        (Icon::Blank, palette.muted)
                                    };
                                    if m.enabled {
                                        painter.text(
                                            pos2(rect.left() + 48.0, rect.center().y),
                                            Align2::CENTER_CENTER,
                                            glyph.glyph().to_string(),
                                            egui::FontId::new(13.0, icon_family()),
                                            color,
                                        );
                                    } else {
                                        painter.circle_stroke(
                                            pos2(rect.left() + 48.0, rect.center().y),
                                            6.0,
                                            Stroke::new(1.0_f32, palette.muted),
                                        );
                                    }
                                    painter.text(
                                        pos2(rect.left() + 64.0, rect.center().y),
                                        Align2::LEFT_CENTER,
                                        m.meta.name.as_str(),
                                        egui::FontId::proportional(14.0),
                                        if m.enabled { palette.text } else { palette.muted },
                                    );
                                }
                            });
                    });
            });
    }
}

fn profile_counts(state: &super::profiles::ProfileManagerState, name: &str) -> (usize, usize) {
    state
        .profiles
        .get(name)
        .map(|profile| {
            let mods = profile.mods();
            (mods.values().filter(|m| m.enabled).count(), mods.len())
        })
        .unwrap_or((0, 0))
}

fn icon_text_job_owned(ui: &Ui, icon: Icon, text: &str) -> egui::text::LayoutJob {
    swmm_widgets::icons::icon_text_job(ui, icon, text, None)
}
