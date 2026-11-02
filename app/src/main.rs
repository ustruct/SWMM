#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::any::Any;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
mod cli;
mod ui;
mod logger;

use anyhow_ext::Result;
use cli::Swmm;

#[cfg(target_os = "windows")]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn AttachConsole(pid: i32) -> bool;
}
#[cfg(target_os = "windows")]
#[link(name = "User32")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: i32, message: *const i8, title: *const i8, utype: usize) -> i32;
}

const INTERFACE: ssilide::Interface = ssilide::Interface::new(6667);

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    unsafe {
        AttachConsole(-1);
    }

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    match Swmm::from_env() {
        Ok(command) => {
            cli::Runner::new(command).run()?;
        }
        Err(e) => {
            if !e.is_help() {
                if std::env::args().any(|a| a == "--debug") {
                    env_logger::init();
                    log::set_max_level(log::LevelFilter::Debug);
                }
                if let Some(path) = std::env::args().nth(1).filter(|a| !a.starts_with('-')) {
                    ui::jobs::handle_mod_arg(path.into());
                }
                ui::jobs::wait_ipc();
                std::panic::set_hook(Box::new(|_| {
                    println!("{}", std::backtrace::Backtrace::force_capture());
                }));
                if let Err(e) = std::panic::catch_unwind(ui::main) {
                    display_error(e);
                    if let Some(file) = logger::LOGGER.log_path() {
                        logger::LOGGER.save_log();
                        println!(
                            "More information may be available in the log file at {}. You can run \
                             with the --debug flag for additional detail.",
                            file.display()
                        );
                    }
                }
            } else {
                Swmm::from_env_or_exit();
            }
        }
    }
    Ok(())
}

fn display_error(e: Box<dyn Any + Send>) {
    let error_msg = format!(
        "An unrecoverable error occurred. Error details: {}",
        e.downcast::<String>()
            .or_else(|e| {
                e.downcast::<&'static str>().map(|s| Box::new((*s).into()))
            })
            .unwrap_or_else(|_| {
                Box::new(
                    "An unknown error occurred, check the log for possible details."
                        .to_string(),
                )
            })
    );
    #[cfg(windows)]
    unsafe {
        let error_msg = error_msg.encode_utf16().collect::<Vec<u16>>();
        let title = "Error".encode_utf16().collect::<Vec<u16>>();
        MessageBoxW(
            0,
            error_msg.as_ptr() as *const i8,
            title.as_ptr() as *const i8,
            0x0 | 0x10,
        );
    }
    #[cfg(not(windows))]
    println!("{}", error_msg);
}
