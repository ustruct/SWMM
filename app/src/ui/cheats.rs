use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use swmm_core::{
    cheats::{self, Cheats, WEAR_LEVELS, WEATHERS},
    control::{Client, CommandError},
    savage_wind,
};
use swmm_i18n::string_ext::LocString;

use super::{
    settings::{MAX_WIDTH, group, toggle},
    *,
};

const POLL: Duration = Duration::from_millis(1200);
const PROCESS_CHECK: Duration = Duration::from_secs(5);
const VISIBLE_FOR: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Link {
    #[default]
    Checking,
    Closed,
    NoControl,
    Live,
}

#[derive(Debug, Clone, Default)]
struct Reply {
    ok: bool,
    text: String,
}

impl Reply {
    fn failed(text: String) -> Self {
        Self { ok: false, text }
    }
}

#[derive(Default)]
struct Shared {
    link: Link,
    game: Option<Cheats>,
    hour: Option<f64>,
    hour_held: bool,
    fly: bool,
    in_world: bool,
    player_pos: Option<[f32; 3]>,
    busy: usize,
    reply: Option<Reply>,
    seen: Option<Instant>,
}

enum Job {
    Cheats(Vec<String>),
    Command { line: String, done: String },
}

struct Live {
    port: u16,
    jobs: Sender<Job>,
    shared: Arc<Mutex<Shared>>,
}

impl Live {
    fn start(port: u16, ctx: &egui::Context) -> Self {
        let (jobs, inbox) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker_shared = shared.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("swmm-cheats".into())
            .spawn(move || worker(port, worker_shared, inbox, ctx))
            .ok();
        Self { port, jobs, shared }
    }

    fn send(&self, job: Job) {
        if let Ok(mut s) = self.shared.lock() {
            s.busy += 1;
        }
        if self.jobs.send(job).is_err()
            && let Ok(mut s) = self.shared.lock()
        {
            s.busy = s.busy.saturating_sub(1);
        }
    }
}

fn command_error(error: &anyhow_ext::Error) -> Option<String> {
    error.downcast_ref::<CommandError>().map(|e| e.0.clone())
}

fn refresh(client: &mut Client, shared: &Mutex<Shared>) -> anyhow_ext::Result<()> {
    let settings = client.call("get")?;
    let time = client.call("time").ok();
    let fly = client.call("fly").ok();
    let pos = client.call("pos").ok();
    let player_pos =
        pos.as_ref().and_then(|p| Some([p["x"].as_f64()? as f32, p["y"].as_f64()? as f32, p["z"].as_f64()? as f32]));
    if let Ok(mut s) = shared.lock() {
        s.in_world = pos.is_some();
        s.player_pos = player_pos;
        s.link = Link::Live;
        s.game = Cheats::from_game(&settings);
        s.hour = time.as_ref().and_then(|t| t["hour"].as_f64());
        s.hour_held = time.as_ref().and_then(|t| t["hour_held"].as_bool()).unwrap_or(false);
        s.fly = fly.as_ref().and_then(|f| f["fly"].as_bool()).unwrap_or(false);
    }
    Ok(())
}

fn finish(shared: &Mutex<Shared>, reply: Option<Reply>) {
    if let Ok(mut s) = shared.lock() {
        s.busy = s.busy.saturating_sub(1);
        if reply.is_some() {
            s.reply = reply;
        }
    }
}

fn run_job(client: &mut Client, job: &Job) -> anyhow_ext::Result<Option<Reply>> {
    match job {
        Job::Cheats(lines) => {
            for line in lines {
                if let Err(e) = client.call(line) {
                    return match command_error(&e) {
                        Some(text) => Ok(Some(Reply::failed(text))),
                        None => Err(e),
                    };
                }
            }
            Ok(None)
        }
        Job::Command { line, done } => match client.call(line) {
            Ok(_) => Ok(Some(Reply { ok: true, text: done.clone() })),
            Err(e) => match command_error(&e) {
                Some(text) => Ok(Some(Reply::failed(text))),
                None => Err(e),
            },
        },
    }
}

