pub mod ext;
pub mod icons;
mod paths;
pub mod visuals;
pub use egui;
pub use egui_extras;
use font_loader::system_fonts::FontPropertyBuilder;
use include_flate::flate;
pub use paths::PathNode;

flate!(static NOTOSANS_REG: [u8] from "fonts/NotoSans-Regular.ttf");
flate!(static NOTOSANS_BOLD: [u8] from "fonts/NotoSans-Bold.ttf");
flate!(static FONT_AWESOME: [u8] from "fonts/fa-solid-900.ttf");
flate!(static TITLE_FONT: [u8] from "fonts/title.otf");

fn font_is_valid(data: &[u8], index: u32) -> bool {
    ab_glyph::FontRef::try_from_slice_and_index(data, index).is_ok()
}

fn font_data(data: Vec<u8>, index: u32) -> Option<egui::FontData> {
    font_is_valid(&data, index).then(|| egui::FontData {
        index,
        ..egui::FontData::from_owned(data)
    })
}

fn load_system_font(name: &str, bold: bool) -> Option<egui::FontData> {
    #[cfg(windows)]
    {
        let fonts_dir = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"))
            .join("Fonts");
        let files: &[&str] = match (name, bold) {
            ("NotoJP", false) => &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"],
            ("NotoJP", true) => &["YuGothB.ttc", "meiryob.ttc"],
            ("NotoKR", false) => &["malgun.ttf"],
            ("NotoKR", true) => &["malgunbd.ttf"],
            ("NotoSC", false) => &["msyh.ttc", "simhei.ttf", "simsun.ttc"],
            ("NotoSC", true) => &["msyhbd.ttc"],
            _ => &[],
        };
        files.iter().find_map(|file| {
            let data = std::fs::read(fonts_dir.join(file)).ok()?;
            font_data(data, 0)
        })
    }
    #[cfg(not(windows))]
    {
        let candidates: &[&str] = match name {
            "NotoJP" => &["Noto Sans CJK JP", "Hiragino Sans", "IPAGothic"],
            "NotoKR" => &["Noto Sans CJK KR", "Apple SD Gothic Neo", "NanumGothic"],
            "NotoSC" => &["Noto Sans CJK SC", "PingFang SC", "WenQuanYi Micro Hei"],
            _ => &[],
        };
        candidates.iter().find_map(|family| {
            let mut builder = FontPropertyBuilder::new().family(family);
            if bold {
                builder = builder.bold();
            }
            let (data, index) = font_loader::system_fonts::get(&builder.build())?;
            font_data(data, index.max(0) as u32)
        })
    }
}

pub const TITLE_FAMILY: &str = "Title";

const CJK_FONTS: &[&str] = &["NotoJP", "NotoKR", "NotoSC"];

pub fn load_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_to_try = if cfg!(windows) {
        "Segoe UI".to_owned()
    } else if cfg!(target_os = "macos") {
        "Lucida Grande".to_owned()
    } else {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "font-name"])
            .output()
            .ok()
            .and_then(|o| (!o.stdout.is_empty()).then_some(o))
            .and_then(|o| {
                String::from_utf8(o.stdout)
                    .map(|s| {
                        let last_space = s.rfind(' ').unwrap();
                        s[..last_space].trim_matches('\'').to_string()
                    })
                    .ok()
            })
            .unwrap_or_else(|| "Ubuntu".to_owned())
    };
    if let Some(system_font) =
        font_loader::system_fonts::get(&FontPropertyBuilder::new().family(&font_to_try).build())
            .and_then(|(data, index)| font_data(data, index.max(0) as u32))
    {
        fonts.font_data.insert("System".to_owned(), system_font);
    }
    if let Some(system_font) = font_loader::system_fonts::get(
        &FontPropertyBuilder::new()
            .family(&font_to_try)
            .bold()
            .build(),
    )
    .or_else(|| {
        let property = FontPropertyBuilder::new()
            .family(&(font_to_try + " Bold"))
            .build();
        font_loader::system_fonts::get(&property)
    })
    .and_then(|(data, index)| font_data(data, index.max(0) as u32))
    {
        fonts.font_data.insert("Bold".to_owned(), system_font);
    }
    fonts.font_data.insert(
        "Noto".to_owned(),
        egui::FontData::from_static(&NOTOSANS_REG)
    );
    fonts.font_data.insert(
        "NotoBold".to_owned(),
        egui::FontData::from_static(&NOTOSANS_BOLD)
    );
    fonts.font_data.insert(
        "FontAwesome".to_owned(),
        egui::FontData::from_static(&FONT_AWESOME)
    );
    fonts.families.insert(
        egui::FontFamily::Name(icons::ICON_FAMILY.into()),
        vec!["FontAwesome".to_owned()],
    );
    fonts.font_data.insert(
        "Title".to_owned(),
        egui::FontData::from_static(&TITLE_FONT).tweak(egui::FontTweak {
            y_offset_factor: 0.09,
            ..Default::default()
        }),
    );
    fonts.families.insert(
        egui::FontFamily::Name(TITLE_FAMILY.into()),
        vec!["Title".to_owned(), "Noto".to_owned()],
    );
    let mut cjk_loaded: Vec<&'static str> = Vec::new();
    for name in CJK_FONTS {
        if let Some(data) = load_system_font(name, false) {
            fonts.font_data.insert((*name).to_owned(), data);
            cjk_loaded.push(*name);
        }
        if let Some(data) = load_system_font(name, true) {
            fonts.font_data.insert(format!("{name}Bold"), data);
        }
    }
    if let Some(family) = fonts
        .families
        .get_mut(&egui::FontFamily::Proportional) {

        let mut order: Vec<String> = Vec::new();
        if fonts.font_data.contains_key("System") {
            order.push("System".to_owned());
        }
        order.push("Noto".to_owned());
        order.extend(cjk_loaded.iter().map(|s| s.to_string()));
        for (i, name) in order.into_iter().enumerate() {
            family.insert(i, name);
        }
        family.push("FontAwesome".to_owned());
    }
    fonts
        .families
        .insert(
            egui::FontFamily::Name("Bold".into()),
            ["Bold", "NotoBold", "NotoJPBold", "NotoKRBold", "NotoSCBold"]
                .into_iter()
                .filter(|n| fonts.font_data.contains_key(*n))
                .map(|n| n.to_owned())
                .collect(),
        );
    context.set_fonts(fonts);
}
