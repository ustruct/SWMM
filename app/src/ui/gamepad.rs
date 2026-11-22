use std::collections::HashMap;

use gilrs::{Axis, Button, EventType, Gilrs};
use swmm_widgets::egui::{self, Id};

use super::{chrome, frame::Page, App, FocusedPane, Message};

const REPEAT_DELAY: f64 = 0.4;
const REPEAT_EVERY: f64 = 0.11;
const STICK_ON: f32 = 0.6;
const STICK_OFF: f32 = 0.35;
pub const SCROLL_TO_SELECTED: &str = "modListScrollToSelected";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pad {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Lt,
    Rt,
    Start,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    Button,
    Stick,
}

pub struct Gamepad {
    gilrs: Option<Gilrs>,
    held: HashMap<(Pad, Source), f64>,
    connected: bool,
    pending: Vec<egui::Event>,
}

impl Gamepad {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(gilrs::Error::NotImplemented(g)) => Some(g),
            Err(e) => {
                log::warn!("Controllers are not available: {e}");
                None
            }
        };
        Self {
            gilrs,
            held: HashMap::new(),
            connected: false,
            pending: Vec::new(),
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    fn key(&mut self, ctx: &egui::Context, key: egui::Key, shift: bool) {
        let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        for pressed in [true, false] {
            self.pending.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        ctx.request_repaint();
    }

    pub fn inject(&mut self, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.pending);
    }

    fn map(button: Button) -> Option<Pad> {
        Some(match button {
            Button::DPadUp => Pad::Up,
            Button::DPadDown => Pad::Down,
            Button::DPadLeft => Pad::Left,
            Button::DPadRight => Pad::Right,
            Button::South => Pad::A,
            Button::East => Pad::B,
            Button::West => Pad::X,
            Button::North => Pad::Y,
            Button::LeftTrigger => Pad::Lb,
            Button::RightTrigger => Pad::Rb,
            Button::LeftTrigger2 => Pad::Lt,
            Button::RightTrigger2 => Pad::Rt,
            Button::Start => Pad::Start,
            Button::Select => Pad::Select,
            _ => return None,
        })
    }

    fn repeats(pad: Pad) -> bool {
        matches!(pad, Pad::Up | Pad::Down | Pad::Left | Pad::Right)
    }

    fn press(&mut self, out: &mut Vec<Pad>, pad: Pad, source: Source, time: f64) {
        if self.held.contains_key(&(pad, source)) {
            return;
        }
        out.push(pad);
        if Self::repeats(pad) {
            self.held.insert((pad, source), time + REPEAT_DELAY);
        }
    }

    fn stick(&mut self, out: &mut Vec<Pad>, value: f32, negative: Pad, positive: Pad, time: f64) {
        if value > STICK_ON {
            self.held.remove(&(negative, Source::Stick));
            self.press(out, positive, Source::Stick, time);
        } else if value < -STICK_ON {
            self.held.remove(&(positive, Source::Stick));
            self.press(out, negative, Source::Stick, time);
        } else if value.abs() < STICK_OFF {
            self.held.remove(&(negative, Source::Stick));
            self.held.remove(&(positive, Source::Stick));
        }
    }

    pub fn poll(&mut self, time: f64) -> Vec<Pad> {
        let mut out = Vec::new();
        let Some(mut gilrs) = self.gilrs.take() else {
            return out;
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(pad) = Self::map(button) {
                        self.press(&mut out, pad, Source::Button, time);
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(pad) = Self::map(button) {
                        self.held.remove(&(pad, Source::Button));
                    }
                }
                EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                    self.stick(&mut out, value, Pad::Down, Pad::Up, time);
                }
                EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                    self.stick(&mut out, value, Pad::Left, Pad::Right, time);
                }
                EventType::Disconnected => self.held.clear(),
                _ => {}
            }
        }
        for ((pad, _), next) in self.held.iter_mut() {
            if time >= *next {
                out.push(*pad);
                *next = time + REPEAT_EVERY;
            }
        }
        self.connected = gilrs.gamepads().any(|(_, g)| g.is_connected());
        self.gilrs = Some(gilrs);
        out
    }
}

impl App {
    fn visible_pages(&self) -> Vec<Page> {
        let browse = self.core.settings().savage_wind.browse_gamebanana;
        Page::MAIN
            .into_iter()
            .filter(|p| browse || *p != Page::Browse)
            .chain(std::iter::once(Page::Settings))
            .collect()
    }