fn worker(port: u16, shared: Arc<Mutex<Shared>>, inbox: Receiver<Job>, ctx: egui::Context) {
    let mut client: Option<Client> = None;
    let mut checked: Option<Instant> = None;
    let mut first = true;
    loop {
        let mut job = if first {
            first = false;
            None
        } else {
            match inbox.recv_timeout(POLL) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let (visible, link) = shared
            .lock()
            .map(|s| (s.seen.is_some_and(|t| t.elapsed() < VISIBLE_FOR), s.link))
            .unwrap_or((false, Link::Checking));
        if job.is_none() && !visible && link != Link::Checking {
            continue;
        }
        if client.is_none() {
            client = Client::connect(port).ok();
        }
        if let Some(c) = client.as_mut() {
            let reply = match job.as_ref().map(|j| run_job(c, j)) {
                Some(Ok(reply)) => reply,
                Some(Err(_)) => {
                    client = None;
                    Some(Reply::failed("Cheats_Lost".localize().to_string()))
                }
                None => None,
            };
            if let Some(c) = client.as_mut()
                && refresh(c, &shared).is_err()
            {
                client = None;
            }
            if job.take().is_some() {
                finish(&shared, reply);
            }
        }
        if client.is_none() {
            let due =
                checked.is_none_or(|t| t.elapsed() > PROCESS_CHECK) || matches!(link, Link::Live | Link::Checking);
            if due {
                checked = Some(Instant::now());
                let running = cheats::game_running();
                if let Ok(mut s) = shared.lock() {
                    s.link = if running { Link::NoControl } else { Link::Closed };
                    s.game = None;
                    s.fly = false;
                    s.hour = None;
                    s.hour_held = false;
                    s.in_world = false;
                    s.player_pos = None;
                }
            }
            match job.take() {
                Some(Job::Cheats(_)) => finish(&shared, None),
                Some(Job::Command { .. }) => {
                    finish(&shared, Some(Reply::failed("Cheats_NotRunning".localize().to_string())))
                }
                None => {}
            }
        }
        ctx.request_repaint();
    }
}

#[derive(Default)]
pub struct CheatsState {
    loaded: bool,
    path: Option<PathBuf>,
    cheats: Cheats,
    error: Option<String>,
    live: Option<Live>,
    target: Option<[f32; 3]>,
    map: super::map_view::MapState,
}

impl CheatsState {
    fn load(&mut self, settings: &swmm_core::settings::Settings) {
        if let Err(e) = savage_wind::sync_control_flag(settings) {
            log::warn!("Couldn't update Savage Wind's launch options: {e:?}");
        }
        match cheats::load(settings) {
            Ok((path, cheats)) => {
                self.path = Some(path);
                self.cheats = cheats;
                self.error = None;
            }
            Err(e) => {
                self.path = None;
                self.error = Some(e.to_string());
            }
        }
        self.loaded = true;
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.error = cheats::save_to(&path, &self.cheats).err().map(|e| e.to_string());
    }

    fn command(&self, line: String, done: String) {
        if let Some(live) = &self.live {
            live.send(Job::Command { line, done });
        }
    }
}

fn wear_label(value: i32) -> String {
    let value = cheats::snap_wear(value);
    if value == 0 {
        return "Cheats_Wear_Never".localize().to_string();
    }
    let times = value as f32 / 100.0;
    let speed = if times.fract() == 0.0 { format!("{}x", times as i32) } else { format!("{times:.1}x") };
    if value == 100 { format!("{speed} ({})", "Cheats_Wear_Normal".localize()) } else { speed }
}

fn weather_label(index: usize) -> String {
    const KEYS: [&str; 12] = [
        "Cheats_Weather_game",
        "Cheats_Weather_no_rain",
        "Cheats_Weather_no_storms",
        "Cheats_Weather_clear",
        "Cheats_Weather_cloudy",
        "Cheats_Weather_rain",
        "Cheats_Weather_heavy_rain",
        "Cheats_Weather_snow",
        "Cheats_Weather_heavy_snow",
        "Cheats_Weather_thunderstorm",
        "Cheats_Weather_thunder_rain",
        "Cheats_Weather_sun_shower",
    ];
    KEYS[index.min(WEATHERS.len() - 1)].localize().to_string()
}

fn clock(hour: f64) -> String {
    let minutes = (hour.rem_euclid(24.0) * 60.0).round() as u32 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

struct LiveView {
    link: Link,
    live: bool,
    in_world: bool,
    player_pos: Option<[f32; 3]>,
    hour: Option<f64>,
    hour_held: bool,
    fly: bool,
    reply: Option<Reply>,
}

impl App {
    pub fn render_cheats(&self, ui: &mut Ui) {
        let palette = visuals::palette(ui);
        let mut state = self.cheats.borrow_mut();
        let settings = self.core.settings().clone();
        let file = cheats::settings_file(&settings);
        if !state.loaded || state.path != file {
            state.load(&settings);
        }
        let port = savage_wind::control_port(&settings);
        if state.live.as_ref().is_none_or(|l| l.port != port) {
            state.live = Some(Live::start(port, ui.ctx()));
        }
        let (link, game, busy, view) = match state.live.as_ref().and_then(|l| l.shared.lock().ok()) {
            Some(mut shared) => {
                shared.seen = Some(Instant::now());
                (
                    shared.link,
                    shared.game.clone(),
                    shared.busy,
                    LiveView {
                        link: shared.link,
                        live: shared.link == Link::Live,
                        in_world: shared.in_world,
                        player_pos: shared.player_pos,
                        hour: shared.hour,
                        hour_held: shared.hour_held,
                        fly: shared.fly,
                        reply: shared.reply.clone(),
                    },
                )
            }
            None => (
                Link::Checking,
                None,
                0,
                LiveView {
                    link: Link::Checking,
                    live: false,
                    in_world: false,
                    player_pos: None,
                    hour: None,
                    hour_held: false,
                    fly: false,
                    reply: None,
                },
            ),
        };
        if link == Link::Live
            && busy == 0
            && let Some(game) = game
            && game != state.cheats
        {
            state.cheats = game;
        }
        let editable = state.path.is_some() && matches!(link, Link::Live | Link::Closed);
        let area =
            egui::ScrollArea::vertical().drag_to_scroll(false).id_source("cheatsScroll").auto_shrink([false, false]);
        area.show(ui, |ui| {
            let width = ui.available_width().min(MAX_WIDTH);
            let left = ((ui.available_width() - width) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(left);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    if let Some(error) = state.error.clone() {
                        super::frame::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                super::frame::chip(ui, &"Cheats_Problem".localize(), visuals::RED);
                                ui.add(Label::new(RichText::new(error).color(palette.text)).wrap());
                            });
                        });
                        ui.add_space(12.0);
                    }
                    self.render_cheat_groups(ui, &mut state, editable, view.live, &settings);
                    self.render_live_tools(ui, &mut state, view);
                    ui.add_space(24.0);
                });
            });
        });
    }

    fn render_cheat_groups(
        &self,
        ui: &mut Ui,
        state: &mut CheatsState,
        editable: bool,
        live: bool,
        settings: &swmm_core::settings::Settings,
    ) {
        let palette = visuals::palette(ui);
        let before = state.cheats.clone();
        ui.add_enabled_ui(editable, |ui| {
            let cheats = &mut state.cheats;
            group(ui, &"Cheats_Group_Player".localize(), |group| {
                group.row(&"Cheats_GodMode".localize(), "", 48.0, |ui| {
                    toggle(ui, &mut cheats.god_mode);
                });
                group.row(&"Cheats_Stamina".localize(), "", 48.0, |ui| {
                    toggle(ui, &mut cheats.infinite_stamina);
                });
            });
            group(ui, &"Cheats_Group_Combat".localize(), |group| {
                group.row(&"Cheats_Wear".localize(), "", 170.0, |ui| {
                    egui::ComboBox::from_id_source("cheatWear")
                        .width(150.0)
                        .selected_text(wear_label(cheats.weapon_wear))
                        .show_ui(ui, |ui| {
                            for value in WEAR_LEVELS {
                                ui.selectable_value(&mut cheats.weapon_wear, value, wear_label(value));
                            }
                        });
                });
                group.row(&"Cheats_Arrows".localize(), "", 48.0, |ui| {
                    toggle(ui, &mut cheats.infinite_arrows);
                });
            });
            group(ui, &"Cheats_Group_World".localize(), |group| {
                group.row(&"Cheats_Rupees".localize(), "", 48.0, |ui| {
                    toggle(ui, &mut cheats.infinite_rupees);
                });
                group.row(&"Cheats_Weather".localize(), "", 190.0, |ui| {
                    egui::ComboBox::from_id_source("cheatWeather")
                        .width(170.0)
                        .selected_text(weather_label(cheats.weather))
                        .show_ui(ui, |ui| {
                            for index in 0..WEATHERS.len() {
                                ui.selectable_value(&mut cheats.weather, index, weather_label(index));
                            }
                        });
                });
            });
        });
        let mut turn_off = false;
        let mut reload = false;
        ui.horizontal(|ui| {
            let active = state.cheats.active();
            let summary = if active == 0 {
                "Cheats_NoneOn".localize().to_string()
            } else {
                "Cheats_SomeOn".localize().replace("{count}", &active.to_string())
            };
            ui.label(RichText::new(summary).size(12.0).color(palette.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(path) = state.path.clone()
                    && ui
                        .icon_text_button("Cheats_OpenFile".localize(), Icon::External)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    && path.exists()
                {
                    open::that(&path).unwrap_or(());
                }
                turn_off =
                    ui.add_enabled(editable && active > 0, egui::Button::new("Cheats_TurnOff".localize())).clicked();
                reload = ui.icon_text_button("Cheats_Reload".localize(), Icon::Refresh).clicked();
            });
        });
        ui.add_space(20.0);
        if turn_off {
            state.cheats = Cheats::default();
        }
        let changed = before.changed(&state.cheats);
        if editable && !changed.is_empty() {
            state.save();
            if live && let Some(l) = &state.live {
                l.send(Job::Cheats(changed.iter().map(|c| state.cheats.command(*c)).collect()));
            }
        }
        if reload {
            state.load(settings);
        }
    }

    fn render_live_tools(&self, ui: &mut Ui, state: &mut CheatsState, view: LiveView) {
        let palette = visuals::palette(ui);
        ui.add_enabled_ui(view.live && view.in_world, |ui| {
            group(ui, &"Cheats_Group_Play".localize(), |group| {
                group.row(&"Cheats_Time".localize(), "", 190.0, |ui| {
                    let current = view.hour.map(clock).unwrap_or_else(|| "--:--".into());
                    egui::ComboBox::from_id_source("cheatHour").width(170.0).selected_text(current).show_ui(ui, |ui| {
                        for h in 0..24u32 {
                            if ui.selectable_label(false, format!("{h:02}:00")).clicked() {
                                state.command(
                                    format!("time {h}"),
                                    "Cheats_Time_Done".localize().replace("{time}", &format!("{h:02}:00")),
                                );
                            }
                        }
                    });
                });
                group.row(&"Cheats_Freeze".localize(), "", 48.0, |ui| {
                    let mut on = view.hour_held;
                    if toggle(ui, &mut on).changed() {
                        let (line, done) = if on {
                            ("time freeze", "Cheats_Freeze_On")
                        } else {
                            ("time unfreeze", "Cheats_Freeze_Off")
                        };
                        state.command(line.into(), done.localize().to_string());
                    }
                });
                group.row(&"Cheats_Fly".localize(), "", 48.0, |ui| {
                    let mut on = view.fly;
                    if toggle(ui, &mut on).changed() {
                        let (line, done) = if on { ("fly on", "Cheats_Fly_On") } else { ("fly off", "Cheats_Fly_Off") };
                        state.command(line.into(), done.localize().to_string());
                    }
                });
            });
        });
        if view.link == Link::NoControl {
            ui.label(RichText::new("Cheats_Restart".localize()).size(12.0).color(palette.muted));
        } else if view.live
            && view.in_world
            && let Some(reply) = view.reply.clone()
        {
            let color = if reply.ok { visuals::GREEN } else { visuals::RED };
            ui.add(Label::new(RichText::new(&reply.text).size(12.0).color(color)).wrap());
        }
        ui.add_space(20.0);
        self.render_teleport(ui, state, &view);
    }

    fn render_teleport(&self, ui: &mut Ui, state: &mut CheatsState, view: &LiveView) {
        let palette = visuals::palette(ui);
        let settings = self.core.settings().clone();
        let can_go = view.live && view.in_world;
        group(ui, &"Cheats_Teleport".localize(), |group| {
            group.block(|ui| {
                if let Some(picked) =
                    super::map_view::show(ui, &mut state.map, &settings, state.target, view.player_pos)
                {
                    state.target = Some(picked);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let mut picked = state.target.unwrap_or([0.0; 3]);
                    ui.add_enabled_ui(state.target.is_some(), |ui| {
                        for (label, value) in ["X", "Y", "Z"].iter().zip(picked.iter_mut()) {
                            ui.label(RichText::new(*label).color(palette.muted));
                            ui.add(egui::DragValue::new(value).speed(1.0).fixed_decimals(1).range(-7000.0..=7000.0));
                            ui.add_space(6.0);
                        }
                    });
                    if state.target.is_some() {
                        state.target = Some(picked);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let go = ui
                            .add_enabled(
                                can_go && state.target.is_some(),
                                egui::Button::new("Cheats_Teleport_Button".localize()),
                            )
                            .clicked();
                        if let Some(player) = view.player_pos
                            && ui.button("Cheats_Teleport_Player".localize()).clicked()
                        {
                            state.target = Some(player);
                        }
                        if go && let Some([x, y, z]) = state.target {
                            state.command(
                                format!("teleport {x:.1} {y:.1} {z:.1}"),
                                "Cheats_Teleport_Done"
                                    .localize()
                                    .replace("{x}", &format!("{x:.0}"))
                                    .replace("{y}", &format!("{y:.0}"))
                                    .replace("{z}", &format!("{z:.0}")),
                            );
                        }
                    });
                });
            });
        });
    }
}
