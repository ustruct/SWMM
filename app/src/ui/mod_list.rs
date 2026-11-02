use std::{collections::HashMap, process::Command};

use serde::{Deserialize, Serialize};
use strfmt::Format;
use swmm_core::mods::Mod;
use swmm_i18n::string_ext::LocString;
use swmm_modpack::ModCategory;
use swmm_widgets::{
    egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Id, Key, Rect, Rounding, Sense, Stroke, Ui},
    icons::Icon,
    visuals,
};

use super::{chrome, App, FocusedPane, Message, Sort};

const ROW_HEIGHT: f32 = 46.0;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModListState {
    pub sort: Sort,
    pub descending: bool,
}

impl Default for ModListState {
    fn default() -> Self {
        Self {
            sort: Sort::Priority,
            descending: false,
        }
    }
}

enum RowAction {
    CopyToProfile(smartstring::alias::String),
    Update,
    Uninstall,
    Toggle(bool),
    Move(usize),
}

fn switch(ui: &mut Ui, rect: Rect, id: Id, on: bool) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let palette = visuals::palette(ui);
    let t = ui.ctx().animate_bool_with_time(id, on, 0.16);
    let track = rect.shrink2(vec2(0.0, (rect.height() - 18.0) / 2.0));
    ui.painter().rect(
        track,
        Rounding::same(9.0),
        chrome::mix(palette.base, visuals::ACCENT, t),
        Stroke::new(1.0_f32, chrome::mix(palette.muted, visuals::ACCENT, t)),
    );
    let x = egui::lerp((track.left() + 9.0)..=(track.right() - 9.0), chrome::ease(t));
    ui.painter().circle_filled(
        pos2(x, track.center().y),
        if response.hovered() { 5.5 } else { 4.5 },
        chrome::mix(palette.muted, Color32::WHITE, t),
    );
    response
}

