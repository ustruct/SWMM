use swmm_core::savage_wind;
use swmm_i18n::string_ext::LocString;
use swmm_widgets::icons::icon_text_job;

use super::*;

fn menu_item(ui: &mut Ui, icon: Icon, text: &str, enabled: bool) -> bool {
    ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);
    let button = if icon == Icon::Wind {
        egui::Button::image_and_text(super::frame::logo_image(ui, 14.0), text)
    } else {
        egui::Button::new(icon_text_job(ui, icon, text, None))
    };
    let clicked = ui
        .add_enabled(enabled, button.min_size(Vec2::new(210.0, 28.0)))
        .clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

impl App {
    pub fn render_menus(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let palette = self.theme.palette();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
            let widgets = &mut ui.style_mut().visuals.widgets;
            for (state, fill) in [
                (&mut widgets.inactive, egui::Color32::TRANSPARENT),
                (&mut widgets.hovered, palette.hover),
                (&mut widgets.active, palette.border),
                (&mut widgets.open, palette.hover),
            ] {
                state.weak_bg_fill = fill;
                state.bg_stroke = egui::Stroke::NONE;
                state.fg_stroke.color = palette.text;
                state.expansion = 0.0;
            }
            ui.menu_button("Menu_File".localize(), |ui| self.file_menu(ui, frame));
            ui.menu_button("SW_Menu_Mod".localize(), |ui| self.mod_menu(ui));
            ui.menu_button("Menu_Tools".localize(), |ui| self.tool_menu(ui));
            ui.menu_button("Menu_Help".localize(), |ui| self.help_menu(ui));
        });
    }

    pub fn play_button(&self, ui: &mut Ui) {
        let enabled = self.game_status.can_launch && !self.busy.get();
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                super::chrome::fancy_button(ui, Icon::Play, &"SW_Play".localize(), 96.0, 30.0)
            })
            .inner
            .on_hover_text("SW_Play_Desc".localize())
            .on_disabled_hover_text("SW_Play_Disabled".localize());
        if response.clicked() {
            self.do_update(Message::Launch);
        }
    }

    pub fn file_menu(&self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if menu_item(ui, Icon::FileImport, &"Menu_File_Open".localize(), true) {
            self.do_update(Message::SelectFile);
        }
        if self.core.settings().savage_wind.browse_gamebanana
            && menu_item(ui, Icon::Globe, &"SW_Page_Browse".localize(), true)
        {
            self.do_update(Message::SetPage(Page::Browse));
        }
        ui.separator();
        if menu_item(ui, Icon::Folder, &"Menu_Tools_StorageFolder".localize(), true) {
            open::that(&self.core.settings().storage_dir).unwrap_or(());
        }
        if menu_item(ui, Icon::Expand, &format!("{}  (F11)", "SW_Fullscreen".localize()), true) {
            super::chrome::toggle_fullscreen(ui.ctx());
        }
        ui.separator();
        if menu_item(ui, Icon::Cancel, &"Generic_Exit".localize(), true) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub fn mod_menu(&self, ui: &mut Ui) {
        let has_selection = !self.selected.is_empty();
        if menu_item(ui, Icon::ListCheck, &"Generic_Apply".localize(), !self.dirty().is_empty()) {
            self.do_update(Message::Apply);
        }
        ui.separator();
        if menu_item(ui, Icon::Sliders, &"SW_Menu_Options".localize(), has_selection) {
            if let Some(mod_) = self.selected.first() {
                self.do_update(Message::RequestOptions(mod_.clone(), true));
            }
        }
        if menu_item(ui, Icon::FileUpload, &"SW_Menu_UpdateMod".localize(), self.selected.len() == 1) {
            self.do_update(Message::ModUpdate);
        }
        ui.separator();
        if menu_item(ui, Icon::Delete, &"SW_Menu_Uninstall".localize(), has_selection) {
            self.do_update(Message::UninstallMods(None));
        }
    }

    pub fn tool_menu(&mut self, ui: &mut Ui) {
        if menu_item(ui, Icon::Refresh, &"Menu_Tools_RefreshMerge".localize(), true) {
            self.do_update(Message::Remerge);
        }
        if menu_item(ui, Icon::Wind, &"SW_Menu_AutoDetect".localize(), true) {
            self.do_update(Message::SwAutoDetect);
        }
        ui.separator();
        let settings = self.core.settings();
        let overlay = savage_wind::overlay_dir(&settings);
        let game_dir = settings
            .savage_wind
            .executable
            .as_ref()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()));
        drop(settings);
        if menu_item(ui, Icon::FolderOpen, &"SW_Menu_OverlayFolder".localize(), overlay.is_some()) {
            if let Some(dir) = overlay {
                std::fs::create_dir_all(&dir).unwrap_or(());
                open::that(dir).unwrap_or(());
            }
        }
        if menu_item(ui, Icon::Gamepad, &"SW_Menu_GameFolder".localize(), game_dir.is_some()) {
            if let Some(dir) = game_dir {
                open::that(dir).unwrap_or(());
            }
        }
    }

    pub fn help_menu(&self, ui: &mut Ui) {
        if menu_item(ui, Icon::Book, &"SW_Menu_Readme".localize(), true) {
            open::that("https://github.com/Revivedx/SavageWindRecomp").unwrap_or(());
        }
        ui.separator();
        if menu_item(ui, Icon::Info, &"Menu_Help_About".localize(), true) {
            self.do_update(Message::ShowAbout);
        }
    }
}
