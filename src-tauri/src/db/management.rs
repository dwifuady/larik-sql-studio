// src-tauri/src/db/management.rs

use crate::commands::AppState;
use std::path::Path;
use tauri::{AppHandle, State};

/// Export the application database to a specified file path
pub fn export_database(state: &State<AppState>, destination: &str) -> Result<(), String> {
    let db_manager = state.db.lock().unwrap();
    let db_path = db_manager.db_path();

    if !db_path.exists() {
        return Err("Database file not found.".to_string());
    }

    // Ensure the destination directory exists
    let dest_path = Path::new(destination);
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(format!("Failed to create destination directory: {}", e));
            }
        }
    }

    match std::fs::copy(db_path, destination) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Failed to copy database file: {}", e)),
    }
}

/// Import the application database from a specified file path and restart the app
pub fn import_database(
    app_handle: &AppHandle,
    state: &State<AppState>,
    source: &str,
) -> Result<(), String> {
    let source_path = Path::new(source);
    if !source_path.exists() {
        return Err("Source database file not found.".to_string());
    }

    // This is a critical step: we need to get the db_path *before* we
    // potentially shut down the connection or lock anything.
    let db_path = {
        let db_manager = state.db.lock().unwrap();
        db_manager.db_path().clone()
    };

    // It's safest to just replace the file and restart.
    // The current DatabaseManager holds an open file handle, which can cause
    // locking issues on Windows. The simplest way to release it is to restart.
    match std::fs::copy(source_path, &db_path) {
        Ok(_) => {
            // Restart the application to apply the changes
            app_handle.restart();
        }
        Err(e) => Err(format!("Failed to replace database file: {}", e)),
    }
}
