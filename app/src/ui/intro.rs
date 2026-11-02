use swmm_widgets::{
    egui::{self, pos2, vec2, Color32, FontFamily, FontId, Id, Order, Rect, Rounding, Sense},
    visuals::Palette,
};

use super::chrome;

const HOLD: f32 = 2.1;
const OUT: f32 = 0.5;
const SKIP_AFTER: f32 = 0.25;

pub struct Intro {
    start: Option<f64>,
    leave_at: Option<f32>,
    done: bool,
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

fn phase(t: f32, start: f32, length: f32) -> f32 {
    ((t - start) / length).clamp(0.0, 1.0)
}

impl Intro {
    pub fn new() -> Self {
        Self {
            start: None,
            leave_at: None,
            done: false,
        }
    }

    pub fn active(&self) -> bool {
        !self.done
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette, logo: egui::TextureId, logo_tint: Color32) {
        if self.done {
            return;
        }
        let time = ctx.input(|i| i.time);
        let start = *self.start.get_or_insert(time);
        let t = (time - start) as f32;

        let skip = ctx.input(|i| {
            i.pointer.any_click()
                || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        if skip && t > SKIP_AFTER && self.leave_at.is_none() && t < HOLD {
            self.leave_at = Some(t);
        }
        let leave = self.leave_at.unwrap_or(HOLD);
        let out = ease_in_out(phase(t, leave, OUT));
        if t >= leave + OUT {
            self.done = true;
            ctx.request_repaint();
            return;
        }
        ctx.request_repaint();

        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("swmmIntro"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .fade_in(false)
            .interactable(true)
            .show(ctx, |ui| {
                ui.allocate_rect(screen, Sense::click_and_drag());
                let painter = ui.painter_at(screen);
                let fade = 1.0 - out;
                painter.rect_filled(
                    screen,
                    Rounding::same(chrome::corner(ctx)),
                    palette.base.gamma_multiply(fade),
                );

                let unit = screen.width().min(screen.height());
                let logo_size = (unit * 0.24).clamp(110.0, 260.0);
                let title_size = (unit * 0.19).clamp(80.0, 200.0);
                let gap = unit * 0.035;
                let drift = -out * 10.0;

                let title_font = FontId::new(title_size, FontFamily::Name(swmm_widgets::TITLE_FAMILY.into()));
                let title = painter.layout_no_wrap("SWMM".to_string(), title_font, palette.text);
                let mut tag_job = egui::text::LayoutJob::default();
                tag_job.append(
                    "SAVAGE WIND MOD MANAGER",
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::proportional((title_size * 0.13).clamp(12.0, 22.0)),
                        color: palette.muted,
                        extra_letter_spacing: (title_size * 0.035).clamp(2.0, 6.0),
                        ..Default::default()
                    },
                );
                let tagline = painter.layout_job(tag_job);

                let block = logo_size + gap + title.size().y + gap * 0.6 + tagline.size().y;
                let top = screen.center().y - block / 2.0 + drift;

                let logo_in = ease_out(phase(t, 0.05, 0.75));
                let logo_rect = Rect::from_center_size(
                    pos2(screen.center().x, top + logo_size / 2.0 + (1.0 - logo_in) * 26.0),
                    egui::Vec2::splat(logo_size),
                );
                painter.image(
                    logo,
                    logo_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    logo_tint.gamma_multiply(logo_in * fade),
                );

                let title_pos = pos2(screen.center().x - title.size().x / 2.0, top + logo_size + gap);
                let wipe = ease_in_out(phase(t, 0.45, 0.75));
                if wipe > 0.0 {
                    let title_rect = Rect::from_min_size(title_pos, title.size());
                    let reveal = Rect::from_min_max(
                        title_rect.min - vec2(4.0, 4.0),
                        pos2(title_rect.left() + (title_rect.width() + 8.0) * wipe, title_rect.bottom() + 4.0),
                    );
                    let color = palette.text.gamma_multiply((0.35 + 0.65 * wipe) * fade);
                    let shaped = painter.layout_no_wrap(
                        "SWMM".to_string(),
                        FontId::new(title_size, FontFamily::Name(swmm_widgets::TITLE_FAMILY.into())),
                        color,
                    );
                    painter
                        .with_clip_rect(reveal.intersect(screen))
                        .galley(title_pos + vec2((1.0 - wipe) * -12.0, 0.0), shaped, color);
                }

                let tag_in = ease_out(phase(t, 1.05, 0.6));
                let mut tag_job = egui::text::LayoutJob::default();
                tag_job.append(
                    "SAVAGE WIND MOD MANAGER",
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::proportional((title_size * 0.13).clamp(12.0, 22.0)),
                        color: palette.muted.gamma_multiply(tag_in * fade),
                        extra_letter_spacing: (title_size * 0.035).clamp(2.0, 6.0),
                        ..Default::default()
                    },
                );
                let tag_shaped = painter.layout_job(tag_job);
                painter.galley(
                    pos2(
                        screen.center().x - tagline.size().x / 2.0,
                        title_pos.y + title.size().y + gap * 0.6 + (1.0 - tag_in) * 8.0,
                    ),
                    tag_shaped,
                    palette.muted,
                );
            });
    }
}

pub struct ThemeFade {
    color: Color32,
    since: f64,
}

impl ThemeFade {
    pub fn new(ctx: &egui::Context, color: Color32) -> Self {
        Self {
            color,
            since: ctx.input(|i| i.time),
        }
    }

    pub fn paint(&self, ctx: &egui::Context) -> bool {
        let t = ((ctx.input(|i| i.time) - self.since) as f32 / 0.35).clamp(0.0, 1.0);
        if t >= 1.0 {
            return false;
        }
        ctx.request_repaint();
        let alpha = 1.0 - ease_in_out(t);
        ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("themeFade"))).rect_filled(
            ctx.screen_rect(),
            Rounding::same(chrome::corner(ctx)),
            self.color.gamma_multiply(alpha),
        );
        true
    }
}
