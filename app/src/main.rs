#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::{
    any::Any,
    sync::atomic::{AtomicUsize, Ordering},
};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

unsafe extern "C" {
    fn mi_collect(force: bool);
}

pub fn release_allocator_memory() {
    unsafe { mi_collect(true) };
}
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
    fn MessageBoxW(hwnd: isize, message: *const u16, title: *const u16, utype: u32) -> i32;
}

const INTERFACE: ssilide::Interface = ssilide::Interface::new(6667);

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    unsafe {
        AttachConsole(-1);
    }

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    match Swmm::from_env() {
        Ok(command) => {
            cli::Runner::new(command)?.run()?;
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
                std::panic::set_hook(Box::new(record_panic));
                let failure = match std::panic::catch_unwind(ui::main) {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(e) => Some(panic_text(e)),
                };
                if let Some(text) = failure {
                    display_error(&text);
                    logger::LOGGER.save_log();
                    if let Some(file) = logger::LOGGER.log_path() {
                        println!("More information may be available in the log file at {}.", file.display());
                    }
                }
            } else {
                Swmm::from_env_or_exit();
            }
        }
    }
    Ok(())
}

fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 5 || count % 100 == 0 {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("{info}
{backtrace}");
        println!("{info}
{backtrace}");
        logger::LOGGER.save_log();
    }
}

fn panic_text(e: Box<dyn Any + Send>) -> String {
    match e.downcast::<String>() {
        Ok(text) => *text,
        Err(e) => e
            .downcast::<&'static str>()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "An unknown error occurred, check the log for possible details.".to_string()),
    }
}

fn display_error(details: &str) {
    let error_msg = format!("An unrecoverable error occurred. Error details: {details}");
    #[cfg(windows)]
    unsafe {
        let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let error_msg = wide(&error_msg);
        let title = wide("SWMM error");
        MessageBoxW(0, error_msg.as_ptr(), title.as_ptr(), 0x10);
    }
    #[cfg(not(windows))]
    println!("{}", error_msg);
}
