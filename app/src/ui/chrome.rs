use std::f32::consts::TAU;

use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{
        emath::easing, pos2, vec2, Color32, CursorIcon, FontId, Order, PointerButton, Rect,
        ResizeDirection, Rounding, Sense, Shape, Stroke, ViewportCommand,
    },
    icons::{icon_family, icon_text_job},
    visuals::{self, Palette},
};

use super::*;

pub const TITLE_HEIGHT: f32 = 42.0;
pub const WINDOW_ROUNDING: f32 = if cfg!(windows) { 8.0 } else { 10.0 };
const CAPTION_WIDTH: f32 = 46.0;
const RESIZE_MARGIN: f32 = 5.0;

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

pub fn ease(t: f32) -> f32 {
    easing::cubic_out(t.clamp(0.0, 1.0))
}

pub fn now(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

pub fn appear(ctx: &egui::Context, id: Id, visible: bool, duration: f32) -> f32 {
    let key = id.with("appearSince");
    if !visible {
        ctx.data_mut(|d| d.remove::<f64>(key));
        return 0.0;
    }
    let time = now(ctx);
    let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || time));
    let t = ((time - since) as f32 / duration).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease(t)
}

pub fn content_fade(ctx: &egui::Context, id: Id, key: u64, duration: f32) -> f32 {
    let key_id = id.with("contentKey");
    let changed = ctx.data_mut(|d| {
        let previous = d.get_temp::<u64>(key_id);
        d.insert_temp(key_id, key);
        previous != Some(key)
    });
    if changed {
        appear(ctx, id, false, duration);
    }
    appear(ctx, id, true, duration)
}

pub fn window_in(ctx: &egui::Context, id: Id, duration: f32) -> f32 {
    let frame_id = id.with("windowInFrame");
    let frame = ctx.frame_nr();
    let last = ctx.data_mut(|d| {
        let last = d.get_temp::<u64>(frame_id);
        d.insert_temp(frame_id, frame);
        last
    });
    if last.map_or(true, |l| frame > l + 1) {
        appear(ctx, id.with("windowIn"), false, duration);
    }
    appear(ctx, id.with("windowIn"), true, duration)
}

fn inline_job(ui: &Ui, text: &str, size: f32, color: Color32, accent: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let (mut bold, mut italic, mut code) = (false, false, false);
    let mut buffer = std::string::String::new();
    let flush = |job: &mut egui::text::LayoutJob, buffer: &mut std::string::String, bold: bool, italic: bool, code: bool| {
        if buffer.is_empty() {
            return;
        }
        let family = if bold || code {
            egui::FontFamily::Name("Bold".into())
        } else {
            egui::FontFamily::Proportional
        };
        job.append(
            buffer,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::new(size, family),
                color: if code { accent } else { color },
                italics: italic,
                ..Default::default()
            },
        );
        buffer.clear();
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            flush(&mut job, &mut buffer, bold, italic, code);
            code = !code;
        } else if !code && c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut job, &mut buffer, bold, italic, code);
            bold = !bold;
            i += 1;
        } else if !code && (c == '*' || c == '_') {
            flush(&mut job, &mut buffer, bold, italic, code);
            italic = !italic;
        } else {
            buffer.push(c);
        }
        i += 1;
    }
    flush(&mut job, &mut buffer, bold, italic, code);
    let _ = ui;
    job
}

fn numbered(line: &str) -> Option<(&str, &str)> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    (digits > 0 && line[digits..].starts_with(". ")).then(|| (&line[..digits], line[digits + 2..].trim()))
}

pub fn rich_text(ui: &mut Ui, text: &str) {
    let palette = visuals::palette(ui);
    let width = ui.available_width();
    for block in text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() || block.starts_with("# ") {
            continue;
        }
        let lines: Vec<&str> = block.lines().collect();
        if lines.iter().all(|l| numbered(l).is_some()) {
            ui.spacing_mut().item_spacing.y = 6.0;
            for line in lines {
                let Some((number, body)) = numbered(line) else { continue };
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(24.0, 18.0), Sense::hover());
                    ui.painter().text(
                        rect.right_top() + Vec2::new(-6.0, 1.0),
                        Align2::RIGHT_TOP,
                        format!("{number}."),
                        egui::FontId::new(14.0, egui::FontFamily::Name("Bold".into())),
                        palette.accent_text,
                    );
                    let job = inline_job(ui, body, 14.0, palette.text, palette.accent_text, width - 32.0);
                    ui.add(egui::Label::new(job).wrap());
                });
            }
        } else {
            let job = inline_job(ui, &lines.join(" "), 14.0, palette.text, palette.accent_text, width);
            ui.add(egui::Label::new(job).wrap());
        }
        ui.add_space(10.0);
    }
}