    fn close_top_dialog(&mut self) {
        if self.crash.is_some() {
            self.do_update(Message::CloseCrash);
        } else if self.error.is_some() {
            self.do_update(Message::CloseError);
        } else if self.confirm.is_some() {
            self.do_update(Message::CloseConfirm);
        } else if self.changelog.is_some() {
            self.do_update(Message::CloseChangelog);
        } else if self.show_licenses {
            self.do_update(Message::CloseLicenses);
        } else if self.show_about {
            self.do_update(Message::CloseAbout);
        } else if self.new_profile.is_some() {
            self.new_profile = None;
        }
    }

    fn select_step(&mut self, ctx: &egui::Context, down: bool) {
        if self.mods.is_empty() {
            return;
        }
        let current = if down {
            self.mods.iter().rposition(|m| self.selected.contains(m))
        } else {
            self.mods.iter().position(|m| self.selected.contains(m))
        };
        let last = self.mods.len() - 1;
        let next = match current {
            Some(i) if down => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.do_update(Message::SetFocus(FocusedPane::ModList));
        self.do_update(Message::SelectOnly(next));
        ctx.data_mut(|d| d.insert_temp(Id::new(SCROLL_TO_SELECTED), true));
    }

    fn home_press(&mut self, ctx: &egui::Context, pad: Pad) {
        match pad {
            Pad::Up => self.select_step(ctx, false),
            Pad::Down => self.select_step(ctx, true),
            Pad::A => {
                if let Some(mod_) = self.selected.first().cloned() {
                    self.do_update(Message::ToggleMods(Some(vec![mod_.clone()]), !mod_.enabled));
                }
            }
            Pad::Lt | Pad::Rt => {
                let first = self.mods.iter().position(|m| self.selected.contains(m));
                let last = self.mods.iter().rposition(|m| self.selected.contains(m));
                if let (Some(first), Some(last)) = (first, last) {
                    if pad == Pad::Lt && first > 0 {
                        self.do_update(Message::MoveSelected(first - 1));
                    } else if pad == Pad::Rt && last + 1 < self.mods.len() {
                        self.do_update(Message::MoveSelected(first + 1));
                    }
                    ctx.data_mut(|d| d.insert_temp(Id::new(SCROLL_TO_SELECTED), true));
                }
            }
            Pad::Y => {
                if !self.dirty().is_empty() {
                    self.do_update(Message::Apply);
                } else if self.core.deploy_manager().pending() {
                    self.do_update(Message::Deploy);
                }
            }
            _ => {}
        }
    }

    pub fn poll_gamepad(&mut self, ctx: &egui::Context) {
        let time = chrome::now(ctx);
        let presses = self.gamepad.poll(time);
        if self.gamepad.connected() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        for pad in presses {
            if pad == Pad::Select {
                chrome::toggle_fullscreen(ctx);
                continue;
            }
            if self.modal_open() {
                if self.busy.get() && self.crash.is_none() && self.error.is_none() {
                    continue;
                }
                match pad {
                    Pad::B => self.close_top_dialog(),
                    Pad::A => self.gamepad.key(ctx, egui::Key::Enter, false),
                    Pad::Up | Pad::Left => self.gamepad.key(ctx, egui::Key::Tab, true),
                    Pad::Down | Pad::Right => self.gamepad.key(ctx, egui::Key::Tab, false),
                    _ => {}
                }
                continue;
            }
            match pad {
                Pad::Lb | Pad::Rb => {
                    let pages = self.visible_pages();
                    let index = pages.iter().position(|p| *p == self.page).unwrap_or(0);
                    let next = if pad == Pad::Rb {
                        (index + 1) % pages.len()
                    } else {
                        (index + pages.len() - 1) % pages.len()
                    };
                    self.page = pages[next];
                }
                Pad::X | Pad::Start => {
                    if self.game_status.can_launch && !self.busy.get() {
                        self.do_update(Message::Launch);
                    }
                }
                Pad::B if self.page != Page::Home => self.page = Page::Home,
                _ if self.page == Page::Home => self.home_press(ctx, pad),
                Pad::A => self.gamepad.key(ctx, egui::Key::Enter, false),
                Pad::Up | Pad::Left => self.gamepad.key(ctx, egui::Key::Tab, true),
                Pad::Down | Pad::Right => self.gamepad.key(ctx, egui::Key::Tab, false),
                _ => {}
            }
        }
    }
}
