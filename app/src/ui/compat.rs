use swmm_core::compat::{self, Entry, Status};
use swmm_i18n::string_ext::LocString;
use swmm_widgets::egui::Color32;

use super::*;

#[derive(Default)]
pub struct CompatState {
    pub query: String,
    status: Option<Status>,
    mine: Vec<Entry>,
    loaded: bool,
}

impl CompatState {
    fn load(&mut self) {
        if !self.loaded {
            self.mine = compat::mine();
            self.loaded = true;
        }
    }

    pub fn tested(&mut self, entry: Entry) {
        self.loaded = false;
        self.load();
        self.status = None;
        self.query = entry.name;
    }

    fn shows(&self, entry: &Entry) -> bool {
        let query = self.query.trim().to_lowercase();
        self.status.is_none_or(|s| s == entry.status)
            && (query.is_empty()
                || entry.name.to_lowercase().contains(&query)
                || entry.file.to_lowercase().contains(&query))
    }
}

pub fn look(status: Status) -> (Color32, String) {
    match status {
        Status::Full => (visuals::GREEN, "Browse_Compat_Full".localize().to_string()),
        Status::Partial => (visuals::YELLOW, "Browse_Compat_Partial".localize().to_string()),
        Status::Failed => (visuals::RED, "Browse_Compat_Failed".localize().to_string()),
    }
}

fn status_name(status: Option<Status>) -> String {
    match status {
        None => "Compat_All".localize().to_string(),
        Some(s) => look(s).1,
    }
}

enum RowAction {
    Open(String),
    Forget(Entry),
}

fn render_row(ui: &mut Ui, entry: &Entry, yours: bool) -> Option<RowAction> {
    let palette = visuals::palette(ui);
    let mut action = None;
    super::frame::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (color, label) = look(entry.status);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 180.0).max(120.0));
                ui.add(Label::new(RichText::new(entry.name.as_str()).size(14.0).strong().color(palette.text)).truncate());
                if !entry.file.is_empty() {
                    ui.add(Label::new(RichText::new(entry.file.as_str()).size(12.0).color(palette.muted)).truncate());
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.horizontal(|ui| super::frame::chip(ui, &label, color));
            });
        });
        if !entry.reason.is_empty() {
            let text = if entry.status == Status::Partial {
                format!("{} {}", "Browse_Compat_LeftOut".localize(), entry.reason)
            } else {
                entry.reason.clone()
            };
            ui.add(Label::new(RichText::new(text).size(12.0).color(palette.text)).wrap());
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mut facts = vec![
                "Compat_Tested".localize().replace("{date}", entry.date()),
                if yours { "Compat_ByYou".localize().to_string() } else { "Compat_BySwmm".localize().to_string() },
            ];
            if entry.converted > 0 {
                facts.push("Compat_Converted".localize().replace("{count}", &entry.converted.to_string()));
            }
            ui.label(RichText::new(facts.join("  ·  ")).size(12.0).color(palette.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if yours && ui.icon_text_button("Compat_Remove".localize(), Icon::Delete).clicked() {
                    action = Some(RowAction::Forget(entry.clone()));
                }
                if let Some(url) = entry.url() {
                    if ui.icon_text_button("Browse_OpenPage".localize(), Icon::External).clicked() {
                        action = Some(RowAction::Open(url));
                    }
                }
            });
        });
    });
    ui.add_space(6.0);
    action
}

fn section(ui: &mut Ui, text: &str) {
    let palette = visuals::palette(ui);
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(13.0).strong().color(palette.muted));
    ui.add_space(4.0);
}

impl App {
    pub fn render_compat(&self, ui: &mut Ui) {
        let palette = visuals::palette(ui);
        let mut state = self.compat.borrow_mut();
        state.load();
        ui.add(Label::new(RichText::new("Compat_Intro".localize()).color(palette.muted)).wrap());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let (filter_width, test_width) = (170.0, 190.0);
            let search_width =
                (ui.available_width() - filter_width - test_width - 2.0 * ui.spacing().item_spacing.x).max(140.0);
            super::chrome::search_box(ui, search_width, &mut state.query, &"Compat_Search".localize());
            super::chrome::control_sized(ui, |ui| {
                egui::ComboBox::from_id_source("compatStatus")
                    .width(filter_width - 16.0)
                    .selected_text(status_name(state.status))
                    .show_ui(ui, |ui| {
                        for status in [None, Some(Status::Full), Some(Status::Partial), Some(Status::Failed)] {
                            ui.selectable_value(&mut state.status, status, status_name(status));
                        }
                    });
            });
            if super::chrome::fancy_button(ui, Icon::FileImport, &"Compat_TestFile".localize(), test_width, super::chrome::CONTROL_HEIGHT)
                .on_hover_text("Compat_TestFile_Help".localize())
                .clicked()
            {
                self.do_update(Message::SelectCompatFile);
            }
        });
        ui.add_space(8.0);
        let bundled: Vec<&Entry> = compat::bundled().iter().filter(|e| state.shows(e)).collect();
        let mine: Vec<Entry> = state.mine.iter().filter(|e| state.shows(e)).cloned().collect();
        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if bundled.is_empty() && mine.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Compat_Empty".localize()).color(palette.muted));
                });
                return;
            }
            if !mine.is_empty() {
                section(ui, &"Compat_Yours".localize());
                for entry in &mine {
                    action = render_row(ui, entry, true).or(action.take());
                }
            }
            if !bundled.is_empty() {
                let all = compat::bundled();
                let count = |s: Status| all.iter().filter(|e| e.status == s).count().to_string();
                let title = "Compat_Bundled"
                    .localize()
                    .replace("{date}", compat::TESTED)
                    .replace("{full}", &count(Status::Full))
                    .replace("{partial}", &count(Status::Partial))
                    .replace("{failed}", &count(Status::Failed));
                section(ui, &title);
                for entry in bundled {
                    action = render_row(ui, entry, false).or(action.take());
                }
            }
        });
        match action {
            Some(RowAction::Open(url)) => open::that(url).unwrap_or(()),
            Some(RowAction::Forget(entry)) => {
                if let Err(e) = compat::forget(&entry) {
                    self.do_update(Message::Error(e));
                }
                state.loaded = false;
            }
            None => {}
        }
    }
}
