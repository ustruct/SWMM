use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use flume::{Receiver, Sender};
use serde_json::Value;
use swmm_i18n::string_ext::LocString;
use swmm_widgets::{
    egui::{
        self, load::SizedTexture, Align, Color32, ColorImage, Layout, Margin, RichText, ScrollArea,
        Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2,
    },
    icons::{Icon, IconButtonExt},
    visuals,
};

use super::{chrome, App, Message};

const GAME_ID: u64 = 6386;
const PER_PAGE: u32 = 20;
const API: &str = "https://gamebanana.com/apiv11";
const MIN_CARD_WIDTH: f32 = 200.0;
const CARD_GAP: f32 = 14.0;
const CARD_TEXT_HEIGHT: f32 = 80.0;
const DETAILS_WIDTH: f32 = 330.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseSort {
    Popular,
    Newest,
    Updated,
}

impl BrowseSort {
    fn key(self) -> &'static str {
        match self {
            BrowseSort::Popular => "Browse_Sort_Popular",
            BrowseSort::Newest => "Browse_Sort_Newest",
            BrowseSort::Updated => "Browse_Sort_Updated",
        }
    }

    fn api(self) -> &'static str {
        match self {
            BrowseSort::Popular => "default",
            BrowseSort::Newest => "new",
            BrowseSort::Updated => "updated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowseMod {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub category: String,
    pub likes: u64,
    pub views: u64,
    pub thumbnail: Option<String>,
    pub nsfw: bool,
}

#[derive(Debug, Clone)]
pub struct BrowseFile {
    pub name: String,
    pub size: u64,
    pub description: String,
    pub downloads: u64,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct BrowseDetails {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub text: String,
    pub files: Vec<BrowseFile>,
    pub url: String,
    pub nsfw: bool,
}

enum BrowseEvent {
    Page(u64, Result<(Vec<BrowseMod>, bool)>),
    Details(Result<BrowseDetails>),
    Thumbnail(u64, Option<(ColorImage, ColorImage)>),
    Downloaded(Result<PathBuf>),
}

pub struct BrowserState {
    query: String,
    sort: BrowseSort,
    page: u32,
    results: Vec<BrowseMod>,
    has_more: bool,
    loading: bool,
    error: Option<String>,
    request: u64,
    details: Option<BrowseDetails>,
    details_loading: bool,
    last_details: Option<BrowseDetails>,
    downloading: Option<String>,
    download_done: Arc<AtomicU64>,
    download_total: u64,
    download_started: Option<Instant>,
    thumbnails: HashMap<u64, Option<TextureHandle>>,
    blurred: HashMap<u64, TextureHandle>,
    revealed: HashSet<u64>,
    loaded_once: bool,
    tx: Sender<BrowseEvent>,
    rx: Receiver<BrowseEvent>,
}

impl Default for BrowserState {
    fn default() -> Self {
        let (tx, rx) = flume::unbounded();
        Self {
            query: String::new(),
            sort: BrowseSort::Popular,
            page: 1,
            results: Vec::new(),
            has_more: false,
            loading: false,
            error: None,
            request: 0,
            details: None,
            details_loading: false,
            last_details: None,
            downloading: None,
            download_done: Arc::new(AtomicU64::new(0)),
            download_total: 0,
            download_started: None,
            thumbnails: HashMap::new(),
            blurred: HashMap::new(),
            revealed: HashSet::new(),
            loaded_once: false,
            tx,
            rx,
        }
    }
}

const CACHE_TIME: Duration = Duration::from_secs(15 * 60);
const COOLDOWN: Duration = Duration::from_secs(5 * 60);

type PageResult = (Vec<BrowseMod>, bool);

static PAGE_CACHE: LazyLock<Mutex<HashMap<String, (Instant, PageResult)>>> = LazyLock::new(Default::default);
static DETAILS_CACHE: LazyLock<Mutex<HashMap<u64, (Instant, BrowseDetails)>>> = LazyLock::new(Default::default);
static PAUSED_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

fn cached<K: std::hash::Hash + Eq, V: Clone>(cache: &Mutex<HashMap<K, (Instant, V)>>, key: &K) -> Option<V> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, (time, _)| time.elapsed() < CACHE_TIME);
    cache.get(key).map(|(_, value)| value.clone())
}

fn remember<K: std::hash::Hash + Eq, V>(cache: &Mutex<HashMap<K, (Instant, V)>>, key: K, value: V) {
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, (Instant::now(), value));
}

