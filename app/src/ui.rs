mod actions;
mod chrome;
mod dialogs;
mod frame;
mod intro;
mod browser;
pub(crate) mod jobs;
mod menus;
mod mod_info;
mod mod_list;
mod mod_options;
mod net;
mod profiles;
mod settings;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt::Write,
    ops::DerefMut,
    path::PathBuf,
    sync::Arc,
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use eframe::{
    egui::IconData,
    NativeOptions,
};
use egui_notify::Toast;
use flume::{Receiver, Sender};
use fs_err as fs;
use parking_lot::{
    MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use mod_list::ModListState;
use serde::{Deserialize, Serialize};
use uk_content::util::HashMap;
use swmm_i18n::*;
use swmm_i18n::string_ext::LocString;
use swmm_core::{
    core::Manager,
    mods::{LookupMod, Mod},
    settings::{Platform, Settings},
};
use uk_mod::{Manifest, ModPlatform};
pub use swmm_widgets::visuals;
use swmm_widgets::{
    egui::{
        self, epaint::Margin, Align, Align2, Frame, Id, Label, LayerId, Layout, RichText, Ui, Vec2,
        ViewportBuilder,
    },
    ext::UiExt,
    icons::{Icon, IconButtonExt},
};

use crate::ui::dialogs::MetaInputModal;

pub use frame::Page;

#[derive(Debug, PartialEq, Eq)]
pub enum FocusedPane {
    ModList,
    None,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Enabled,
    Name,
    Category,
    Version,
    Priority,
}

type Orderer = dyn Fn(&(usize, Mod), &(usize, Mod)) -> std::cmp::Ordering;

impl Sort {
    pub fn orderer(&self) -> Box<Orderer> {
        match self {
            Sort::Enabled => {
                Box::new(|(_, a): &(_, Mod), (_, b): &(_, Mod)| a.enabled.cmp(&b.enabled))
            }
            Sort::Name => {
                Box::new(|(_, a): &(_, Mod), (_, b): &(_, Mod)| a.meta.name.cmp(&b.meta.name))
            }
            Sort::Category => {
                Box::new(|(_, a): &(_, Mod), (_, b): &(_, Mod)| {
                    a.meta.category.u8().cmp(&b.meta.category.u8())
                })
            }
            Sort::Version => {
                Box::new(|(_, a): &(_, Mod), (_, b): &(_, Mod)| {
                    a.meta
                        .version
                        .partial_cmp(&b.meta.version)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            }
            Sort::Priority => Box::new(|&(a, _), &(b, _)| a.cmp(&b)),
        }
    }
}

#[derive(Debug)]
pub enum Message {
    AddMod(Mod),
    AddToProfile(smartstring::alias::String),
    AddProfile,
    Apply,
    ChangeProfile(String),
    ChangeSort(Sort, bool),
    CleanProfile(String),
    ClearDrag,
    ClearSelect,
    CloseAbout,
    CloseLicenses,
    CloseConfirm,
    CloseError,
    CloseChangelog,
    Confirm(Box<Message>, String),
    DeleteProfile(String),
    Deploy,
    Deselect(usize),
    DuplicateProfile(String),
    Error(anyhow_ext::Error),
    HandleMod(Mod),
    HandleSettings,
    InstallMod(Mod),
    ModUpdate,
    MoveSelected(usize),
    NewProfile,
    Noop,
    OpenMod(PathBuf),
    OpenDownloadedMod(PathBuf, swmm_core::source::SourceInfo),
    RefreshModsDisplay,
    Remerge,
    ReloadProfiles,
    RemoveMods(Vec<Mod>),
    RenameProfile(String, String),
    RequestOptions(Mod, bool),
    ResetMods(Option<Manifest>),
    ResetPending,
    ResetSettings,
    SaveSettings,
    SelectAlso(usize),
    SelectFile,
    SelectOnly(usize),
    SelectThrough(usize),
    SetFocus(FocusedPane),
    SetLanguage(LocLang),
    SetTheme(swmm_widgets::visuals::Theme),
    ShowAbout,
    ShowLicenses,
    StartDrag(usize),
    Toast(String),
    ToggleMods(Option<Vec<Mod>>, bool),
    UninstallMods(Option<Vec<Mod>>),
    UpdateOptions(Mod),
    SetPage(Page),
    Launch,
    Launched,
    SwAutoDetect,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct UiState {
    theme: swmm_widgets::visuals::Theme,
    list_state: ModListState,
}

#[derive(Serialize)]
struct UiStateRef<'a> {
    theme: swmm_widgets::visuals::Theme,
    list_state: &'a ModListState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            theme: visuals::Theme::Dark,
            list_state: ModListState::default(),
        }
    }
}

pub struct App {
    core: Arc<Manager>,
    channel: (Sender<Message>, Receiver<Message>),
    mods: Vec<Mod>,
    displayed_mods: Vec<Mod>,
    mod_list_filter: String,
    selected: Vec<Mod>,
    install_queue: VecDeque<PathBuf>,
    update_mod: Option<Mod>,
    error_queue: VecDeque<anyhow_ext::Error>,
    drag_index: Option<usize>,
    hover_index: Option<usize>,
    list_state: ModListState,
    profiles_state: RefCell<profiles::ProfileManagerState>,
    meta_input: dialogs::MetaInputModal,
    page: Page,
    browser: RefCell<browser::BrowserState>,
    game_status: frame::GameStatus,
    focused: FocusedPane,
    error: Option<anyhow_ext::Error>,
    new_profile: Option<String>,
    confirm: Option<(Message, String)>,
    busy: Cell<bool>,
    show_about: bool,
    show_licenses: bool,
    dirty: RwLock<HashMap<String, Manifest>>,
    options_mod: Option<(Mod, bool)>,
    temp_settings: Settings,
    toasts: egui_notify::Toasts,
    theme: swmm_widgets::visuals::Theme,
    intro: intro::Intro,
    theme_fade: Option<intro::ThemeFade>,
    changelog: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        #[cfg(not(windows))]
        if std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland" {
            cc.egui_ctx.set_pixels_per_point(
                std::env::var("WINIT_X11_SCALE_FACTOR")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
            );
        }
        swmm_widgets::load_fonts(&cc.egui_ctx);
        let core = Arc::new(Manager::init().unwrap());
        frame::auto_configure_on_start(&core);
        let ui_state: UiState = fs::read_to_string(core.settings().state_file())
            .context("")
            .and_then(|s| serde_json::from_str(&s).context(""))
            .unwrap_or_default();
        ui_state.theme.set_theme(&cc.egui_ctx);
        #[cfg(windows)]
        {
            chrome::round_window(cc, ui_state.theme.palette().border);
            let shutdown_core = core.clone();
            chrome::close_when_minimized(cc, move || shutdown(&shutdown_core));
        }
        let mods: Vec<_> = core.mod_manager().all_mods().collect();
        let mut temp = mods.iter().cloned().enumerate().collect::<Vec<_>>();
        temp.sort_by(ui_state.list_state.sort.orderer());
        let displayed_mods = if ui_state.list_state.descending {
            temp.into_iter().rev().map(|(_, m)| m).collect()
        } else {
            temp.into_iter().map(|(_, m)| m).collect()
        };
        let (send, recv) = flume::unbounded();
        jobs::UI_SENDER.set(send.clone()).unwrap_or(());
        let temp_settings = core.settings().clone();
        LOCALIZATION.write().update_language(&temp_settings.lang);
        Self {
            selected: mods.first().cloned().into_iter().collect(),
            drag_index: None,
            hover_index: None,
            list_state: ui_state.list_state,
            profiles_state: RefCell::new(profiles::ProfileManagerState::new(&core)),
            meta_input: MetaInputModal::new(send.clone()),
            intro: intro::Intro::new(),
            theme_fade: None,
            displayed_mods,
            mod_list_filter: "".parse().unwrap(),
            mods,
            temp_settings,
            changelog: {
                let settings = core.settings();
                let first_run = settings.last_version.is_none() || settings.platform_config().is_none();
                drop(settings);
                first_run.then(|| "Intro_Message".localize().to_string())
            },
            channel: (send, recv),
            page: Page::Home,
            browser: RefCell::new(browser::BrowserState::default()),
            game_status: frame::GameStatus::check(&core),
            focused: FocusedPane::None,
            error: None,
            new_profile: None,
            confirm: None,
            show_about: false,
            show_licenses: false,
            busy: Cell::new(false),
            dirty: {
                let settings = core.settings();
                RwLock::new(
                    settings
                        .profiles()
                        .map(|p| (p.into(), Default::default()))
                        .collect(),
                )
            },
            options_mod: None,
            toasts: egui_notify::Toasts::new().with_anchor(egui_notify::Anchor::BottomRight),
            theme: ui_state.theme,
            install_queue: Default::default(),
            update_mod: Default::default(),
            error_queue: Default::default(),
            core,
        }
    }

    #[inline(always)]
    fn platform(&self) -> Platform {
        self.core.settings().current_mode
    }

    #[inline(always)]
    fn dirty(&self) -> MappedRwLockReadGuard<'_, uk_mod::Manifest> {
        let dirty = self.dirty.read();
        RwLockReadGuard::map(dirty, |dirty| {
            dirty
                .get(self.core.mod_manager().profile().key().as_str())
                .or_else(|| dirty.values().next())
                .unwrap()
        })
    }

    #[inline(always)]
    fn dirty_mut(&self) -> MappedRwLockWriteGuard<'_, uk_mod::Manifest> {
        let dirty = self.dirty.write();
        RwLockWriteGuard::map(dirty, |dirty| {
            dirty.get_mut(self.core.mod_manager().profile().key().as_str())
            .map(|m| unsafe { &mut *(m as *mut _) })
            .or_else(|| dirty.values_mut().next())
            .unwrap()
        })
    }

    #[inline(always)]
    fn modal_open(&self) -> bool {
        self.error.is_some()
            || self.busy.get()
            || self.options_mod.is_some()
            || self.confirm.is_some()
            || self.show_about
            || self.show_licenses
            || self.new_profile.is_some()
            || self.meta_input.is_open()
            || self.changelog.is_some()
    }

    fn do_update(&self, message: Message) {
        self.channel.0.send(message).unwrap();
    }

    fn do_task(
        &self,
        task: impl 'static
        + Send
        + Sync
        + FnOnce(Arc<Manager>) -> Result<Message>
        + std::panic::UnwindSafe,
    ) {
        let sender = self.channel.0.clone();
        let core = self.core.clone();
        let task = Box::new(task);
        self.busy.set(true);
        thread::spawn(move || {
            let response = match std::panic::catch_unwind(|| task(core.clone())) {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => Message::Error(e),
                Err(e) => {
                    Message::Error(anyhow::format_err!(
                        "{}",
                        e.downcast::<String>().unwrap_or_else(|_| {
                            Box::new(
                                "An unknown error occurred, check the log for possible details."
                                    .to_string(),
                            )
                        })
                    ).context(std::backtrace::Backtrace::force_capture()))
                }
            };
            if let Some(d) = core.settings().dump() {
                d.clear_cache()
            }
            sender.send(response).unwrap();
        });
    }

    fn handle_drops(&mut self, ctx: &eframe::egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if !(self.modal_open() || files.is_empty()) {
            let first = files.first().and_then(|f| f.path.clone()).unwrap();
            self.install_queue
                .extend(files.iter().skip(1).filter_map(|f| f.path.clone()));
            self.error_queue.clear();
            self.do_task(move |core| jobs::open_mod(&core, &first, None));
        }
    }

    fn save_ui_state(&self) {
        let state = UiStateRef {
            theme: self.theme,
            list_state: &self.list_state,
        };
        if let Ok(text) = serde_json::to_string_pretty(&state) {
            fs::write(self.core.settings().state_file(), text).unwrap_or(());
        }
    }

    fn filtered_mods(&self, filter: &str) -> Vec<(usize, &Mod)> {
        self.displayed_mods
            .iter()
            .enumerate()
            .filter(|(_, _mod)| _mod
                .meta
                .name
                .to_lowercase()
                .contains(&filter.to_lowercase())
            )
            .collect::<Vec<_>>()
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        self.handle_update(ctx, frame);
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let size = ctx.screen_rect().size();
        if minimized || size.x < 200.0 || size.y < 200.0 {
            return;
        }
        self.render_title_bar(ctx, frame);
        self.render_status_bar(ctx);
        self.render_nav(ctx);
        self.render_page(ctx);
        chrome::paint_modal_dim(ctx, self.modal_open());
        self.render_error(ctx);
        self.render_confirm(ctx);
        self.render_new_profile(ctx);
        self.render_about(ctx);
        self.render_licenses(ctx);
        self.render_option_picker(ctx);
        self.render_changelog(ctx);
        self.meta_input.ui(ctx);
        self.render_busy(ctx, frame);
        self.toasts.show(ctx);
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        chrome::paint_drop_overlay(ctx, hovering_files && !self.modal_open());
        if self.theme_fade.as_ref().is_some_and(|fade| !fade.paint(ctx)) {
            self.theme_fade = None;
        }
        if self.intro.active() {
            let palette = self.theme.palette();
            let tint = if ctx.style().visuals.dark_mode {
                swmm_widgets::egui::Color32::WHITE
            } else {
                swmm_widgets::egui::Color32::BLACK
            };
            self.intro.show(ctx, &palette, frame::logo_texture(ctx), tint);
        }
        chrome::paint_window_border(ctx, &self.theme.palette());
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            chrome::toggle_fullscreen(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) && i.viewport().fullscreen.unwrap_or(false)) && !self.modal_open() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        chrome::handle_resize(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_ui_state();
        shutdown(&self.core);
    }
}

fn shutdown(core: &Manager) {
    crate::logger::LOGGER.save_log();
    core.settings_mut().last_version = Some(env!("CARGO_PKG_VERSION").into());
    core.settings().save().unwrap_or(());
    swmm_core::util::clear_temp();
}

pub fn main() -> Result<(), eframe::Error> {
    crate::logger::init();
    log::debug!("Logger initialized");
    log::info!("Started SWMM v{}", env!("CARGO_PKG_VERSION"));
    eframe::run_native(
        "SWMM",
        NativeOptions {
            viewport: ViewportBuilder {
                icon: Some(
                    IconData {
                        height: 512,
                        width:  512,
                        rgba:   image::load_from_memory(include_bytes!("../../assets/icon.png"))
                            .unwrap()
                            .to_rgba8()
                            .into_vec(),
                    }
                    .into(),
                ),
                title: Some("Savage Wind Mod Manager".into()),
                app_id: Some("swmm".into()),
                decorations: Some(false),
                transparent: Some(!cfg!(windows)),
                resizable: Some(true),
                min_inner_size: Some(egui::Vec2::new(900.0, 560.0)),
                inner_size: Some(egui::Vec2::new(1180.0, 740.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
