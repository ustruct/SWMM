mod actions;
mod cheats;
mod chrome;
mod compat;
mod conflicts;
mod crash;
mod dialogs;
mod frame;
mod intro;
mod browser;
pub(crate) mod jobs;
mod map_view;
mod menus;
mod mod_info;
mod mod_list;
mod mod_options;
mod net;
mod profiles;
mod settings;
mod updater;
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
                Box::new(|(_, a): &(_, Mod), (_, b): &(_, Mod)| {
                    a.display_name().to_lowercase().cmp(&b.display_name().to_lowercase())
                })
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
    EditMod(Mod),
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
    ReinstallMods(Option<Vec<Mod>>),
    Remerge,
    ReloadProfiles,
    RemoveMods(Vec<Mod>),
    RenameProfile(String, String),
    RequestOptions(Mod, bool),
    ResetMods(Option<Manifest>),
    ResetPending,
    ResetSettings,
    SaveModDetails(usize, String, String),
    SaveSettings,
    SelectAll,
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
    OpenGameConfig,
    GameConfigOpened,
    SwAutoDetect,
    GameCrashed(Box<swmm_core::crash::CrashReport>),
    CloseCrash,
    BackupSaves,
    RestoreSaves(PathBuf),
    SavesChanged(Option<String>),
    ExportModpack(String),
    ImportModpack(PathBuf),
    SelectModpack,
    ModpackImported(String),
    Finished,
    SelectCompatFile,
    TestCompat { path: PathBuf, id: u64, name: String, file: String, downloaded: bool },
    CompatTested(Box<swmm_core::compat::Entry>),
    ShowCompat(String),
    CheckForUpdates,
    UpdateAvailable(Box<updater::Release>, bool),
    UpToDate,
    InstallUpdate(Box<updater::Release>),
    UpdateReady(Box<updater::Finish>),
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
    compat: RefCell<compat::CompatState>,
    cheats: RefCell<cheats::CheatsState>,
    game_status: frame::GameStatus,
    focused: FocusedPane,
    error: Option<anyhow_ext::Error>,
    new_profile: Option<String>,
    edit_mod: Option<dialogs::ModDetailsEdit>,
    confirm: Option<(Message, String)>,
    busy: Cell<bool>,
    show_about: bool,
    show_licenses: bool,
    dirty: RwLock<HashMap<String, Manifest>>,
    saved_dirty: RefCell<HashMap<String, Manifest>>,
    options_mod: Option<(Mod, bool)>,
    temp_settings: Settings,
    toasts: egui_notify::Toasts,
    theme: swmm_widgets::visuals::Theme,
    intro: intro::Intro,
    theme_fade: Option<intro::ThemeFade>,
    changelog: Option<String>,
    crash: Option<Box<swmm_core::crash::CrashReport>>,
    conflicts: RefCell<conflicts::ConflictCache>,
    saves: RefCell<Option<(f64, Vec<swmm_core::saves::Backup>)>>,
    last_panic: Option<String>,
    pending_update: Option<Box<updater::Release>>,
    after_exit: Option<updater::Finish>,
}

