// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

// Module declarations
pub mod commands;
pub mod db;
pub mod export;
pub mod mcp;
pub mod storage;

use commands::AppState;
use db::{ConnectionManager, DriverRegistry, QueryEngine, SchemaMetadataManager};
use mcp::{McpState, McpSupervisor};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use storage::{get_default_db_path, DatabaseManager};
use tauri::window::{Effect, EffectsBuilder};
use tauri::Manager;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{interval, Duration};

/// Whether the Windows 11 Mica backdrop was applied to the main window.
/// The frontend only switches to translucent surfaces when this is true,
/// so unsupported systems keep the opaque theme.
pub struct MicaEnabled(pub bool);

#[tauri::command]
fn is_mica_enabled(state: tauri::State<'_, MicaEnabled>) -> bool {
    state.0
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the database
    let db_path = get_default_db_path()?;
    println!("[Startup] Database path: {:?}", db_path);
    let db_manager = DatabaseManager::new(db_path.clone())?;

    // Initialize snippets schema and seed built-in snippets (T046)
    db_manager
        .init_snippets_schema()
        .map_err(|e| format!("Failed to initialize snippets schema: {e}"))?;
    let seeded_count = db_manager
        .seed_builtin_snippets()
        .map_err(|e| format!("Failed to seed built-in snippets: {e}"))?;
    if seeded_count > 0 {
        println!("[Startup] Seeded {} new built-in snippets", seeded_count);
    }

    // Log total snippet count
    if let Ok(all_snippets) = db_manager.get_all_snippets() {
        println!(
            "[Startup] Total snippets in database: {}",
            all_snippets.len()
        );
    }

    // Initialize default settings for archive/history
    db_manager
        .init_default_settings()
        .map_err(|e| format!("Failed to initialize default settings: {e}"))?;

    // Initialize connection manager with all built-in database drivers (T015, T016)
    let connections = Arc::new(ConnectionManager::new(
        DriverRegistry::with_builtin_drivers(),
    ));

    // Initialize query engine (T017)
    let query_engine = Arc::new(QueryEngine::new(Arc::clone(&connections)));

    // Initialize schema metadata manager (T024)
    let schema_db_manager = DatabaseManager::new(db_path.clone())?;
    let schema_manager = Arc::new(SchemaMetadataManager::new(
        Arc::clone(&connections),
        Arc::new(schema_db_manager),
    ));

    let db = Arc::new(Mutex::new(db_manager));

    let mcp_state = Arc::new(McpState {
        connections: Arc::clone(&connections),
        query_engine: Arc::clone(&query_engine),
        schema_manager: Arc::clone(&schema_manager),
        db: Arc::clone(&db),
        token: RwLock::new(String::new()),
        pending_approvals: parking_lot::Mutex::new(HashMap::new()),
        app_handle: parking_lot::Mutex::new(None),
    });
    let mcp = Arc::new(McpSupervisor::new(Arc::clone(&mcp_state)));

    let app_state = AppState {
        db,
        connections,
        query_engine,
        schema_manager,
        export_cancel_flags: RwLock::new(HashMap::new()),
        archive_task: StdMutex::new(None),
        mcp,
    };

    // Clone DB path for background task
    let db_path_for_bg = db_path.clone();

    tauri::Builder::default()
        .setup(move |app| {
            // Spawn background auto-archive task and keep its handle so it can
            // be aborted on exit (Step 1.6d).
            let handle = spawn_auto_archive_task(db_path_for_bg);
            let state = app.state::<AppState>();
            let mut slot = state
                .archive_task
                .lock()
                .map_err(|_| "AppState lock poisoned")?;
            *slot = Some(handle);

            // MCP server: attach the handle (approval dialogs), load the token,
            // then start if the user enabled it.
            let supervisor = state.mcp.clone();
            supervisor
                .state()
                .app_handle
                .lock()
                .replace(app.handle().clone());
            tauri::async_runtime::spawn(async move {
                let mcp_state = supervisor.state();
                let token = {
                    let db = mcp_state.db.lock().await;
                    db.get_mcp_token().unwrap_or_default()
                };
                supervisor.set_token(token).await;
                let enabled = {
                    let db = mcp_state.db.lock().await;
                    db.get_mcp_enabled().unwrap_or(false)
                };
                if enabled {
                    if let Err(e) = supervisor.start().await {
                        eprintln!("[MCP] failed to start on launch: {e}");
                    }
                }
            });
            let mica = app
                .get_webview_window("main")
                .map(|w| {
                    w.set_effects(EffectsBuilder::new().effect(Effect::Mica).build())
                        .is_ok()
                })
                .unwrap_or(false);
            app.manage(MicaEnabled(mica));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window quits the app, taking any pop-out tab
            // windows with it; their tabs simply reopen docked on next launch.
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .manage(app_state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            is_mica_enabled,
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
            commands::get_history_retention_days,
            commands::update_history_retention_days,
            commands::purge_archived_tabs_now,
            commands::purge_all_archived_tabs,
            // App Settings commands
            commands::get_app_settings,
            commands::update_app_settings,
            // Database Management commands
            commands::export_database,
            commands::import_database,
            // Sticky Notes commands (v2 — gutter icon + DB storage)
            commands::get_tab_notes,
            commands::save_note,
            commands::delete_note,
            commands::move_note,
            commands::clear_tab_notes,
            // Virtual reference commands (user-defined foreign keys)
            commands::get_virtual_references,
            commands::save_virtual_reference,
            commands::delete_virtual_reference,
            // MCP server commands
            commands::get_mcp_settings,
            commands::set_mcp_enabled,
            commands::set_mcp_port,
            commands::regenerate_mcp_token,
            commands::get_mcp_log,
            commands::respond_mcp_approval,
            commands::get_pending_mcp_approvals,
        ])
        .build(tauri::generate_context!())?
        .run(|app_handle, event| {
            // Abort the background archiver cleanly on exit (Step 1.6d).
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                let state = app_handle.state::<AppState>();
                let taken = state
                    .archive_task
                    .lock()
                    .ok()
                    .and_then(|mut slot| slot.take());
                if let Some(handle) = taken {
                    handle.abort();
                }
            }
        });
    Ok(())
}

/// Spawn background task for auto-archiving inactive tabs.
/// Returns the task handle so the caller can abort it on exit.
fn spawn_auto_archive_task(db_path: std::path::PathBuf) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        // Create a separate DatabaseManager for this background task
        let db = match DatabaseManager::new(db_path) {
            Ok(db) => db,
            Err(e) => {
                eprintln!("[Auto-Archive] Failed to initialize database: {}", e);
                return;
            }
        };

        // Jitter the first tick (0-299s) so multiple instances/timers do not
        // wake in lockstep (Step 1.6d). Derived from clock nanos — no rand dep.
        let jitter_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.subsec_nanos() as u64) % 300)
            .unwrap_or(0);
        tokio::time::sleep(Duration::from_secs(jitter_secs)).await;

        let mut ticker = interval(Duration::from_secs(3600)); // Every hour

        loop {
            ticker.tick().await;

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
                println!(
                    "[Auto-Archive] Found {} inactive tabs to archive",
                    tabs_to_archive.len()
                );

                for tab in tabs_to_archive {
                    match db.archive_tab(&tab.id) {
                        Ok(_) => println!("[Auto-Archive] Archived tab: {}", tab.title),
                        Err(e) => {
                            eprintln!("[Auto-Archive] Failed to archive tab {}: {}", tab.id, e)
                        }
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
    })
}
