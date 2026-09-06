#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod ui {
    slint::include_modules!();
}
fn main() {
    if let Err(error) = app::run() {
        eprintln!("Kova Image: {error}");
        #[cfg(windows)]
        kova_image::windows_integration::startup_error(&error.to_string());
    }
}