pub fn clean_error(text: impl ToString) -> std::string::String {
    let mut text = text.to_string();
    while let Some(start) = text.find(" at `") {
        let Some(len) = text[start + 5..].find('`') else { break };
        let inner = &text[start + 5..start + 5 + len];
        if !(inner.contains(".rs@") || inner.contains(".rs:")) {
            break;
        }
        text.replace_range(start..start + 5 + len + 1, "");
    }
    text.trim().to_string()
}

pub fn progress_ring(ui: &mut Ui, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_ring(ui, rect, color);
    response
}

pub fn paint_ring(ui: &Ui, rect: Rect, color: Color32) {
    let time = ui.input(|i| i.time) as f32;
    ui.ctx().request_repaint();
    let thickness = (rect.width() / 10.0).max(2.0);
    let radius = rect.width() / 2.0 - thickness;
    let center = rect.center();
    ui.painter()
        .circle_stroke(center, radius, Stroke::new(thickness, color.gamma_multiply(0.15)));
    let sweep = 0.12 + 0.58 * (0.5 - 0.5 * (time * 2.4).cos());
    let start = time * 1.35 * TAU + (time * 1.2).sin() * 0.6;
    let steps = 40;
    let points = (0..=steps)
        .map(|i| {
            let a = start + sweep * TAU * i as f32 / steps as f32;
            center + radius * vec2(a.cos(), a.sin())
        })
        .collect();
    ui.painter().add(Shape::line(points, Stroke::new(thickness, color)));
}

pub fn indeterminate_bar(painter: &egui::Painter, rect: Rect, time: f64, color: Color32) {
    let width = rect.width();
    let segment = width * 0.28;
    let phase = ((time * 0.75) % 1.0) as f32;
    let left = rect.left() - segment + phase * (width + segment);
    let bar = Rect::from_min_max(
        pos2(left.max(rect.left()), rect.top()),
        pos2((left + segment).min(rect.right()), rect.bottom()),
    );
    if bar.width() > 0.0 {
        painter
            .with_clip_rect(rect)
            .rect_filled(bar, Rounding::same(rect.height() / 2.0), color);
    }
}

pub fn fancy_button(ui: &mut Ui, icon: Icon, text: &str, min_width: f32, height: f32) -> egui::Response {
    let enabled = ui.is_enabled();
    let job = icon_text_job(ui, icon, text, Some(Color32::WHITE));
    let galley = ui.fonts(|f| f.layout_job(job));
    let width = (galley.size().x + 28.0).max(min_width);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let palette = visuals::palette(ui);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() && enabled, 0.14);
    let pressed = response.is_pointer_button_down_on() && enabled;
    let shape_rect = if pressed { rect.shrink(1.0) } else { rect };
    let rounding = Rounding::same(6.0);
    let painter = ui.painter();
    if enabled && hover > 0.0 {
        let glow = egui::epaint::Shadow {
            offset: vec2(0.0, 3.0),
            blur: 14.0,
            spread: 0.0,
            color: visuals::ACCENT.gamma_multiply(0.4 * hover),
        };
        painter.add(glow.as_shape(shape_rect, rounding));
    }
    let fill = if enabled {
        mix(visuals::ACCENT, Color32::from_rgb(0x19, 0x8C, 0xE6), hover)
    } else {
        palette.hover
    };
    painter.rect_filled(shape_rect, rounding, fill);
    let text_color = if enabled { Color32::WHITE } else { palette.muted };
    let job = icon_text_job(ui, icon, text, Some(text_color));
    let galley = ui.fonts(|f| f.layout_job(job));
    painter.galley(shape_rect.center() - galley.size() / 2.0, galley, text_color);
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