fn paused_message() -> Option<std::string::String> {
    let until = (*PAUSED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()))?;
    let left = until.checked_duration_since(Instant::now())?;
    let minutes = left.as_secs().div_ceil(60).max(1);
    Some(format!(
        "{} ({minutes} min)",
        "Browse_RateLimited".localize()
    ))
}

fn get_json(url: &str) -> Result<Value> {
    if let Some(message) = paused_message() {
        anyhow::bail!(message);
    }
    let (status, bytes) =
        super::net::response_with_status(url).context("Could not reach GameBanana")?;
    if status == 429 || status == 503 {
        *PAUSED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + COOLDOWN);
        log::warn!("GameBanana answered HTTP {status}, pausing requests for {} minutes", COOLDOWN.as_secs() / 60);
        anyhow::bail!(paused_message().unwrap_or_else(|| "Browse_RateLimited".localize().to_string()));
    }
    if !(200..300).contains(&status) {
        anyhow::bail!("GameBanana answered with HTTP {status}");
    }
    serde_json::from_slice(&bytes).context("GameBanana sent an unexpected answer")
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn text_of(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn number_of(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or_default()
}

fn thumbnail_of(value: &Value) -> Option<String> {
    let image = value.get("_aPreviewMedia")?.get("_aImages")?.as_array()?.first()?;
    let base = image.get("_sBaseUrl")?.as_str()?;
    let file = image
        .get("_sFile220")
        .or_else(|| image.get("_sFile530"))
        .or_else(|| image.get("_sFile"))?
        .as_str()?;
    Some(format!("{base}/{file}"))
}

fn parse_mod(value: &Value) -> Option<BrowseMod> {
    if value.get("_sModelName").and_then(Value::as_str).is_some_and(|m| m != "Mod") {
        return None;
    }
    let id = number_of(value, "_idRow");
    if id == 0 {
        return None;
    }
    Some(BrowseMod {
        id,
        name: text_of(value, "_sName"),
        author: value
            .get("_aSubmitter")
            .map(|s| text_of(s, "_sName"))
            .unwrap_or_default(),
        category: value
            .get("_aRootCategory")
            .map(|c| text_of(c, "_sName"))
            .unwrap_or_default(),
        likes: number_of(value, "_nLikeCount"),
        views: number_of(value, "_nViewCount"),
        thumbnail: thumbnail_of(value),
        nsfw: is_nsfw(value),
    })
}

fn fetch_page(query: &str, sort: BrowseSort, page: u32) -> Result<(Vec<BrowseMod>, bool)> {
    let key = format!("{}|{}|{page}", query.trim().to_lowercase(), sort.key());
    if let Some(result) = cached(&*PAGE_CACHE, &key) {
        return Ok(result);
    }
    let result = fetch_page_uncached(query, sort, page)?;
    remember(&*PAGE_CACHE, key, result.clone());
    Ok(result)
}

fn fetch_page_uncached(query: &str, sort: BrowseSort, page: u32) -> Result<(Vec<BrowseMod>, bool)> {
    let mut search = query.trim().to_string();
    if !search.to_lowercase().contains("switch") {
        if !search.is_empty() {
            search.push(' ');
        }
        search.push_str("switch");
    }
    let url = if search.is_empty() {
        format!(
            "{API}/Game/{GAME_ID}/Subfeed?_nPage={page}&_nPerpage={PER_PAGE}&_sSort={}&_csvModelInclusions=Mod",
            sort.api()
        )
    } else {
        format!(
            "{API}/Util/Search/Results?_sModelName=Mod&_sOrder={}&_idGameRow={GAME_ID}&_sSearchString={}&_nPage={page}&_nPerpage={PER_PAGE}",
            match sort {
                BrowseSort::Popular => "best_match",
                BrowseSort::Newest | BrowseSort::Updated => "date",
            },
            encode(&search)
        )
    };
    let json = get_json(&url)?;
    let records = json
        .get("_aRecords")
        .and_then(Value::as_array)
        .context("GameBanana sent no mods")?;
    let mods: Vec<BrowseMod> = records.iter().filter_map(parse_mod).collect();
    let complete = json
        .get("_aMetadata")
        .and_then(|m| m.get("_bIsComplete"))
        .and_then(Value::as_bool)
        .unwrap_or(records.len() < PER_PAGE as usize);
    Ok((mods, !complete))
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_lowercase();
                if matches!(name.as_str(), "br" | "br/" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "tr") {
                    out.push('\n');
                    if name == "li" && !tag.starts_with('/') {
                        out.push_str("• ");
                    }
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    cleaned.trim().to_string()
}

fn fetch_details(id: u64) -> Result<BrowseDetails> {
    if let Some(details) = cached(&*DETAILS_CACHE, &id) {
        return Ok(details);
    }
    let details = fetch_details_uncached(id)?;
    remember(&*DETAILS_CACHE, id, details.clone());
    Ok(details)
}

fn fetch_details_uncached(id: u64) -> Result<BrowseDetails> {
    let json = get_json(&format!("{API}/Mod/{id}/ProfilePage"))?;
    let files = json
        .get("_aFiles")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .map(|f| BrowseFile {
                    name: text_of(f, "_sFile"),
                    size: number_of(f, "_nFilesize"),
                    description: text_of(f, "_sDescription"),
                    downloads: number_of(f, "_nDownloadCount"),
                    url: {
                        let url = text_of(f, "_sDownloadUrl");
                        if url.is_empty() {
                            format!("https://gamebanana.com/dl/{}", number_of(f, "_idRow"))
                        } else {
                            url
                        }
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(BrowseDetails {
        id,
        name: text_of(&json, "_sName"),
        author: json
            .get("_aSubmitter")
            .map(|s| text_of(s, "_sName"))
            .unwrap_or_default(),
        text: strip_html(&text_of(&json, "_sText")),
        files,
        url: format!("https://gamebanana.com/mods/{id}"),
        nsfw: is_nsfw(&json),
    })
}

const NSFW_RATINGS: &[&str] = &["st", "sa", "sc", "ft", "lp", "pn", "nu"];

fn is_nsfw(value: &Value) -> bool {
    let hidden = value
        .get("_sInitialVisibility")
        .and_then(Value::as_str)
        .is_some_and(|v| matches!(v, "hide" | "warn"));
    let rated = value
        .get("_aContentRatings")
        .and_then(Value::as_object)
        .is_some_and(|ratings| ratings.keys().any(|k| NSFW_RATINGS.contains(&k.as_str())));
    hidden || rated
}

fn to_color_image(image: &image::RgbaImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, image.as_raw())
}

fn blur_image(image: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (image.width().max(1), image.height().max(1));
    let small_w = 40.min(w);
    let small_h = ((h as f32 * small_w as f32 / w as f32).round() as u32).max(1);
    let small = image::imageops::resize(image, small_w, small_h, image::imageops::FilterType::Triangle);
    let soft = image::imageops::blur(&small, 2.5);
    image::imageops::resize(&soft, (small_w * 4).min(w), (small_h * 4).min(h), image::imageops::FilterType::Triangle)
}

fn fetch_thumbnail(url: &str) -> Option<(ColorImage, ColorImage)> {
    let bytes = super::net::response(url).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    Some((to_color_image(&image), to_color_image(&blur_image(&image))))
}

fn download(file: BrowseFile, done: &AtomicU64) -> Result<PathBuf> {
    log::info!("Downloading {} from GameBanana", file.name);
    let name: String = file
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let path = swmm_core::util::get_temp_file()
        .with_file_name(if name.is_empty() { "download.zip".to_string() } else { name });
    super::net::download_file(&file.url, &path, done).with_context(|| format!("Could not download {}", file.name))?;
    log::info!("Downloaded {}", path.display());
    Ok(path)
}

fn render_download_progress(ui: &mut Ui, state: &BrowserState) {
    let palette = visuals::palette(ui);
    let done = state.download_done.load(Ordering::Relaxed);
    let total = state.download_total.max(done);
    let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
    let elapsed = state.download_started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let speed = if elapsed > 0.5 { done as f64 / elapsed } else { 0.0 };
    let finishing = total > 0 && done >= total;

    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    let shown = ui.ctx().animate_value_with_time(egui::Id::new("downloadBar"), fraction, 0.25);
    ui.painter().rect_filled(rect, 4.0, palette.border);
    if finishing || total == 0 {
        let width = rect.width() * 0.3;
        let t = (ui.input(|i| i.time) * 0.8).fract() as f32;
        let left = rect.left() - width + (rect.width() + width) * t;
        let bar = egui::Rect::from_x_y_ranges(left.max(rect.left())..=(left + width).min(rect.right()), rect.y_range());
        ui.painter().rect_filled(bar, 4.0, palette.accent_text);
    } else if shown > 0.0 {
        let bar = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * shown.clamp(0.0, 1.0), rect.height()));
        ui.painter().rect_filled(bar, 4.0, palette.accent_text);
    }
    ui.add_space(6.0);
    let status = if finishing {
        "Browse_Installing".localize().to_string()
    } else if total > 0 {
        let mut text = format!("{} / {}  ·  {:.0}%", format_size(done), format_size(total), fraction * 100.0);
        if speed > 0.0 {
            text.push_str(&format!("  ·  {}/s", format_size(speed as u64)));
            let remaining = (total - done) as f64 / speed;
            if remaining.is_finite() && remaining >= 1.0 {
                let secs = remaining.round() as u64;
                text.push_str(&format!("  ·  {}:{:02}", secs / 60, secs % 60));
            }
        }
        text
    } else {
        format!("{}", format_size(done))
    };
    ui.label(RichText::new(status).size(12.0).color(palette.muted));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
}

fn format_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

fn format_count(count: u64) -> String {
    match count {
        c if c >= 1_000_000 => format!("{:.1}M", c as f64 / 1_000_000.0),
        c if c >= 1_000 => format!("{:.1}k", c as f64 / 1_000.0),
        c => c.to_string(),
    }
}

impl BrowserState {
    fn reload(&mut self) {
        self.request += 1;
        let request = self.request;
        self.loading = true;
        self.error = None;
        let (query, sort, page) = (self.query.clone(), self.sort, self.page);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = fetch_page(&query, sort, page);
            tx.send(BrowseEvent::Page(request, result)).unwrap_or(());
        });
    }

    fn open_details(&mut self, id: u64) {
        self.details_loading = true;
        self.details = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            tx.send(BrowseEvent::Details(fetch_details(id))).unwrap_or(());
        });
    }

    fn request_thumbnail(&mut self, item: &BrowseMod) {
        if self.thumbnails.contains_key(&item.id) {
            return;
        }
        self.thumbnails.insert(item.id, None);
        if let Some(url) = item.thumbnail.clone() {
            let (tx, id) = (self.tx.clone(), item.id);
            std::thread::spawn(move || {
                tx.send(BrowseEvent::Thumbnail(id, fetch_thumbnail(&url))).unwrap_or(());
            });
        }
    }

    fn poll(&mut self, ctx: &egui::Context, sender: &Sender<Message>) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                BrowseEvent::Page(request, result) if request == self.request => {
                    self.loading = false;
                    match result {
                        Ok((mods, more)) => {
                            self.results = mods;
                            self.has_more = more;
                        }
                        Err(e) => {
                            log::warn!("Mod browser: {e:?}");
                            self.error = Some(e.to_string());
                        }
                    }
                }
                BrowseEvent::Page(..) => (),
                BrowseEvent::Details(result) => {
                    self.details_loading = false;
                    match result {
                        Ok(details) => self.details = Some(details),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                BrowseEvent::Thumbnail(id, images) => {
                    let texture = images.map(|(clear, blurred)| {
                        self.blurred.insert(
                            id,
                            ctx.load_texture(format!("browse-blur-{id}"), blurred, TextureOptions::LINEAR),
                        );
                        ctx.load_texture(format!("browse-thumb-{id}"), clear, TextureOptions::LINEAR)
                    });
                    self.thumbnails.insert(id, texture);
                }
                BrowseEvent::Downloaded(result) => {
                    self.downloading = None;
                    match result {
                        Ok(path) => sender.send(Message::OpenMod(path)).unwrap_or(()),
                        Err(e) => sender.send(Message::Error(e.into())).unwrap_or(()),
                    }
                }
            }
            ctx.request_repaint();
        }
    }

    fn start_download(&mut self, file: BrowseFile) {
        self.downloading = Some(file.name.clone());
        self.download_done.store(0, Ordering::Relaxed);
        self.download_total = file.size;
        self.download_started = Some(Instant::now());
        let tx = self.tx.clone();
        let done = self.download_done.clone();
        std::thread::spawn(move || {
            tx.send(BrowseEvent::Downloaded(download(file, &done))).unwrap_or(());
        });
    }
}

