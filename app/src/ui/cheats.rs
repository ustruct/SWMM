use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use swmm_core::settings::Settings;
use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{self, pos2, vec2, Color32, Margin, RichText, Rounding, Sense, Stroke, Ui},
    visuals,
};

use super::App;

const STATUS_EVERY: Duration = Duration::from_secs(1);

struct Entry {
    key: &'static str,
    name: &'static str,
    desc: &'static str,
    ready: bool,
}

const ENTRIES: [Entry; 15] = [
    Entry { key: "map_stamps", name: "CH_MapStamps", desc: "CH_MapStamps_Desc", ready: false },
    Entry { key: "arrow_draw_speed", name: "CH_ArrowDrawSpeed", desc: "CH_ArrowDrawSpeed_Desc", ready: false },
    Entry { key: "durability", name: "CH_Durability", desc: "CH_Durability_Desc", ready: false },
    Entry { key: "infinite_amiibo", name: "CH_InfiniteAmiibo", desc: "CH_InfiniteAmiibo_Desc", ready: true },
    Entry { key: "infinite_arrows", name: "CH_InfiniteArrows", desc: "CH_InfiniteArrows_Desc", ready: false },
    Entry { key: "infinite_daruk", name: "CH_InfiniteDaruk", desc: "CH_InfiniteDaruk_Desc", ready: true },
    Entry { key: "infinite_hearts", name: "CH_InfiniteHearts", desc: "CH_InfiniteHearts_Desc", ready: true },
    Entry { key: "infinite_mipha", name: "CH_InfiniteMipha", desc: "CH_InfiniteMipha_Desc", ready: true },
    Entry {
        key: "infinite_motorcycle_energy",
        name: "CH_InfiniteMotorcycle",
        desc: "CH_InfiniteMotorcycle_Desc",
        ready: true,
    },
    Entry { key: "infinite_revali", name: "CH_InfiniteRevali", desc: "CH_InfiniteRevali_Desc", ready: true },
    Entry { key: "infinite_stamina", name: "CH_InfiniteStamina", desc: "CH_InfiniteStamina_Desc", ready: true },
    Entry { key: "infinite_urbosa", name: "CH_InfiniteUrbosa", desc: "CH_InfiniteUrbosa_Desc", ready: true },
    Entry { key: "master_mode", name: "CH_MasterMode", desc: "CH_MasterMode_Desc", ready: false },
    Entry { key: "prevent_spawns", name: "CH_PreventSpawns", desc: "CH_PreventSpawns_Desc", ready: false },
    Entry { key: "motorcycle_regions", name: "CH_MotorcycleRegions", desc: "CH_MotorcycleRegions_Desc", ready: false },
];

#[derive(Default, Clone, PartialEq)]
enum GameState {
    #[default]
    Off,
    Starting,
    Loading(String),
    Ready(String),
    Unsupported,
}

pub struct CheatsState {
    enabled: [bool; 15],
    state: GameState,
    checked: Option<Instant>,
}

impl Default for CheatsState {
    fn default() -> Self {
        let text = std::fs::read_to_string(config_path()).unwrap_or_default();
        let mut enabled = [false; 15];
        for (i, entry) in ENTRIES.iter().enumerate() {
            enabled[i] = entry.ready
                && text
                    .lines()
                    .filter_map(|l| l.split_once('='))
                    .any(|(k, v)| k.trim() == entry.key && v.trim() == "1");
        }
        Self { enabled, state: GameState::Off, checked: None }
    }
}

fn config_path() -> PathBuf {
    Settings::config_dir().join("cheats.ini")
}

fn status_path() -> PathBuf {
    Settings::config_dir().join("cheats_status.ini")
}

impl CheatsState {
    fn save(&self) {
        let mut text = String::new();
        for (i, entry) in ENTRIES.iter().enumerate() {
            text.push_str(&format!("{}={}\n", entry.key, self.enabled[i] as u8));
        }
        let _ = std::fs::create_dir_all(Settings::config_dir());
        let _ = std::fs::write(config_path(), text);
    }