impl App {
    pub fn render_modlist(&mut self, ui: &mut Ui) {
        let palette = visuals::palette(ui);
        let filter = self.mod_list_filter.to_lowercase();
        let rows: Vec<usize> = self
            .mods
            .iter()
            .enumerate()
            .filter(|(_, m)| filter.is_empty() || m.meta.name.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect();
        if rows.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let text = if self.mods.is_empty() {
                    "Helper_Modlist_NoMods".localize()
                } else {
                    "SW_SearchMods".localize()
                };
                ui.label(egui::RichText::new(text).color(palette.muted));
            });
            return;
        }
        let can_drag = filter.is_empty();
        let pointer = ui.ctx().pointer_interact_pos();
        let mut row_rects: Vec<(usize, Rect)> = Vec::with_capacity(rows.len());
        let mut pending: Option<(usize, RowAction)> = None;
        egui::ScrollArea::vertical().drag_to_scroll(false)
            .id_source("modListScroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for &index in rows.iter() {
                    let mod_ = self.mods[index].clone();
                    let width = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
                    row_rects.push((index, rect));
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let id = Id::new(("modRow", mod_.hash()));
                    let response = ui.interact(rect, id, Sense::click_and_drag());
                    let selected = self.selected.contains(&mod_);
                    let hover = ui.ctx().animate_bool_with_time(id.with("hover"), response.hovered(), 0.12);
                    let picked = ui.ctx().animate_bool_with_time(id.with("picked"), selected, 0.16);
                    let fill = (hover * 0.55).max(picked);
                    if fill > 0.0 {
                        ui.painter().rect_filled(rect, Rounding::same(8.0), palette.hover.gamma_multiply(fill));
                    }
                    if picked > 0.0 {
                        let bar = Rect::from_center_size(
                            pos2(rect.left() + 2.0, rect.center().y),
                            vec2(3.0, 18.0 * picked),
                        );
                        ui.painter().rect_filled(bar, Rounding::same(1.5), palette.accent_text);
                    }
                    let painter = ui.painter().with_clip_rect(rect);
                    painter.text(
                        pos2(rect.left() + 30.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        (index + 1).to_string(),
                        FontId::proportional(12.0),
                        palette.muted,
                    );
                    let toggle_rect = Rect::from_min_size(pos2(rect.left() + 42.0, rect.top()), vec2(34.0, rect.height()));
                    let category = match mod_.meta.category {
                        ModCategory::None | ModCategory::Other => None,
                        other => Some(other.to_loc_str().localize().into_owned()),
                    };
                    let category_width = category
                        .as_ref()
                        .map(|c| {
                            ui.fonts(|f| f.layout_no_wrap(c.clone(), FontId::proportional(12.0), palette.muted))
                                .size()
                                .x
                                + 24.0
                        })
                        .unwrap_or(0.0);
                    let text_left = toggle_rect.right() + 14.0;
                    let text_rect = Rect::from_min_max(
                        pos2(text_left, rect.top()),
                        pos2((rect.right() - 12.0 - category_width).max(text_left + 40.0), rect.bottom()),
                    );
                    let text_painter = ui.painter().with_clip_rect(text_rect);
                    text_painter.text(
                        pos2(text_rect.left(), rect.center().y - 9.0),
                        Align2::LEFT_CENTER,
                        mod_.meta.name.as_str(),
                        FontId::proportional(14.0),
                        if mod_.enabled { palette.text } else { palette.muted },
                    );
                    let mut sub = format!("v{}", mod_.meta.version);
                    if !mod_.meta.author.is_empty() {
                        sub.push_str(&format!("  ·  {}", mod_.meta.author));
                    }
                    text_painter.text(
                        pos2(text_rect.left(), rect.center().y + 10.0),
                        Align2::LEFT_CENTER,
                        sub,
                        FontId::proportional(12.0),
                        palette.muted,
                    );
                    if let Some(category) = category {
                        let chip = Rect::from_min_max(
                            pos2(rect.right() - 12.0 - category_width + 8.0, rect.center().y - 11.0),
                            pos2(rect.right() - 12.0, rect.center().y + 11.0),
                        );
                        painter.rect_filled(chip, Rounding::same(11.0), palette.base);
                        painter.text(chip.center(), Align2::CENTER_CENTER, category, FontId::proportional(12.0), palette.muted);
                    }
                    let toggled = switch(ui, toggle_rect, id.with("switch"), mod_.enabled).clicked();
                    let menu_mod = mod_.clone();
                    response.context_menu(|ui| {
                        if let Some(action) = Self::render_mod_context_menu(&self.core, menu_mod, ui) {
                            pending = Some((index, action));
                        }
                    });
                    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                    if toggled {
                        self.do_update(Message::ToggleMods(Some(vec![mod_.clone()]), !mod_.enabled));
                    } else if response.clicked() {
                        self.do_update(Message::SetFocus(FocusedPane::ModList));
                        if selected && ctrl {
                            self.do_update(Message::Deselect(index));
                        } else if shift {
                            self.do_update(Message::SelectThrough(index));
                        } else if ctrl {
                            self.do_update(Message::SelectAlso(index));
                        } else {
                            self.do_update(Message::SelectOnly(index));
                        }
                    } else if response.drag_started() && can_drag && !ctrl && !shift {
                        self.do_update(Message::StartDrag(index));
                    }
                }
                if self.drag_index.is_some() && can_drag {
                    if let Some(pointer) = pointer {
                        let dest = row_rects
                            .iter()
                            .find(|(_, r)| pointer.y < r.center().y)
                            .map(|(i, _)| *i)
                            .unwrap_or(self.mods.len());
                        self.hover_index = Some(dest);
                        let y = row_rects
                            .iter()
                            .find(|(i, _)| *i == dest)
                            .map(|(_, r)| r.top() - 1.0)
                            .or_else(|| row_rects.last().map(|(_, r)| r.bottom() + 1.0));
                        if let Some(y) = y {
                            let left = row_rects.first().map(|(_, r)| r.left()).unwrap_or(0.0);
                            let right = row_rects.first().map(|(_, r)| r.right()).unwrap_or(0.0);
                            ui.painter().line_segment(
                                [pos2(left + 8.0, y), pos2(right - 8.0, y)],
                                Stroke::new(2.0_f32, palette.accent_text),
                            );
                            ui.painter().circle_filled(pos2(left + 8.0, y), 3.5, palette.accent_text);
                        }
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    }
                }
            });
        if let Some((index, action)) = pending {
            self.run_row_action(index, action);
        }
        if ui.input(|i| i.pointer.any_released()) && self.drag_index.is_some() {
            match self.hover_index.take() {
                Some(dest) => {
                    let selected_before = self
                        .mods
                        .iter()
                        .take(dest)
                        .filter(|m| self.selected.contains(m))
                        .count();
                    let target = dest.saturating_sub(selected_before);
                    self.do_update(Message::MoveSelected(target));
                }
                None => self.do_update(Message::ClearDrag),
            }
        }
        self.handle_list_keys(ui);
    }

    fn handle_list_keys(&self, ui: &Ui) {
        if self.focused != FocusedPane::ModList || self.modal_open() || ui.memory(|m| m.focused().is_some()) {
            return;
        }
        let (down, up, shift) = ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.modifiers.shift));
        if down {
            if let Some(last) = self.mods.iter().rposition(|m| self.selected.contains(m)) {
                let next = (last + 1).min(self.mods.len().saturating_sub(1));
                self.do_update(if shift { Message::SelectAlso(next) } else { Message::SelectOnly(next) });
            }
        } else if up {
            if let Some(first) = self.mods.iter().position(|m| self.selected.contains(m)) {
                let prev = first.saturating_sub(1);
                self.do_update(if shift { Message::SelectAlso(prev) } else { Message::SelectOnly(prev) });
            }
        }
    }

    fn run_row_action(&self, index: usize, action: RowAction) {
        let Some(mod_) = self.mods.get(index) else {
            return;
        };
        if !self.selected.contains(mod_) {
            self.do_update(Message::SelectOnly(index));
        }
        match action {
            RowAction::CopyToProfile(profile) => self.do_update(Message::AddToProfile(profile)),
            RowAction::Update => self.do_update(Message::ModUpdate),
            RowAction::Uninstall => {
                let message = "Mod_Uninstall_Confirmation".localize();
                let vars = HashMap::from([("mod_name".to_string(), mod_.meta.name.to_string())]);
                let prompt = message.format(&vars).unwrap_or_else(|_| message.to_string());
                self.do_update(Message::Confirm(Message::UninstallMods(None).into(), prompt));
            }
            RowAction::Toggle(state) => self.do_update(Message::ToggleMods(None, state)),
            RowAction::Move(dest) => self.do_update(Message::MoveSelected(dest)),
        }
    }

    fn render_mod_context_menu(core: &swmm_core::core::Manager, mod_: Mod, ui: &mut Ui) -> Option<RowAction> {
        let mut result = None;
        let item = |ui: &mut Ui, icon: Icon, text: std::borrow::Cow<'static, str>| -> bool {
            ui.spacing_mut().button_padding = vec2(10.0, 4.0);
            let job = swmm_widgets::icons::icon_text_job(ui, icon, &text, None);
            let clicked = ui.add(egui::Button::new(job).min_size(vec2(190.0, 26.0))).clicked();
            if clicked {
                ui.close_menu();
            }
            clicked
        };
        let toggle_text = if mod_.enabled { "Mod_Disable" } else { "Mod_Enable" };
        if item(ui, if mod_.enabled { Icon::Blank } else { Icon::Check }, toggle_text.localize()) {
            result = Some(RowAction::Toggle(!mod_.enabled));
        }
        if item(ui, Icon::FileUpload, "Generic_Update".localize()) {
            result = Some(RowAction::Update);
        }
        if item(ui, Icon::FolderOpen, "Mod_View".localize()) {
            let folder = if mod_.path.is_dir() { mod_.path.clone() } else { mod_.path.parent().map(|p| p.to_path_buf()).unwrap_or_default() };
            let _ = Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(folder).spawn();
        }
        ui.separator();
        if item(ui, Icon::DoubleUp, "Mod_Move_Start".localize()) {
            result = Some(RowAction::Move(0));
        }
        if item(ui, Icon::DoubleDown, "Mod_Move_End".localize()) {
            result = Some(RowAction::Move(usize::MAX / 2));
        }
        let others: Vec<_> = core
            .settings()
            .profiles()
            .filter(|p| core.mod_manager().profile().key() != p)
            .collect();
        if !others.is_empty() {
            ui.menu_button("Mod_Send".localize(), |ui| {
                for profile in others {
                    if ui.button(profile.as_str()).clicked() {
                        result = Some(RowAction::CopyToProfile(profile));
                        ui.close_menu();
                    }
                }
            });
        }
        ui.separator();
        if item(ui, Icon::Delete, "Mod_Uninstall".localize()) {
            result = Some(RowAction::Uninstall);
        }
        result
    }
}