impl App {
    pub fn render_browser(&mut self, ui: &mut Ui) {
        let sender = self.channel.0.clone();
        let mut state = self.browser.borrow_mut();
        state.poll(ui.ctx(), &sender);
        if !state.loaded_once {
            state.loaded_once = true;
            state.reload();
        }
        let palette = visuals::palette(ui);
        let mut changed = false;
        ui.horizontal(|ui| {
            let filter_width = 150.0;
            let refresh_width = 34.0;
            let search_width =
                (ui.available_width() - filter_width - refresh_width - 2.0 * ui.spacing().item_spacing.x).max(140.0);
            egui::Frame::none()
                .fill(palette.raised)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .rounding(8.0)
                .inner_margin(Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(search_width - 22.0);
                    ui.horizontal(|ui| {
                        ui.label(Icon::Search.rich(12.0).color(palette.muted));
                        let search = ui.add(
                            egui::TextEdit::singleline(&mut state.query)
                                .frame(false)
                                .hint_text("Browse_Search".localize())
                                .desired_width(f32::INFINITY),
                        );
                        if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            changed = true;
                        }
                    });
                });
            let current = state.sort;
            let label = swmm_widgets::icons::icon_text_job(ui, Icon::Filter, &current.key().localize(), None);
            let button = ui.add(egui::Button::new(label).min_size(Vec2::new(filter_width, 32.0)));
            let popup = ui.make_persistent_id("browseFilters");
            if button.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup::popup_below_widget(ui, popup, &button, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(filter_width + 40.0);
                ui.label(RichText::new("Browse_Sort".localize()).size(12.0).color(palette.muted));
                ui.add_space(2.0);
                for sort in [BrowseSort::Popular, BrowseSort::Newest, BrowseSort::Updated] {
                    let picked = sort == current;
                    let job = swmm_widgets::icons::icon_text_job(
                        ui,
                        if picked { Icon::Check } else { Icon::Blank },
                        &sort.key().localize(),
                        Some(if picked { palette.text } else { palette.muted }),
                    );
                    if ui
                        .add(egui::Button::new(job).frame(false).min_size(Vec2::new(filter_width + 40.0, 28.0)))
                        .clicked()
                        && !picked
                    {
                        state.sort = sort;
                        changed = true;
                    }
                }
            });
            if ui
                .add(egui::Button::new(Icon::Refresh.rich(13.0)).min_size(Vec2::new(refresh_width, 32.0)))
                .on_hover_text("Browse_Refresh".localize())
                .clicked()
            {
                changed = true;
            }
        });
        if changed {
            state.page = 1;
            state.reload();
        }
        ui.add_space(14.0);
        let show_details = state.details.is_some() || state.details_loading;
        if state.details.is_some() {
            state.last_details = state.details.clone();
        }
        let open = ui.ctx().animate_bool_with_time(egui::Id::new("browseDetailsOpen"), show_details, 0.3);
        let eased = chrome::ease(open);
        if open <= 0.0 {
            state.last_details = None;
        } else {
            egui::SidePanel::right("browseDetails")
                .resizable(false)
                .exact_width((DETAILS_WIDTH * eased).max(1.0))
                .show_separator_line(false)
                .frame(egui::Frame::none())
                .show_inside(ui, |ui| {
                    let panel = ui.max_rect();
                    let content = egui::Rect::from_min_size(
                        egui::pos2(panel.left() + 22.0, panel.top()),
                        Vec2::new(DETAILS_WIDTH - 22.0, panel.height()),
                    );
                    ui.allocate_ui_at_rect(content, |ui| {
                        ui.set_clip_rect(panel.intersect(ui.clip_rect()));
                        ui.multiply_opacity(eased);
                        let closing = !show_details;
                        if closing {
                            state.details = state.last_details.clone();
                        }
                        render_details(ui, &mut state);
                        if closing {
                            state.details = None;
                        }
                    });
                });
        }
        egui::TopBottomPanel::bottom("browsePages")
            .show_separator_line(false)
            .frame(egui::Frame::none().inner_margin(Margin {
                top: 12.0,
                ..Default::default()
            }))
            .show_inside(ui, |ui| {
                let row = ui.available_rect_before_wrap();
                let row = egui::Rect::from_min_size(row.min, Vec2::new(row.width(), 30.0));
                ui.allocate_ui_at_rect(row, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(RichText::new("Browse_Source".localize()).size(12.0).color(palette.muted));
                    });
                });
                let center = egui::Rect::from_center_size(row.center(), Vec2::new(150.0, 30.0));
                ui.allocate_ui_at_rect(center, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let page = state.page;
                        if ui
                            .add_enabled(
                                page > 1 && !state.loading,
                                egui::Button::new(Icon::ChevronLeft.rich(12.0)).min_size(Vec2::new(30.0, 28.0)),
                            )
                            .clicked()
                        {
                            state.page -= 1;
                            state.reload();
                        }
                        ui.add_sized(
                            Vec2::new(70.0, 28.0),
                            egui::Label::new(
                                RichText::new(format!("{} {}", "Browse_Page".localize(), page)).color(palette.text),
                            ),
                        );
                        if ui
                            .add_enabled(
                                state.has_more && !state.loading,
                                egui::Button::new(Icon::ChevronRight.rich(12.0)).min_size(Vec2::new(30.0, 28.0)),
                            )
                            .clicked()
                        {
                            state.page += 1;
                            state.reload();
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show_inside(ui, |ui| {
                if state.loading && state.results.is_empty() {
                    ui.centered_and_justified(|ui| {
                        chrome::progress_ring(ui, 28.0, palette.accent_text);
                    });
                    return;
                }
                if let Some(error) = state.error.clone() {
                    ui.horizontal(|ui| {
                        ui.label(Icon::Warning.rich(14.0).color(ui.visuals().warn_fg_color));
                        ui.label(RichText::new(error).color(ui.visuals().warn_fg_color));
                    });
                    ui.add_space(6.0);
                }
                if state.results.is_empty() && !state.loading {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Browse_Empty".localize()).color(palette.muted));
                    });
                    return;
                }
                let results = state.results.clone();
                let selected = state.details.as_ref().map(|d| d.id);
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                ui.spacing_mut().scroll.bar_width = 8.0;
                ScrollArea::vertical().drag_to_scroll(false)
                    .id_source("browseScroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        let available = ui.available_width() - 6.0;
                        let columns = (((available + CARD_GAP) / (MIN_CARD_WIDTH + CARD_GAP)).floor() as usize).max(1);
                        let card_width = ((available - CARD_GAP * (columns - 1) as f32) / columns as f32).floor();
                        let card_size = Vec2::new(card_width, (card_width * 9.0 / 16.0).round() + CARD_TEXT_HEIGHT);
                        ui.add_space(4.0);
                        for chunk in results.chunks(columns) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = CARD_GAP;
                                for item in chunk {
                                    state.request_thumbnail(item);
                                    let hidden = item.nsfw && !state.revealed.contains(&item.id);
                                    let texture = if hidden {
                                        state.thumbnails.get(&item.id).cloned().flatten().and(state.blurred.get(&item.id).cloned())
                                    } else {
                                        state.thumbnails.get(&item.id).cloned().flatten()
                                    };
                                    if render_card(ui, item, texture.as_ref(), hidden, selected == Some(item.id), card_size) {
                                        state.open_details(item.id);
                                    }
                                }
                            });
                            ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
                        }
                        if state.loading {
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                chrome::progress_ring(ui, 22.0, palette.accent_text);
                            });
                        }
                    });
            });
    }
}