fn caption_button(
    ui: &mut Ui,
    id: &str,
    close: bool,
    enabled: bool,
    paint: impl FnOnce(&egui::Painter, Rect, Stroke),
) -> egui::Response {
    let height = ui.available_height().max(TITLE_HEIGHT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(CAPTION_WIDTH, height), sense);
    let palette = visuals::palette(ui);
    let hover = ui.ctx().animate_bool_with_time(Id::new(id), enabled && response.hovered(), 0.1);
    if hover > 0.0 {
        let base = if close { visuals::CLOSE_RED } else { palette.hover };
        let fill = if response.is_pointer_button_down_on() {
            base.gamma_multiply(0.8)
        } else {
            base.gamma_multiply(hover)
        };
        let rounding = if close {
            Rounding {
                ne: corner(ui.ctx()),
                ..Rounding::ZERO
            }
        } else {
            Rounding::ZERO
        };
        ui.painter().rect_filled(rect, rounding, fill);
    }
    let glyph = if !enabled {
        palette.muted.gamma_multiply(0.45)
    } else if close {
        mix(palette.text, Color32::WHITE, hover)
    } else {
        palette.text
    };
    let glyph_rect = Rect::from_center_size(rect.center(), Vec2::splat(10.0));
    paint(ui.painter(), glyph_rect, Stroke::new(1.0_f32, glyph));
    response
}

pub fn window_controls(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.x = 0.0;
    let ctx = ui.ctx().clone();
    if caption_button(ui, "captionClose", true, true, |p, r, s| {
        p.line_segment([r.left_top(), r.right_bottom()], s);
        p.line_segment([r.right_top(), r.left_bottom()], s);
    })
    .clicked()
    {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    if caption_button(ui, "captionMaximize", false, true, |p, r, s| {
        if maximized || fullscreen {
            let front = Rect::from_min_size(r.left_top() + vec2(0.0, 2.0), Vec2::splat(8.0));
            p.rect_stroke(front, Rounding::same(1.5), s);
            p.line_segment([r.left_top() + vec2(2.0, 0.0), r.right_top()], s);
            p.line_segment([r.right_top(), r.right_top() + vec2(0.0, 8.0)], s);
        } else {
            p.rect_stroke(r, Rounding::same(1.5), s);
        }
    })
    .clicked()
    {
        if fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
    }
    if caption_button(ui, "captionMinimize", false, true, |p, r, s| {
        p.line_segment([r.left_center(), r.right_center()], s);
    })
    .clicked()
    {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[cfg(windows)]
pub fn round_window(cc: &eframe::CreationContext, border: Color32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const std::ffi::c_void, size: u32) -> i32;
    }

    const CORNER_PREFERENCE: u32 = 33;
    const BORDER_COLOR: u32 = 34;
    const ROUND: u32 = 2;
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get();
    let color = border.r() as u32 | (border.g() as u32) << 8 | (border.b() as u32) << 16;
    let rounded = unsafe { DwmSetWindowAttribute(hwnd, CORNER_PREFERENCE, (&ROUND as *const u32).cast(), 4) };
    unsafe {
        DwmSetWindowAttribute(hwnd, BORDER_COLOR, (&color as *const u32).cast(), 4);
    }
    if rounded < 0 {
        LEGACY_CORNERS.store(true, std::sync::atomic::Ordering::Relaxed);
        apply_round_region(hwnd);
    }
}

static LEGACY_CORNERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn legacy_corners() -> bool {
    LEGACY_CORNERS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct MonitorInfo {
    size: u32,
    monitor: WinRect,
    work: WinRect,
    flags: u32,
}

#[cfg(windows)]
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateRoundRectRgn(x1: i32, y1: i32, x2: i32, y2: i32, width: i32, height: i32) -> isize;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetWindowRect(hwnd: isize, rect: *mut WinRect) -> i32;
    fn SetWindowRgn(hwnd: isize, region: isize, redraw: i32) -> i32;
    fn IsZoomed(hwnd: isize) -> i32;
    fn GetDpiForWindow(hwnd: isize) -> u32;
    fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
    fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
}

#[cfg(windows)]
fn apply_round_region(hwnd: isize) {
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    unsafe {
        let mut rect = WinRect::default();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return;
        }
        let mut info = MonitorInfo {
            size: std::mem::size_of::<MonitorInfo>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let covers_monitor = monitor != 0 && GetMonitorInfoW(monitor, &mut info) != 0 && info.monitor == rect;
        if IsZoomed(hwnd) != 0 || covers_monitor {
            SetWindowRgn(hwnd, 0, 1);
            return;
        }
        let dpi = match GetDpiForWindow(hwnd) {
            0 => 96,
            dpi => dpi,
        };
        let diameter = ((WINDOW_ROUNDING * 2.0) * dpi as f32 / 96.0).round() as i32;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
        if region != 0 {
            SetWindowRgn(hwnd, region, 1);
        }
    }
}

#[cfg(windows)]
static ORIGINAL_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
static CLOSE_HOOK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
    fn CallWindowProcW(proc: isize, hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn IsIconic(hwnd: isize) -> i32;
    fn DefWindowProcW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
}

#[cfg(windows)]
unsafe extern "system" fn window_proc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
    const WM_CLOSE: u32 = 0x0010;
    if msg == WM_CLOSE && unsafe { IsIconic(hwnd) } != 0 {
        if let Some(hook) = CLOSE_HOOK.get() {
            hook();
        }
        std::process::exit(0);
    }
    let original = ORIGINAL_PROC.load(std::sync::atomic::Ordering::Relaxed);
    let result = if original == 0 {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    } else {
        unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) }
    };
    const WM_SIZE: u32 = 0x0005;
    const WM_DPICHANGED: u32 = 0x02E0;
    if (msg == WM_SIZE || msg == WM_DPICHANGED) && legacy_corners() {
        apply_round_region(hwnd);
    }
    result
}

