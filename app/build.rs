#[cfg(windows)]
extern crate winres;

fn copyright_string() {
    use std::io::Write;

    use astrolabe::DateUtilities;
    let year = astrolabe::Date::now().year();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = format!("{}/build_info.rs", out_dir);
    let mut file = std::fs::File::create(dest_path).unwrap();
    writeln!(
        &mut file,
        "pub const COPYRIGHT: &str = \"© {year} SWMM contributors. Free software under the GNU GPL v3 or later, provided without warranty. Contains code © 2022-{year} Caleb Smith and Ginger Chody.\";"
    )
    .unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}

#[cfg(windows)]
fn add_icon() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("../assets/swmm.ico");
    res.compile().unwrap();
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/NODEFAULTLIB:msvcrtd.lib");
    }
    #[cfg(windows)]
    add_icon();
    copyright_string();
}