fn paint_cover(ui: &Ui, texture: &TextureHandle, rect: egui::Rect, rounding: egui::Rounding, tint: Color32) {
    let [w, h] = texture.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let scale = (rect.width() / w).max(rect.height() / h);
    let shown = Vec2::new(rect.width() / (w * scale), rect.height() / (h * scale));
    egui::Image::new(SizedTexture::new(texture.id(), rect.size()))
        .uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), shown))
        .rounding(rounding)
        .tint(tint)
        .paint_at(ui, rect);
}

fn paint_nsfw_badge(ui: &Ui, rect: egui::Rect, rounding: egui::Rounding, fade: f32) {
    ui.painter()
        .rect_filled(rect, rounding, Color32::from_black_alpha((70.0 * fade) as u8));
    let center = rect.center();
    let white = Color32::WHITE.gamma_multiply(fade);
    ui.painter().text(
        center - Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        Icon::EyeSlash.glyph().to_string(),
        egui::FontId::new(20.0, swmm_widgets::icons::icon_family()),
        white,
    );
    ui.painter().text(
        center + Vec2::new(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        "NSFW",
        egui::FontId::new(12.0, egui::FontFamily::Name("Bold".into())),
        white,
    );
}

fn render_card(ui: &mut Ui, item: &BrowseMod, texture: Option<&TextureHandle>, hidden: bool, selected: bool, size: Vec2) -> bool {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let palette = visuals::palette(ui);
    let id = response.id;
    let hover = ui.ctx().animate_bool_with_time(id.with("hover"), response.hovered(), 0.16);
    let picked = ui.ctx().animate_bool_with_time(id.with("picked"), selected, 0.2);
    let pressed = response.is_pointer_button_down_on();
    let card = if pressed { rect.shrink(1.5) } else { rect };
    let rounding = egui::Rounding::same(10.0);
    if hover > 0.0 {
        let shadow = egui::epaint::Shadow {
            offset: Vec2::new(0.0, 6.0),
            blur: 20.0,
            spread: 0.0,
            color: Color32::from_black_alpha((90.0 * hover) as u8),
        };
        ui.painter().add(shadow.as_shape(card, rounding));
    }
    let fill = chrome::mix(palette.raised, palette.hover, hover * 0.5);
    let stroke = chrome::mix(palette.border, palette.accent_text, picked);
    ui.painter().rect(card, rounding, fill, Stroke::new(1.0_f32, stroke));
    let image_rect = egui::Rect::from_min_size(card.min, Vec2::new(card.width(), card.width() * 9.0 / 16.0));
    let image_rounding = egui::Rounding {
        nw: 10.0,
        ne: 10.0,
        sw: 0.0,
        se: 0.0,
    };
    let fade = chrome::appear(ui.ctx(), id.with("thumb"), texture.is_some(), 0.35);
    if fade < 1.0 {
        ui.painter().rect_filled(image_rect, image_rounding, palette.base);
        ui.painter().text(
            image_rect.center(),
            egui::Align2::CENTER_CENTER,
            Icon::Image.glyph().to_string(),
            egui::FontId::new(26.0, swmm_widgets::icons::icon_family()),
            palette.border,
        );
    }
    if let Some(texture) = texture {
        paint_cover(ui, texture, image_rect, image_rounding, Color32::WHITE.gamma_multiply(fade));
    }
    if hidden {
        paint_nsfw_badge(ui, image_rect, image_rounding, fade.max(0.6));
    }
    let text = egui::Rect::from_min_max(
        egui::pos2(card.left() + 12.0, image_rect.bottom() + 10.0),
        egui::pos2(card.right() - 12.0, card.bottom() - 10.0),
    );
    let painter = ui.painter().with_clip_rect(text);
    let title = ui.fonts(|f| {
        f.layout_job(egui::text::LayoutJob {
            text: item.name.clone(),
            sections: vec![egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: 0..item.name.len(),
                format: egui::TextFormat::simple(
                    egui::FontId::new(14.0, egui::FontFamily::Name("Bold".into())),
                    palette.text,
                ),
            }],
            wrap: egui::text::TextWrapping::truncate_at_width(text.width()),
            ..Default::default()
        })
    });
    painter.galley(text.min, title, palette.text);
    painter.text(
        egui::pos2(text.left(), text.top() + 21.0),
        egui::Align2::LEFT_TOP,
        format!("{} {}", "SW_By".localize(), item.author),
        egui::FontId::proportional(12.0),
        palette.muted,
    );
    let stats_y = text.bottom() - 8.0;
    let icon_font = egui::FontId::new(10.0, swmm_widgets::icons::icon_family());
    let small = egui::FontId::proportional(12.0);
    let mut x = text.left();
    for (icon, value) in [(Icon::Heart, item.likes), (Icon::Eye, item.views)] {
        let glyph = painter.text(egui::pos2(x, stats_y), egui::Align2::LEFT_CENTER, icon.glyph().to_string(), icon_font.clone(), palette.muted);
        let label = painter.text(
            egui::pos2(glyph.right() + 5.0, stats_y),
            egui::Align2::LEFT_CENTER,
            format_count(value),
            small.clone(),
            palette.muted,
        );
        x = label.right() + 12.0;
    }
    if !item.category.is_empty() {
        painter.text(
            egui::pos2(text.right(), stats_y),
            egui::Align2::RIGHT_CENTER,
            item.category.as_str(),
            small,
            palette.accent_text,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(item.name.as_str())
        .clicked()
}

fn render_details(ui: &mut Ui, state: &mut BrowserState) {
    let palette = visuals::palette(ui);
    if state.details_loading {
        ui.centered_and_justified(|ui| {
            chrome::progress_ring(ui, 26.0, palette.accent_text);
        });
        return;
    }
    if state.details.is_none() {
        return;
    }
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    ui.spacing_mut().scroll.bar_width = 8.0;
    let area = ui.available_rect_before_wrap();
    let width = (area.width() - 14.0).max(80.0);
    ScrollArea::vertical().drag_to_scroll(false)
        .id_source("browseDetails")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(width);
                render_details_body(ui, state);
            });
            ui.add_space(8.0);
        });
    let chip = egui::Rect::from_center_size(
        egui::pos2(area.left() + width - 20.0, area.top() + 20.0),
        Vec2::splat(28.0),
    );
    if close_chip(ui, chip) {
        state.details = None;
    }
}