static NO_CHANGES: std::sync::LazyLock<uk_mod::Manifest> = std::sync::LazyLock::new(Default::default);

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&'static str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "an unknown problem".to_string())
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Result<Self> {
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
        let core = Arc::new(Manager::init().context("SWMM couldn't load its settings and profiles")?);
        updater::clean_old_files();
        updater::migrate_preference(&core);
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
        updater::check_in_background(&core, send.clone());
        let temp_settings = core.settings().clone();
        let unapplied: HashMap<String, Manifest> = {
            let settings = core.settings();
            let dir = settings.profiles_dir();
            settings
                .profiles()
                .map(|p| {
                    let changes = swmm_core::mods::load_unapplied(&dir, p.as_str());
                    (p.as_str().into(), changes)
                })
                .collect()
        };
        LOCALIZATION.write().update_language(&temp_settings.lang);
        Ok(Self {
            selected: mods.first().cloned().into_iter().collect(),
            drag_index: None,
            hover_index: None,
            list_state: ui_state.list_state,
            profiles_state: RefCell::new(profiles::ProfileManagerState::new(&core)),
            meta_input: MetaInputModal::new(send.clone()),
            intro: intro::Intro::new(),
            theme_fade: None,
            displayed_mods,
            mod_list_filter: Default::default(),
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
            compat: Default::default(),
            cheats: Default::default(),
            game_status: frame::GameStatus::check(&core),
            focused: FocusedPane::None,
            error: None,
            new_profile: None,
            edit_mod: None,
            confirm: None,
            show_about: false,
            show_licenses: false,
            busy: Cell::new(false),
            dirty: RwLock::new(unapplied.clone()),
            saved_dirty: RefCell::new(unapplied),
            options_mod: None,
            crash: None,
            conflicts: Default::default(),
            saves: Default::default(),
            toasts: egui_notify::Toasts::new().with_anchor(egui_notify::Anchor::BottomRight),
            theme: ui_state.theme,
            install_queue: Default::default(),
            update_mod: Default::default(),
            error_queue: Default::default(),
            last_panic: None,
            pending_update: None,
            after_exit: None,
            core,
        })
    }

    #[inline(always)]
    fn platform(&self) -> Platform {
        self.core.settings().current_mode
    }

    #[inline(always)]
    fn dirty(&self) -> MappedRwLockReadGuard<'_, uk_mod::Manifest> {
        let key = self.core.mod_manager().profile().key().to_string();
        let dirty = self.dirty.read();
        RwLockReadGuard::map(dirty, |dirty| {
            dirty.get(key.as_str()).unwrap_or(&NO_CHANGES)
        })
    }

    #[inline(always)]
    fn dirty_mut(&self) -> MappedRwLockWriteGuard<'_, uk_mod::Manifest> {
        let key = self.core.mod_manager().profile().key().to_string();
        let dirty = self.dirty.write();
        RwLockWriteGuard::map(dirty, |dirty| dirty.entry(key.as_str().into()).or_default())
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
            || self.edit_mod.is_some()
            || self.meta_input.is_open()
            || self.changelog.is_some()
            || self.crash.is_some()
    }

    fn persist_dirty(&self) {
        let dirty = self.dirty.read();
        let mut saved = self.saved_dirty.borrow_mut();
        let dir = self.core.settings().profiles_dir();
        for (profile, changes) in dirty.iter() {
            if saved.get(profile) == Some(changes) {
                continue;
            }
            match swmm_core::mods::save_unapplied(&dir, profile, changes) {
                Ok(()) => {
                    saved.insert(profile.clone(), changes.clone());
                }
                Err(e) => log::warn!("Couldn't save the unapplied changes of {profile}: {e:?}"),
            }
        }
    }

    fn do_update(&self, message: Message) {
        if self.channel.0.send(message).is_err() {
            log::warn!("The message channel is closed");
        }
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
            sender.send(response).unwrap_or(());
        });
    }

    fn handle_drops(&mut self, ctx: &eframe::egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if !(self.modal_open() || files.is_empty()) {
            let Some(first) = files.first().and_then(|f| f.path.clone()) else {
                return;
            };
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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.draw(ctx, frame)));
        if let Err(payload) = result {
            let text = panic_text(payload.as_ref());
            log::error!("The window hit a problem and kept running: {text}");
            crate::logger::LOGGER.save_log();
            self.busy.set(false);
            if self.last_panic.as_deref() != Some(text.as_str()) {
                self.last_panic = Some(text.clone());
                self.do_update(Message::Error(anyhow::anyhow!(
                    "SWMM ran into a problem but kept running: {text}. Details are in the log."
                )));
            }
            ctx.request_repaint();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_ui_state();
        shutdown(&self.core);
        if let Some(finish) = self.after_exit.take() {
            finish.run();
        }
    }
}