    fn refresh(&mut self) {
        if self.checked.is_some_and(|t| t.elapsed() < STATUS_EVERY) {
            return;
        }
        self.checked = Some(Instant::now());
        let path = status_path();
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(60 * 60 * 12));
        let text = if fresh { std::fs::read_to_string(&path).unwrap_or_default() } else { String::new() };
        let value = |key: &str| {
            text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v.trim().to_string())
        };
        self.state = match value("state").as_deref() {
            Some("starting") => GameState::Starting,
            Some("loading") => GameState::Loading(value("version").unwrap_or_default()),
            Some("ready") => GameState::Ready(value("version").unwrap_or_default()),
            Some("unsupported") => GameState::Unsupported,
            _ => GameState::Off,
        };
    }
}

fn switch(ui: &mut Ui, on: bool, enabled: bool) -> egui::Response {
    let palette = visuals::palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    let t = ui.ctx().animate_bool_with_time(response.id, on, 0.12);
    let fill = if on { visuals::ACCENT } else { palette.base };
    let fill = if enabled { fill } else { fill.gamma_multiply(0.45) };
    ui.painter().rect(
        rect,
        Rounding::same(9.0),
        fill,
        Stroke::new(1.0_f32, if on { visuals::ACCENT } else { palette.border }),
    );
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    let knob = if enabled { Color32::WHITE } else { palette.muted };
    ui.painter().circle_filled(pos2(x, rect.center().y), 6.0, knob);
    response
}

fn status_line(ui: &mut Ui, state: &GameState) {
    let palette = visuals::palette(ui);
    let green = Color32::from_rgb(0x5c, 0xd1, 0x7a);
    let amber = Color32::from_rgb(0xe8, 0xc3, 0x5c);
    let (text, color) = match state {
        GameState::Off => ("CH_Status_Off".localize().into_owned(), palette.muted),
        GameState::Starting => ("CH_Status_Starting".localize().into_owned(), amber),
        GameState::Loading(v) => (format!("{} · {v}", "CH_Status_Loading".localize()), amber),
        GameState::Ready(version) => (format!("{} · {version}", "CH_Status_Ready".localize()), green),
        GameState::Unsupported => ("CH_Status_Unsupported".localize().into_owned(), ui.visuals().warn_fg_color),
    };
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.5, color);
        ui.label(RichText::new(text).size(12.5).color(palette.muted));
    });
}

impl App {
    pub fn render_cheats(&mut self, ui: &mut Ui) {
        let mut state = self.cheats.borrow_mut();
        state.refresh();
        ui.ctx().request_repaint_after(STATUS_EVERY);
        let palette = visuals::palette(ui);
        status_line(ui, &state.state);
        ui.add_space(8.0);
        let mut changed = false;
        egui::ScrollArea::vertical().id_source("cheatsList").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::none()
                .fill(palette.raised)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .rounding(8.0)
                .inner_margin(Margin::symmetric(4.0, 4.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, entry) in ENTRIES.iter().enumerate() {
                        if i > 0 {
                            let y = ui.cursor().top();
                            let r = ui.max_rect();
                            ui.painter().hline((r.left() + 8.0)..=(r.right() - 8.0), y, Stroke::new(1.0_f32, palette.border));
                        }
                        let row = egui::Frame::none().inner_margin(Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let toggled = switch(ui, state.enabled[i], entry.ready).clicked();
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.horizontal(|ui| {
                                        let color = if entry.ready { palette.text } else { palette.muted };
                                        ui.label(RichText::new(entry.name.localize()).size(13.5).color(color));
                                        if !entry.ready {
                                            ui.label(
                                                RichText::new("CH_Soon".localize())
                                                    .size(10.5)
                                                    .color(palette.muted)
                                                    .background_color(palette.base),
                                            );
                                        }
                                    });
                                    let desc = entry.desc.localize().into_owned();
                                    ui.add(egui::Label::new(RichText::new(desc).size(11.5).color(palette.muted)).wrap());
                                });
                                toggled
                            })
                            .inner
                        });
                        let clicked = row.inner || row.response.interact(Sense::click()).clicked();
                        if clicked && entry.ready {
                            state.enabled[i] = !state.enabled[i];
                            changed = true;
                        }
                    }
                });
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new("CH_Note".localize()).size(11.5).color(palette.muted)).wrap());
        });
        if changed {
            state.save();
        }
    }
}