fn close_chip(ui: &mut Ui, rect: egui::Rect) -> bool {
    let response = ui
        .interact(rect, ui.id().with("browseDetailsClose"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Generic_Close".localize());
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
    let fill = Color32::from_black_alpha((165.0 + 60.0 * hover) as u8);
    let stroke = Stroke::new(1.0_f32, Color32::from_white_alpha((35.0 + 40.0 * hover) as u8));
    let glyph = Color32::from_white_alpha((210.0 + 45.0 * hover) as u8);
    let painter = ui.painter();
    painter.circle_filled(rect.center() + Vec2::new(0.0, 1.5), rect.width() / 2.0 + 1.0, Color32::from_black_alpha(60));
    painter.circle(rect.center(), rect.width() / 2.0, fill, stroke);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        Icon::Cancel.glyph(),
        egui::FontId::new(11.0, swmm_widgets::icons::icon_family()),
        glyph,
    );
    response.clicked()
}

fn render_details_body(ui: &mut Ui, state: &mut BrowserState) {
    let palette = visuals::palette(ui);
    let Some(details) = state.details.clone() else { return };
    let fade = chrome::content_fade(ui.ctx(), egui::Id::new("browseDetailsFade"), details.id, 0.22);
    ui.multiply_opacity(fade);
    ui.add_space((1.0 - fade) * 8.0);
    let width = ui.available_width();
    if let Some(Some(texture)) = state.thumbnails.get(&details.id).cloned() {
        let hidden = details.nsfw && !state.revealed.contains(&details.id);
        let shown = match (hidden, state.blurred.get(&details.id)) {
            (true, Some(blurred)) => blurred.clone(),
            _ => texture.clone(),
        };
        let [w, h] = texture.size();
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let scale = (width / w).min(220.0 / h);
        let size = Vec2::new(w * scale, h * scale);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, size.y), Sense::hover());
        let image = egui::Rect::from_center_size(rect.center(), size);
        let response = ui.interact(image, egui::Id::new(("browseDetailsImage", details.id)), Sense::click());
        egui::Image::new(SizedTexture::new(shown.id(), size))
            .rounding(10.0)
            .paint_at(ui, image);
        if hidden {
            paint_nsfw_badge(ui, image, egui::Rounding::same(10.0), 1.0);
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Browse_ShowNsfw".localize())
                .clicked()
            {
                state.revealed.insert(details.id);
            }
        }
        ui.add_space(14.0);
    }
    let has_thumbnail = state.thumbnails.get(&details.id).is_some_and(|t| t.is_some());
    ui.horizontal(|ui| {
        let title_width = if has_thumbnail { width } else { (width - 40.0).max(60.0) };
        ui.allocate_ui_with_layout(Vec2::new(title_width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(title_width);
            ui.add(
                egui::Label::new(
                    RichText::new(details.name.as_str())
                        .size(18.0)
                        .family(egui::FontFamily::Name("Bold".into()))
                        .color(palette.text),
                )
                .wrap(),
            );
        });
    });
    if !details.author.is_empty() {
        ui.label(RichText::new(format!("{} {}", "SW_By".localize(), details.author)).size(13.0).color(palette.muted));
    }
    ui.add_space(8.0);
    if ui.icon_text_button("Browse_OpenPage".localize(), Icon::External).clicked() {
        open::that(&details.url).unwrap_or(());
    }
    ui.add_space(14.0);
    ui.label(RichText::new("Browse_Files".localize()).size(13.0).strong().color(palette.muted));
    ui.add_space(4.0);
    if details.files.is_empty() {
        ui.label(RichText::new("Browse_NoFiles".localize()).color(palette.muted));
    }
    let busy = state.downloading.clone();
    for file in details.files.iter() {
        egui::Frame::none()
            .fill(palette.raised)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .rounding(8.0)
            .inner_margin(Margin::same(10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(file.name.as_str()).color(palette.text)).truncate());
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {} {}",
                        format_size(file.size),
                        format_count(file.downloads),
                        "Browse_Downloads".localize()
                    ))
                    .size(12.0)
                    .color(palette.muted),
                );
                if !file.description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(file.description.as_str()).size(12.0).color(palette.muted)).wrap());
                }
                ui.add_space(6.0);
                let downloading = busy.as_deref() == Some(file.name.as_str());
                if downloading {
                    render_download_progress(ui, state);
                } else {
                    let clicked = ui
                        .add_enabled_ui(busy.is_none(), |ui| {
                            chrome::fancy_button(ui, Icon::Download, &"Browse_Install".localize(), ui.available_width(), 30.0)
                        })
                        .inner
                        .clicked();
                    if clicked {
                        state.start_download(file.clone());
                    }
                }
            });
        ui.add_space(6.0);
    }
    ui.add_space(10.0);
    if details.text.is_empty() {
        ui.label(RichText::new("SW_NoDescription".localize()).color(palette.muted));
    } else {
        ui.add(egui::Label::new(RichText::new(details.text.as_str()).color(palette.text)).wrap());
    }
}
