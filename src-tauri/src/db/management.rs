// src-tauri/src/db/management.rs

use crate::commands::AppState;
use std::path::Path;
use tauri::{AppHandle, State};

/// Export the application database to a specified file path
pub async fn export_database(state: &State<'_, AppState>, destination: &str) -> Result<(), String> {
    let validated_dest = crate::storage::paths::validate_export_path(destination, "db")?;
    let db_manager = state.db.lock().await;
    let db_path = db_manager.db_path();

    if !db_path.exists() {
        return Err("Database file not found.".to_string());
    }

    // Ensure the destination directory exists
    let dest_path = Path::new(&validated_dest);
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(format!("Failed to create destination directory: {}", e));
            }
        }
    }

    match std::fs::copy(db_path, &validated_dest) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Failed to copy database file: {}", e)),
    }
}

/// Import the application database from a specified file path and restart the app
pub async fn import_database(
    app_handle: &AppHandle,
    state: &State<'_, AppState>,
    source: &str,
) -> Result<(), String> {
    let validated_source = crate::storage::paths::validate_import_path(source, "db")?;
    let source_path = Path::new(&validated_source);
    if !source_path.exists() {
        return Err("Source database file not found.".to_string());
    }

    // This is a critical step: we need to get the db_path *before* we
    // potentially shut down the connection or lock anything.
    let db_path = {
        let db_manager = state.db.lock().await;
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