impl App {
    fn release_memory(&self, ctx: &egui::Context) {
        mod_info::release_previews();
        self.browser.borrow_mut().release_images();
        self.cheats.borrow_mut().release_map();
        *self.conflicts.borrow_mut() = Default::default();
        ctx.forget_all_images();
        crate::release_allocator_memory();
        swmm_core::process::trim_memory();
        log::info!("Freed cached images and memory while Savage Wind runs");
    }

    fn draw(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        self.handle_update(ctx, frame);
        chrome::set_calm(jobs::game_running() || !ctx.input(|i| i.focused));
        fit_small_screen(ctx);
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let size = ctx.screen_rect().size();
        if minimized || size.x < 200.0 || size.y < 200.0 {
            return;
        }
        if !self.modal_open() && !self.intro.active() {
            if let Some(release) = self.pending_update.take() {
                self.ask_to_update(release);
            }
        }
        self.render_title_bar(ctx, frame);
        self.render_status_bar(ctx);
        self.render_nav(ctx);
        self.render_page(ctx);
        self.render_dialogs(ctx, frame);
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

    fn render_dialogs(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        chrome::paint_modal_dim(ctx, self.modal_open());
        self.render_error(ctx);
        self.render_confirm(ctx);
        self.render_new_profile(ctx);
        self.render_edit_mod(ctx);
        self.render_about(ctx);
        self.render_licenses(ctx);
        self.render_option_picker(ctx);
        self.render_changelog(ctx);
        self.render_crash(ctx);
        self.meta_input.ui(ctx);
        self.render_busy(ctx, frame);
        self.toasts.show(ctx);
    }
}

fn shutdown(core: &Manager) {
    crate::logger::LOGGER.save_log();
    core.settings_mut().last_version = Some(env!("CARGO_PKG_VERSION").into());
    core.settings().save().unwrap_or(());
    swmm_core::util::clear_temp();
}

const MIN_WINDOW: egui::Vec2 = egui::Vec2::new(900.0, 560.0);

fn fit_small_screen(ctx: &egui::Context) {
    let (monitor, inner) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().inner_rect));
    let Some(monitor) = monitor.filter(|m| m.x > 0.0 && m.y > 0.0) else { return };
    let zoom = ctx.zoom_factor();
    let base_id = egui::Id::new("swmm_base_zoom");
    let pending_id = egui::Id::new("swmm_zoom_pending");
    let base = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(base_id, || zoom.max(0.5)));
    if ctx.data(|d| d.get_temp::<egui::Vec2>(pending_id)) == Some(monitor) {
        return;
    }
    ctx.data_mut(|d| d.remove::<egui::Vec2>(pending_id));
    let unzoomed = monitor * zoom;
    let fit = (unzoomed.x / MIN_WINDOW.x).min(unzoomed.y / MIN_WINDOW.y);
    let target = fit.clamp(0.5, base);
    if (target - zoom).abs() > 0.01 {
        ctx.set_zoom_factor(target);
        ctx.data_mut(|d| d.insert_temp(pending_id, monitor));
        ctx.request_repaint();
        return;
    }
    let Some(inner) = inner else { return };
    let id = egui::Id::new("swmm_fitted_monitor");
    let fitted = ctx.data(|d| d.get_temp::<egui::Vec2>(id));
    if fitted != Some(monitor) && (inner.width() > monitor.x + 1.0 || inner.height() > monitor.y + 1.0) {
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_WINDOW.min(monitor)));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(inner.size().min(monitor)));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(0.0, 0.0)));
        ctx.data_mut(|d| d.insert_temp(id, monitor));
    }
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
                            .map(|i| i.to_rgba8().into_vec())
                            .unwrap_or_else(|_| vec![0; 512 * 512 * 4]),
                    }
                    .into(),
                ),
                title: Some("Savage Wind Mod Manager".into()),
                app_id: Some("swmm".into()),
                decorations: Some(false),
                transparent: Some(true),
                resizable: Some(true),
                min_inner_size: Some(MIN_WINDOW),
                inner_size: Some(egui::Vec2::new(1180.0, 740.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(|cc| {
            App::new(cc)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|e| format!("{e:?}").into())
        }),
    )
}
