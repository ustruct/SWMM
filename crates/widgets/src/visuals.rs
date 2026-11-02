use egui::{
    epaint::Shadow,
    style::{Selection, TextCursorStyle, WidgetVisuals, Widgets},
    vec2, Color32, FontFamily, Rounding, Stroke, Style, Visuals,
};
use serde::{Deserialize, Serialize};

pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xD4);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x60, 0xCD, 0xFF);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x00, 0x5A, 0x9E);
pub const GREEN: Color32 = Color32::from_rgb(0x6C, 0xCB, 0x5F);
pub const BLUE: Color32 = Color32::from_rgb(0x38, 0xB6, 0xF1);
pub const RED: Color32 = Color32::from_rgb(0xFF, 0x99, 0xA4);
pub const YELLOW: Color32 = Color32::from_rgb(0xFC, 0xE1, 0x00);
pub const ORANGE: Color32 = Color32::from_rgb(0xFF, 0x95, 0x3F);
pub const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub base: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent_text: Color32,
}

impl Theme {
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }

    pub fn iter() -> impl Iterator<Item = Self> {
        [Theme::Dark, Theme::Light].into_iter()
    }

    pub fn is_light(&self) -> bool {
        matches!(self, Theme::Light)
    }

    pub fn palette(&self) -> Palette {
        match self {
            Theme::Dark => Palette {
                base: Color32::from_rgb(0x0D, 0x0E, 0x10),
                panel: Color32::from_rgb(0x13, 0x14, 0x17),
                raised: Color32::from_rgb(0x19, 0x1B, 0x1E),
                hover: Color32::from_rgb(0x22, 0x25, 0x29),
                border: Color32::from_rgb(0x25, 0x28, 0x2D),
                text: Color32::from_rgb(0xE7, 0xE8, 0xEA),
                muted: Color32::from_rgb(0x86, 0x8B, 0x93),
                accent_text: ACCENT_LIGHT,
            },
            Theme::Light => Palette {
                base: Color32::from_rgb(0xF3, 0xF3, 0xF3),
                panel: Color32::from_rgb(0xF9, 0xF9, 0xF9),
                raised: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                hover: Color32::from_rgb(0xF0, 0xF0, 0xF0),
                border: Color32::from_rgb(0xE5, 0xE5, 0xE5),
                text: Color32::from_rgb(0x1B, 0x1B, 0x1B),
                muted: Color32::from_rgb(0x5D, 0x5D, 0x5D),
                accent_text: ACCENT_DARK,
            },
        }
    }

    pub fn style(&self) -> Style {
        let p = self.palette();
        let light = self.is_light();
        let rounding = Rounding::same(5.0);
        let widget = |bg: Color32, weak: Color32, stroke: Color32, fg: Color32| WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: weak,
            bg_stroke: Stroke::new(1.0_f32, stroke),
            rounding,
            fg_stroke: Stroke::new(1.0_f32, fg),
            expansion: 0.0,
        };
        let (control, control_hover, control_press) = if light {
            (
                Color32::from_rgb(0xFB, 0xFB, 0xFB),
                Color32::from_rgb(0xF5, 0xF5, 0xF5),
                Color32::from_rgb(0xEE, 0xEE, 0xEE),
            )
        } else {
            (
                Color32::from_rgb(0x1E, 0x21, 0x25),
                Color32::from_rgb(0x26, 0x29, 0x2E),
                Color32::from_rgb(0x18, 0x1A, 0x1D),
            )
        };
        let (check, check_hover) = if light {
            (Color32::from_rgb(0xE1, 0xE1, 0xE1), Color32::from_rgb(0xD6, 0xD6, 0xD6))
        } else {
            (Color32::from_rgb(0x2A, 0x2D, 0x33), Color32::from_rgb(0x33, 0x37, 0x3E))
        };
        let strong = if light { Color32::BLACK } else { Color32::WHITE };
        let mut visuals = if light { Visuals::light() } else { Visuals::dark() };
        visuals.widgets = Widgets {
            noninteractive: widget(p.raised, p.raised, p.border, p.text),
            inactive: widget(check, control, p.border, strong),
            hovered: widget(check_hover, control_hover, p.border, strong),
            active: WidgetVisuals {
                expansion: -1.0,
                ..widget(ACCENT, control_press, p.border, strong)
            },
            open: widget(p.raised, control_hover, p.border, strong),
        };
        visuals.selection = Selection {
            bg_fill: if light { ACCENT.linear_multiply(0.3) } else { ACCENT_DARK },
            stroke: Stroke::new(1.0_f32, p.text),
        };
        visuals.text_cursor = TextCursorStyle {
            stroke: Stroke::new(1.5_f32, p.accent_text),
            preview: false,
            ..Default::default()
        };
        visuals.hyperlink_color = p.accent_text;
        visuals.faint_bg_color = p.raised;
        visuals.extreme_bg_color = if light { Color32::WHITE } else { Color32::from_rgb(0x0A, 0x0B, 0x0D) };
        visuals.code_bg_color = if light {
            Color32::from_rgb(0xEA, 0xEA, 0xEA)
        } else {
            Color32::from_rgb(0x0F, 0x10, 0x12)
        };
        visuals.window_fill = if light { Color32::from_rgb(0xFB, 0xFB, 0xFB) } else { Color32::from_rgb(0x16, 0x18, 0x1B) };
        visuals.panel_fill = p.panel;
        visuals.window_stroke = Stroke::new(1.0_f32, p.border);
        visuals.window_rounding = Rounding::same(10.0);
        visuals.menu_rounding = Rounding::same(8.0);
        visuals.window_shadow = Shadow {
            offset: vec2(0.0, 10.0),
            blur: 32.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if light { 45 } else { 140 }),
        };
        visuals.popup_shadow = Shadow {
            offset: vec2(0.0, 6.0),
            blur: 16.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if light { 30 } else { 100 }),
        };
        visuals.warn_fg_color = if light { Color32::from_rgb(0x9D, 0x5D, 0x00) } else { YELLOW };
        visuals.error_fg_color = if light { Color32::from_rgb(0xC4, 0x2B, 0x1C) } else { RED };
        visuals.striped = false;
        visuals.slider_trailing_fill = true;
        visuals.handle_shape = egui::style::HandleShape::Circle;
        visuals.collapsing_header_frame = false;
        visuals.indent_has_left_vline = true;
        let mut style = Style {
            visuals,
            animation_time: 0.16,
            ..Default::default()
        };
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(11.0, 5.0);
        style.spacing.interact_size = vec2(32.0, 28.0);
        style.spacing.window_margin = egui::Margin::same(18.0);
        style.spacing.menu_margin = egui::Margin::same(6.0);
        style.spacing.icon_width = 16.0;
        style.spacing.icon_width_inner = 9.0;
        style.spacing.combo_width = 160.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(20.0, FontFamily::Name("Bold".into())),
        );
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
        style
    }

    pub fn set_theme(&self, ctx: &egui::Context) {
        ctx.set_style(self.style());
    }
}

pub fn palette(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode {
        Theme::Dark.palette()
    } else {
        Theme::Light.palette()
    }
}

pub fn ctx_palette(ctx: &egui::Context) -> Palette {
    if ctx.style().visuals.dark_mode {
        Theme::Dark.palette()
    } else {
        Theme::Light.palette()
    }
}

pub fn error_bg(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(0x2E, 0x15, 0x17)
    } else {
        Color32::from_rgb(0xFD, 0xE7, 0xE9)
    }
}
