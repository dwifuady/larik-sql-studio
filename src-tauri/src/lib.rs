// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

// Module declarations
pub mod commands;
pub mod db;
pub mod export;
pub mod storage;

use commands::AppState;
use db::{MssqlConnectionManager, QueryEngine, SchemaMetadataManager};
use storage::{DatabaseManager, get_default_db_path};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize the database
    let db_path = get_default_db_path().expect("Failed to get database path");
    println!("[Startup] Database path: {:?}", db_path);
    let db_manager = DatabaseManager::new(db_path.clone()).expect("Failed to initialize database");
    
    // Initialize snippets schema and seed built-in snippets (T046)
    db_manager.init_snippets_schema().expect("Failed to initialize snippets schema");
    let seeded_count = db_manager.seed_builtin_snippets().expect("Failed to seed built-in snippets");
    if seeded_count > 0 {
        println!("[Startup] Seeded {} new built-in snippets", seeded_count);
    }
    
    // Log total snippet count
    if let Ok(all_snippets) = db_manager.get_all_snippets() {
        println!("[Startup] Total snippets in database: {}", all_snippets.len());
    }

    // Initialize default settings for archive/history
    db_manager.init_default_settings().expect("Failed to initialize default settings");

    // Initialize MS-SQL connection manager (T015, T016)
    let mssql_manager = Arc::new(MssqlConnectionManager::new());

    // Initialize query engine (T017)
    let query_engine = Arc::new(QueryEngine::new(Arc::clone(&mssql_manager)));

    // Initialize schema metadata manager (T024)
    let schema_manager = Arc::new(SchemaMetadataManager::new(Arc::clone(&mssql_manager)));

    let app_state = AppState {
        db: Mutex::new(db_manager),
        mssql_manager,
        query_engine,
        schema_manager,
        export_cancel_flags: RwLock::new(HashMap::new()),
    };

    // Clone DB path for background task
    let db_path_for_bg = db_path.clone();

    tauri::Builder::default()
        .setup(move |_app| {
            // Spawn background auto-archive task
            spawn_auto_archive_task(db_path_for_bg);
            Ok(())
        })
        .manage(app_state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // Space commands (T006) - with integrated connection
            commands::create_space,
            commands::get_spaces,
            commands::get_space,
            commands::update_space,
            commands::delete_space,
            commands::reorder_spaces,
            commands::update_space_last_active_tab,
            // Tab commands (T007)
            commands::create_tab,
            commands::get_tabs_by_space,
            commands::get_tab,
            commands::update_tab,
            commands::update_tab_database,
            commands::autosave_tab_content,
            commands::toggle_tab_pinned,
            commands::delete_tab,
            commands::reorder_tabs,
            commands::reorder_tabs,
            commands::move_tab_to_space,
            commands::search_tabs,
            // Folder commands - Arc Browser-style tab organization
            commands::create_folder,
            commands::get_folders_by_space,
            commands::update_folder,
            commands::delete_folder,
            commands::add_tab_to_folder,
            commands::remove_tab_from_folder,
            commands::reorder_folders,
            commands::create_folder_from_tabs,
            // Space Connection commands (T018) - 1:1 model
            commands::connect_to_space,
            commands::disconnect_from_space,
            commands::get_space_connection_status,
            commands::get_space_databases,
            commands::get_space_databases_with_access,
            // Legacy Connection commands (T018) - kept for flexibility
            commands::create_connection,
            commands::test_connection,
            commands::get_connections,
            commands::get_connections_by_space,
            commands::get_connection,
            commands::update_connection,
            commands::delete_connection,
            commands::connect_database,
            commands::disconnect_database,
            commands::get_connection_databases,
            commands::check_connection_health,
            // Query commands (T019)
            commands::execute_query,
            commands::cancel_query,
            commands::cancel_queries_for_connection,
            commands::get_query_status,
            commands::close_tab_connection,
            // Schema metadata commands (T025)
            commands::get_schema_info,
            commands::get_table_columns,
            commands::refresh_schema,
            // Export commands (T034, T035, T036)
            commands::export_to_csv,
            commands::export_to_json,
            commands::export_to_string,
            commands::cancel_export,
            // SQL File Import/Export commands
            commands::export_tab_as_sql,
            commands::import_sql_file_as_tab,
            // Snippet commands (T046)
            commands::get_snippets,
            commands::get_enabled_snippets,
            commands::get_snippet,
            commands::get_snippet_by_trigger,
            commands::create_snippet,
            commands::update_snippet,
            commands::delete_snippet,
            commands::reset_builtin_snippet,
            commands::import_snippets,
            // Archive/History commands
            commands::archive_tab,
            commands::restore_archived_tab,
            commands::search_archived_tabs,
            commands::get_archived_tabs,
            commands::get_archived_tabs_count,
            commands::delete_archived_tab,
            commands::touch_tab,
            commands::get_auto_archive_settings,
            commands::update_auto_archive_settings,
            // App Settings commands
            commands::get_app_settings,
            commands::update_app_settings,
            // Database Management commands
            commands::export_database,
            commands::import_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Spawn background task for auto-archiving inactive tabs
fn spawn_auto_archive_task(db_path: std::path::PathBuf) {
    tauri::async_runtime::spawn(async move {
        // Create a separate DatabaseManager for this background task
        let db = match DatabaseManager::new(db_path) {
            Ok(db) => db,
            Err(e) => {
                eprintln!("[Auto-Archive] Failed to initialize database: {}", e);
                return;
            }
        };

        let mut interval = interval(Duration::from_secs(3600)); // Every hour

        loop {
            interval.tick().await;

            // Check if enabled
            let enabled = match db.get_auto_archive_enabled() {
                Ok(enabled) => enabled,
                Err(_) => continue,
            };

            if !enabled {
                continue;
            }

            // Get settings
            let days_inactive = match db.get_auto_archive_days() {
                Ok(days) => days,
                Err(_) => 14,
            };

            // Archive inactive tabs
            let tabs_to_archive = match db.find_inactive_tabs(days_inactive) {
                Ok(tabs) => tabs,
                Err(e) => {
                    eprintln!("[Auto-Archive] Failed to find inactive tabs: {}", e);
                    continue;
                }
            };

            if !tabs_to_archive.is_empty() {
                println!("[Auto-Archive] Found {} inactive tabs to archive", tabs_to_archive.len());

                for tab in tabs_to_archive {
                    match db.archive_tab(&tab.id) {
                        Ok(_) => println!("[Auto-Archive] Archived tab: {}", tab.title),
                        Err(e) => eprintln!("[Auto-Archive] Failed to archive tab {}: {}", tab.id, e),
                    }
                }
            }

            // Cleanup old archived tabs (>90 days)
            let retention_days = match db.get_history_retention_days() {
                Ok(days) => days,
                Err(_) => 90,
            };

            match db.cleanup_old_archived_tabs(retention_days) {
                Ok(count) => {
                    if count > 0 {
                        println!("[Auto-Archive] Cleaned up {} old archived tabs", count);
                    }
                }
                Err(e) => eprintln!("[Auto-Archive] Failed to cleanup old tabs: {}", e),
            }
        }
    });
}

