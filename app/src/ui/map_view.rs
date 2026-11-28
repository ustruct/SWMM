use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use swmm_core::{
    map::{self, Image, Terrain},
    savage_wind,
    settings::Settings,
};
use swmm_i18n::string_ext::LocString;
use swmm_widgets::egui::{
    self, Align2, Color32, ColorImage, FontId, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::visuals;

const TILE_SIZE: usize = 300;
const MAX_ZOOM: f32 = 8.0;
const GROUND_CLEARANCE: f32 = 1.5;

type Loaded = Result<(Image, Terrain), String>;

pub struct MapState {
    romfs: Option<PathBuf>,
    pending: Option<Arc<Mutex<Option<Loaded>>>>,
    texture: Option<TextureHandle>,
    terrain: Option<Terrain>,
    error: Option<String>,
    zoom: f32,
    center: Vec2,
}

impl Default for MapState {
    fn default() -> Self {
        Self {
            romfs: None,
            pending: None,
            texture: None,
            terrain: None,
            error: None,
            zoom: 1.0,
            center: vec2(0.5, 0.5),
        }
    }
}

impl MapState {
    pub fn release(&mut self) {
        if self.pending.is_none() {
            self.texture = None;
            self.terrain = None;
            self.romfs = None;
        }
    }

    fn start(&mut self, romfs: Option<PathBuf>, ctx: &egui::Context) {
        *self = Self { romfs: romfs.clone(), ..Self::default() };
        let Some(romfs) = romfs else {
            self.error = Some("Cheats_Map_NoGame".localize().to_string());
            return;
        };
        let slot = Arc::new(Mutex::new(None));
        self.pending = Some(slot.clone());
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("swmm-map".into())
            .spawn(move || {
                let loaded = map::build(&romfs, TILE_SIZE)
                    .and_then(|image| Ok((image, Terrain::open(&romfs)?)))
                    .map_err(|e| e.to_string());
                if let Ok(mut slot) = slot.lock() {
                    *slot = Some(loaded);
                }
                ctx.request_repaint();
            })
            .ok();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(slot) = self.pending.as_ref() else {
            return;
        };
        let Some(loaded) = slot.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        self.pending = None;
        match loaded {
            Ok((image, terrain)) => {
                let color = ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba);
                self.texture = Some(ctx.load_texture("cheatsMap", color, TextureOptions::LINEAR));
                self.terrain = Some(terrain);
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn view(&self) -> Rect {
        let size = 1.0 / self.zoom;
        let min = vec2(
            (self.center.x - size / 2.0).clamp(0.0, 1.0 - size),
            (self.center.y - size / 2.0).clamp(0.0, 1.0 - size),
        );
        Rect::from_min_size(min.to_pos2(), vec2(size, size))
    }

    fn zoom_at(&mut self, factor: f32, anchor_uv: Vec2) {
        let old = self.zoom;
        self.zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let ratio = old / self.zoom;
        self.center = anchor_uv + (self.center - anchor_uv) * ratio;
        let half = 0.5 / self.zoom;
        self.center = vec2(self.center.x.clamp(half, 1.0 - half), self.center.y.clamp(half, 1.0 - half));
    }

    pub fn ground(&mut self, x: f32, z: f32) -> Option<f32> {
        self.terrain.as_mut()?.height_at(x, z).map(|h| h + GROUND_CLEARANCE)
    }
}

fn marker(painter: &egui::Painter, at: egui::Pos2, fill: Color32) {
    painter.circle(at, 6.0, fill, Stroke::new(2.0_f32, Color32::WHITE));
    painter.circle_stroke(at, 9.0, Stroke::new(1.0_f32, Color32::from_black_alpha(140)));
}

fn round_button(ui: &mut Ui, rect: Rect, text: &str, id: egui::Id) -> bool {
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if response.hovered() { Color32::from_black_alpha(200) } else { Color32::from_black_alpha(150) };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), Color32::WHITE);
    response.clicked()
}

pub fn show(
    ui: &mut Ui,
    state: &mut MapState,
    settings: &Settings,
    target: Option<[f32; 3]>,
    player: Option<[f32; 3]>,
) -> Option<[f32; 3]> {
    let palette = visuals::palette(ui);
    let romfs = savage_wind::current_game_files(settings).ok().map(|g| g.romfs);
    if state.romfs != romfs || (state.texture.is_none() && state.pending.is_none() && state.error.is_none()) {
        state.start(romfs, ui.ctx());
    }
    state.poll(ui.ctx());
    let width = ui.available_width();
    let height = width * map::WORLD_DEPTH / map::WORLD_WIDTH;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, palette.base);
    let Some(texture) = state.texture.clone() else {
        let text = match &state.error {
            Some(e) => e.clone(),
            None => "Cheats_Map_Loading".localize().to_string(),
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), palette.muted);
        if state.pending.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        return None;
    };
    let to_uv = |p: egui::Pos2, view: Rect| {
        let t = (p - rect.min) / rect.size();
        vec2(view.min.x + t.x * view.width(), view.min.y + t.y * view.height())
    };
    if response.hovered() {
        let wheel = ui.input_mut(|i| {
            let y = i.smooth_scroll_delta.y + i.smooth_scroll_delta.x;
            i.smooth_scroll_delta = Vec2::ZERO;
            y
        });
        let zoom = ui.input(|i| i.zoom_delta()) * (wheel / 200.0).exp();
        if zoom != 1.0
            && let Some(p) = response.hover_pos()
        {
            state.zoom_at(zoom, to_uv(p, state.view()));
        }
    }
    if response.dragged() {
        let delta = response.drag_delta() / rect.size() / state.zoom;
        let half = 0.5 / state.zoom;
        state.center = vec2(
            (state.center.x - delta.x).clamp(half, 1.0 - half),
            (state.center.y - delta.y).clamp(half, 1.0 - half),
        );
    }
    let view = state.view();
    painter.image(texture.id(), rect, view, Color32::WHITE);
    let to_screen = |x: f32, z: f32| {
        let (u, v) = map::world_to_map(x, z);
        let t = vec2((u - view.min.x) / view.width(), (v - view.min.y) / view.height());
        rect.min + t * rect.size()
    };
    if let Some([x, _, z]) = player {
        marker(&painter, to_screen(x, z), visuals::GREEN);
    }
    if let Some([x, _, z]) = target {
        let at = to_screen(x, z);
        painter.line_segment([at - vec2(12.0, 0.0), at + vec2(12.0, 0.0)], Stroke::new(1.5_f32, Color32::WHITE));
        painter.line_segment([at - vec2(0.0, 12.0), at + vec2(0.0, 12.0)], Stroke::new(1.5_f32, Color32::WHITE));
        marker(&painter, at, visuals::RED);
    }
    if let Some(p) = response.hover_pos() {
        let uv = to_uv(p, view);
        let (x, z) = map::map_to_world(uv.x, uv.y);
        let label = format!("X {x:.0}   Z {z:.0}");
        let at = pos2(rect.left() + 10.0, rect.bottom() - 10.0);
        let galley = painter.layout_no_wrap(label, FontId::monospace(12.0), Color32::WHITE);
        let back = Rect::from_min_size(at - vec2(4.0, galley.size().y + 2.0), galley.size() + vec2(8.0, 4.0));
        painter.rect_filled(back, 4.0, Color32::from_black_alpha(160));
        painter.galley(back.min + vec2(4.0, 2.0), galley, Color32::WHITE);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let button = vec2(28.0, 28.0);
    let plus = Rect::from_min_size(pos2(rect.right() - 38.0, rect.top() + 10.0), button);
    let minus = Rect::from_min_size(pos2(rect.right() - 38.0, rect.top() + 42.0), button);
    let center_uv = state.center;
    if round_button(ui, plus, "+", ui.id().with("mapZoomIn")) {
        state.zoom_at(1.5, center_uv);
    }
    if round_button(ui, minus, "\u{2212}", ui.id().with("mapZoomOut")) {
        state.zoom_at(1.0 / 1.5, center_uv);
    }
    let over_buttons = response.hover_pos().is_some_and(|p| plus.contains(p) || minus.contains(p));
    if response.clicked()
        && !over_buttons
        && let Some(p) = response.interact_pointer_pos()
    {
        let uv = to_uv(p, view);
        let (x, z) = map::map_to_world(uv.x, uv.y);
        let y = state.ground(x, z).unwrap_or(target.map(|t| t[1]).unwrap_or(300.0));
        return Some([x, y, z]);
    }
    None
}
