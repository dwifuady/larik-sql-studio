// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = larik_sql_studio_lib::run() {
        eprintln!("[Fatal] Larik SQL Studio failed to start: {e}");
        std::process::exit(1);
    }
}
