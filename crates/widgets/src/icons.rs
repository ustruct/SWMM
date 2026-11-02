use egui::{
    text::LayoutJob, Button, Color32, FontFamily, FontId, Response, RichText, TextFormat, Ui,
    WidgetText,
};

pub const ICON_FAMILY: &str = "Icons";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Add,
    Article,
    ArrowBack,
    ArrowUp,
    ArrowDown,
    Blank,
    Bolt,
    Book,
    Box,
    Cancel,
    Check,
    CircleCheck,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    Clock,
    CloudDownload,
    Compass,
    Copy,
    Delete,
    DoubleDown,
    DoubleUp,
    Download,
    Expand,
    Eye,
    EyeSlash,
    External,
    FileImport,
    FileUpload,
    Filter,
    Folder,
    FolderOpen,
    FolderZip,
    Gamepad,
    Gear,
    Globe,
    Hammer,
    Heart,
    Help,
    Home,
    Image,
    Import,
    Info,
    Language,
    Layers,
    List,
    ListCheck,
    Menu,
    Moon,
    Package,
    Palette,
    Pen,
    People,
    Play,
    Publish,
    Puzzle,
    Refresh,
    Reset,
    Rocket,
    Save,
    Search,
    Settings,
    Sliders,
    Spinner,
    Star,
    Sun,
    Tag,
    ThumbsUp,
    Tune,
    User,
    Warning,
    Wind,
}

impl Icon {
    pub fn glyph(self) -> char {
        match self {
            Icon::Add => '\u{2b}',
            Icon::Article => '\u{f15c}',
            Icon::ArrowBack => '\u{f053}',
            Icon::ArrowUp => '\u{f062}',
            Icon::ArrowDown => '\u{f063}',
            Icon::Blank => ' ',
            Icon::Bolt => '\u{f0e7}',
            Icon::Book => '\u{f02d}',
            Icon::Box => '\u{f49e}',
            Icon::Cancel => '\u{f00d}',
            Icon::Check => '\u{f00c}',
            Icon::CircleCheck => '\u{f058}',
            Icon::ChevronDown => '\u{f078}',
            Icon::ChevronLeft => '\u{f053}',
            Icon::ChevronRight => '\u{f054}',
            Icon::ChevronUp => '\u{f077}',
            Icon::Clock => '\u{f017}',
            Icon::CloudDownload => '\u{f0ed}',
            Icon::Compass => '\u{f14e}',
            Icon::Copy => '\u{f0c5}',
            Icon::Delete => '\u{f2ed}',
            Icon::DoubleDown => '\u{f103}',
            Icon::DoubleUp => '\u{f102}',
            Icon::Download => '\u{f019}',
            Icon::Expand => '\u{f065}',
            Icon::Eye => '\u{f06e}',
            Icon::EyeSlash => '\u{f070}',
            Icon::External => '\u{f35d}',
            Icon::FileImport => '\u{f56f}',
            Icon::FileUpload => '\u{f574}',
            Icon::Filter => '\u{f0b0}',
            Icon::Folder => '\u{f07b}',
            Icon::FolderOpen => '\u{f07c}',
            Icon::FolderZip => '\u{f1c6}',
            Icon::Gamepad => '\u{f11b}',
            Icon::Gear => '\u{f013}',
            Icon::Globe => '\u{f0ac}',
            Icon::Hammer => '\u{f6e3}',
            Icon::Heart => '\u{f004}',
            Icon::Help => '\u{f059}',
            Icon::Home => '\u{f015}',
            Icon::Image => '\u{f03e}',
            Icon::Import => '\u{f56f}',
            Icon::Info => '\u{f05a}',
            Icon::Language => '\u{f1ab}',
            Icon::Layers => '\u{f5fd}',
            Icon::List => '\u{f03a}',
            Icon::ListCheck => '\u{f14a}',
            Icon::Menu => '\u{f0c9}',
            Icon::Moon => '\u{f186}',
            Icon::Package => '\u{f187}',
            Icon::Palette => '\u{f53f}',
            Icon::Pen => '\u{f304}',
            Icon::People => '\u{f0c0}',
            Icon::Play => '\u{f04b}',
            Icon::Publish => '\u{f093}',
            Icon::Puzzle => '\u{f12e}',
            Icon::Refresh => '\u{f021}',
            Icon::Reset => '\u{f2ea}',
            Icon::Rocket => '\u{f135}',
            Icon::Save => '\u{f0c7}',
            Icon::Search => '\u{f002}',
            Icon::Settings => '\u{f0ae}',
            Icon::Sliders => '\u{f1de}',
            Icon::Spinner => '\u{f110}',
            Icon::Star => '\u{f005}',
            Icon::Sun => '\u{f185}',
            Icon::Tag => '\u{f02b}',
            Icon::ThumbsUp => '\u{f164}',
            Icon::Tune => '\u{f1de}',
            Icon::User => '\u{f007}',
            Icon::Warning => '\u{f071}',
            Icon::Wind => '\u{f72e}',
        }
    }

    pub fn rich(self, size: f32) -> RichText {
        RichText::new(self.glyph().to_string()).font(FontId::new(size, icon_family()))
    }
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name(ICON_FAMILY.into())
}

pub fn icon_label(icon: Icon, size: f32, color: Color32) -> RichText {
    icon.rich(size).color(color)
}

pub fn icon_text_job(ui: &Ui, icon: Icon, text: &str, color: Option<Color32>) -> LayoutJob {
    let color = color.unwrap_or_else(|| ui.visuals().widgets.inactive.fg_stroke.color);
    let size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .map(|f| f.size)
        .unwrap_or(14.0);
    let mut job = LayoutJob::default();
    job.append(
        &icon.glyph().to_string(),
        0.0,
        TextFormat {
            font_id: FontId::new(size - 1.0, icon_family()),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    if !text.is_empty() {
        job.append(
            text,
            8.0,
            TextFormat {
                font_id: FontId::proportional(size),
                color,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
    }
    job
}

pub trait IconButtonExt {
    fn icon_button(&mut self, icon: Icon) -> Response;
    fn icon_text_button(&mut self, text: impl Into<WidgetText>, icon: Icon) -> Response;
}

impl IconButtonExt for Ui {
    fn icon_button(&mut self, icon: Icon) -> Response {
        let size = self
            .style()
            .text_styles
            .get(&egui::TextStyle::Button)
            .map(|f| f.size)
            .unwrap_or(14.0);
        self.add(Button::new(icon.rich(size)).min_size([30.0, 26.0].into()))
    }

    fn icon_text_button(&mut self, text: impl Into<WidgetText>, icon: Icon) -> Response {
        let text: WidgetText = text.into();
        let job = icon_text_job(self, icon, text.text(), None);
        self.add(Button::new(job).min_size([0.0, 26.0].into()))
    }
}
