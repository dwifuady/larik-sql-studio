// src-tauri/src/db/management.rs

use crate::commands::AppState;
use std::path::Path;
use tauri::{AppHandle, State};

/// Export the application database to a specified file path using the online
/// `Backup` API (safe under WAL, no file-handle copy).
pub async fn export_database(state: &State<'_, AppState>, destination: &str) -> Result<(), String> {
    let validated_dest = crate::storage::paths::validate_export_path(destination, "db")?;

    // Ensure the destination directory exists
    let dest_path = Path::new(&validated_dest);
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create destination directory: {}", e))?;
        }
    }

    let db_manager: tokio::sync::MutexGuard<'_, crate::storage::DatabaseManager> =
        state.db.lock().await;
    // Online backup — holds the src lock only for the duration of the copy
    db_manager
        .with_connection(|src| {
            let mut dst = rusqlite::Connection::open(&validated_dest).map_err(|e| {
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
            })?;
            let backup = rusqlite::backup::Backup::new(src, &mut dst).map_err(|e| {
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
            })?;
            backup
                .run_to_completion(100, std::time::Duration::from_millis(50), None)
                .map_err(|e| {
                    rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(1),
                        Some(e.to_string()),
                    )
                })?;
            Ok(())
        })
        .map_err(|e: crate::storage::StorageError| e.to_string())?;

    Ok(())
}

/// Import the application database from a specified file path and restart the app.
/// Uses `Backup`-validated file copy with handle-close dance to avoid Windows locking issues.
pub async fn import_database(
    app_handle: &AppHandle,
    state: &State<'_, AppState>,
    source: &str,
) -> Result<(), String> {
    let validated_source = crate::storage::paths::validate_import_path(source, "db")?;

    // Integrity check on source read-only
    {
        let src_conn = rusqlite::Connection::open_with_flags(
            &validated_source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let integrity: String = src_conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err(format!(
                "Source database integrity check failed: {}",
                integrity
            ));
        }
    }

    let db_path = {
        let guard = state.db.lock().await;
        guard.db_path().clone()
    };

    // Close the old handle by swapping in a temporary placeholder, dropping the old manager
    let temp_placeholder_path = std::env::temp_dir().join(format!(
        "larik_import_placeholder_{}.db",
        uuid::Uuid::new_v4()
    ));
    {
        let mut guard = state.db.lock().await;
        let placeholder = crate::storage::DatabaseManager::new(temp_placeholder_path.clone())
            .map_err(|e| e.to_string())?;
        let old_manager = std::mem::replace(&mut *guard, placeholder);
        drop(old_manager);
    }

    // Copy validated source over the real file — now safe, no open handle holds it
    if let Err(e) = std::fs::copy(&validated_source, &db_path) {
        // Attempt to restore by reopening the (possibly corrupted) destination
        // so the app remains usable; propagate the error and do NOT restart
        let _ = crate::storage::DatabaseManager::new(db_path.clone()).map(|new_mgr| {
            let guard = state.db.try_lock();
            if let Ok(mut g) = guard {
                let placeholder = std::mem::replace(&mut *g, new_mgr);
                let _ = std::fs::remove_file(placeholder.db_path());
            }
        });
        let _ = std::fs::remove_file(&temp_placeholder_path);
        return Err(format!("Failed to replace database file: {}", e));
    }

    // Reopen the database from the newly copied file
    let new_manager = crate::storage::DatabaseManager::new(db_path.clone())
        .map_err(|e| format!("Failed to reopen database after import: {}", e))?;

    {
        let mut guard = state.db.lock().await;
        let placeholder = std::mem::replace(&mut *guard, new_manager);
        let _ = std::fs::remove_file(placeholder.db_path());
        drop(placeholder);
    }
    let _ = std::fs::remove_file(&temp_placeholder_path);

    // Only restart after a successful reopen
    app_handle.restart();
}