#[cfg(windows)]
pub fn close_when_minimized(cc: &eframe::CreationContext, hook: impl Fn() + Send + Sync + 'static) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const GWLP_WNDPROC: i32 = -4;
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    if CLOSE_HOOK.set(Box::new(hook)).is_err() {
        return;
    }
    let hwnd = handle.hwnd.get();
    let proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize = window_proc;
    let original = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as isize) };
    ORIGINAL_PROC.store(original, std::sync::atomic::Ordering::Relaxed);
}

pub fn handle_resize(ctx: &egui::Context) {
    let (maximized, fullscreen) = ctx.input(|i| {
        (
            i.viewport().maximized.unwrap_or(false),
            i.viewport().fullscreen.unwrap_or(false),
        )
    });
    if maximized || fullscreen {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let rect = ctx.screen_rect();
    let left = pos.x <= rect.left() + RESIZE_MARGIN;
    let right = pos.x >= rect.right() - RESIZE_MARGIN;
    let top = pos.y <= rect.top() + RESIZE_MARGIN;
    let bottom = pos.y >= rect.bottom() - RESIZE_MARGIN;
    let direction = match (left, right, top, bottom) {
        (true, _, true, _) => Some((ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest)),
        (_, true, true, _) => Some((ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast)),
        (true, _, _, true) => Some((ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest)),
        (_, true, _, true) => Some((ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast)),
        (true, ..) => Some((ResizeDirection::West, CursorIcon::ResizeWest)),
        (_, true, ..) => Some((ResizeDirection::East, CursorIcon::ResizeEast)),
        (_, _, true, _) => Some((ResizeDirection::North, CursorIcon::ResizeNorth)),
        (.., true) => Some((ResizeDirection::South, CursorIcon::ResizeSouth)),
        _ => None,
    };
    if let Some((direction, cursor)) = direction {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

pub fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

pub fn corner(ctx: &egui::Context) -> f32 {
    if is_maximized(ctx) { 0.0 } else { WINDOW_ROUNDING }
}

pub fn toggle_fullscreen(ctx: &egui::Context) {
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
}

pub fn paint_window_border(ctx: &egui::Context, palette: &Palette) {
    if (cfg!(windows) && !legacy_corners()) || is_maximized(ctx) {
        return;
    }
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let color = if focused { palette.border } else { palette.border.gamma_multiply(0.6) };
    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("windowBorder")))
        .rect_stroke(ctx.screen_rect().shrink(0.5), Rounding::same(WINDOW_ROUNDING), Stroke::new(1.0_f32, color));
}

pub fn bottom_rounding(ctx: &egui::Context) -> Rounding {
    let r = corner(ctx);
    Rounding {
        sw: r,
        se: r,
        ..Rounding::ZERO
    }
}

pub fn top_rounding(ctx: &egui::Context) -> Rounding {
    let r = corner(ctx);
    Rounding {
        nw: r,
        ne: r,
        ..Rounding::ZERO
    }
}

pub fn paint_modal_dim(ctx: &egui::Context, open: bool) {
    let t = ctx.animate_bool_with_time(Id::new("modalDim"), open, 0.2);
    if t <= 0.0 {
        return;
    }
    let mut rect = ctx.screen_rect();
    rect.min.y += TITLE_HEIGHT;
    ctx.layer_painter(LayerId::new(Order::PanelResizeLine, Id::new("modalDim")))
        .rect_filled(rect, bottom_rounding(ctx), Color32::from_black_alpha((120.0 * t) as u8));
}

pub fn paint_drop_overlay(ctx: &egui::Context, active: bool) {
    let t = ctx.animate_bool_with_time(Id::new("dropOverlay"), active, 0.18);
    if t <= 0.0 {
        return;
    }
    let palette = visuals::ctx_palette(ctx);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("dropOverlay")));
    let mut screen = ctx.screen_rect();
    screen.min.y += TITLE_HEIGHT;
    painter.rect_filled(screen, bottom_rounding(ctx), palette.base.gamma_multiply(0.9 * t));
    let time = now(ctx);
    let inset = 18.0 + (1.0 - ease(t)) * 24.0;
    let zone = screen.shrink(inset);
    let r = zone.shrink(1.0);
    let points = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    let accent = palette.accent_text.gamma_multiply(t);
    painter.extend(Shape::dashed_line_with_offset(
        &points,
        Stroke::new(2.0_f32, accent),
        &[10.0],
        &[7.0],
        (time * 30.0) as f32 % 17.0,
    ));
    let bob = ((time * 3.5).sin() as f32) * 5.0;
    painter.text(
        zone.center() + vec2(0.0, -26.0 + bob),
        Align2::CENTER_CENTER,
        Icon::FileImport.glyph().to_string(),
        FontId::new(48.0, icon_family()),
        accent,
    );
    painter.text(
        zone.center() + vec2(0.0, 34.0),
        Align2::CENTER_CENTER,
        "SW_Drop_Here".localize(),
        FontId::new(20.0, egui::FontFamily::Name("Bold".into())),
        palette.text.gamma_multiply(t),
    );
    ctx.request_repaint();
}

