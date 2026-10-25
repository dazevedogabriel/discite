//! Discite — caderno de estudos e flashcards com repetição espaçada (FSRS).

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod cards;
mod db;
mod gui;
mod i18n;
mod media;
mod rich;
mod search;
mod srs;
mod stats;

fn main() {
    if let Err(e) = gui::run() {
        #[cfg(windows)]
        message_box(&format!("{e:#}"));
        #[cfg(not(windows))]
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn message_box(msg: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(msg), wide("Discite"));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR);
    }
}