impl App {
    pub fn render_title_bar(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let palette = self.theme.palette();
        egui::TopBottomPanel::top("titleBar")
            .exact_height(TITLE_HEIGHT)
            .show_separator_line(false)
            .frame(Frame::none().fill(palette.base).rounding(top_rounding(ctx)))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let drag = ui.interact(rect, Id::new("titleDrag"), Sense::click_and_drag());
                if drag.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started_by(PointerButton::Primary) {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.add_space(14.0);
                        frame::logo(ui, 18.0);
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new("Savage Wind Mod Manager")
                                    .family(egui::FontFamily::Name(swmm_widgets::TITLE_FAMILY.into()))
                                    .size(19.0)
                                    .color(palette.text),
                            )
                            .selectable(false),
                        );
                        ui.add_space(12.0);
                        let modal = self.modal_open();
                        ui.add_enabled_ui(!modal, |ui| self.render_menus(ui, frame));
                        ui.with_layout(Layout::right_to_left(Align::Center), window_controls);
                    });
                });
            });
    }
}

pub fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    icon: Icon,
    icon_color: Option<Color32>,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let palette = visuals::ctx_palette(ctx);
    let style = ctx.style();
    let t = window_in(ctx, Id::new(id), 0.24);
    egui::Window::new(title)
        .id(Id::new(id))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - t) * 16.0))
        .min_width(width)
        .max_width(width)
        .frame(
            Frame::window(&style)
                .inner_margin(Margin::same(22.0))
                .rounding(Rounding::same(12.0)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                if icon == Icon::Wind {
                    ui.add(super::frame::logo_image(ui, 18.0));
                } else {
                    let color = icon_color.unwrap_or(palette.accent_text);
                    ui.label(icon.rich(15.0).color(color));
                }
                ui.label(
                    RichText::new(title)
                        .family(egui::FontFamily::Name("Bold".into()))
                        .size(17.0)
                        .color(palette.text),
                );
            });
            ui.add_space(12.0);
            add(ui)
        })
        .and_then(|r| r.inner)
}

pub fn dialog_buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 30.0),
        Layout::right_to_left(Align::Center),
        add,
    );
}
