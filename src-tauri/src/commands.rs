// IPC Bridge - Tauri Command Handlers
// This module contains all commands exposed to the frontend via Tauri's invoke system

use tauri::{command, State, AppHandle, Emitter};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};

use crate::storage::{
    DatabaseManager, StorageError,
    Space, CreateSpaceInput, UpdateSpaceInput,
    Tab, TabType, CreateTabInput, UpdateTabInput,
    TabFolder, CreateFolderInput, UpdateFolderInput,
    Snippet, CreateSnippetInput, UpdateSnippetInput,
    ArchivedTab, ArchiveSearchResult, AutoArchiveSettings, AppSettings,
};

use crate::db::{
    ConnectionConfig, ConnectionConfigUpdate, ConnectionInfo,
    MssqlConnectionManager, QueryEngine, QueryResult, QueryInfo,
    SchemaMetadataManager, SchemaInfo, SchemaColumnInfo,
    management::{export_database as export_db, import_database as import_db},
};

use crate::export::{
    CsvExporter, JsonExporter, ExportOptions, ExportProgress,
};

/// Application state managed by Tauri
pub struct AppState {
    pub db: Mutex<DatabaseManager>,
    pub mssql_manager: Arc<MssqlConnectionManager>,
    pub query_engine: Arc<QueryEngine>,
    pub schema_manager: Arc<SchemaMetadataManager>,
    pub export_cancel_flags: RwLock<HashMap<String, Arc<AtomicBool>>>,
}

/// Convert StorageError to a string for IPC
impl From<StorageError> for String {
    fn from(err: StorageError) -> Self {
        err.to_string()
    }
}

// ============================================================================
// Space Commands (T006) - Now with integrated connection (1:1)
// ============================================================================

/// Create a new space with optional connection configuration
#[command]
pub async fn create_space(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    // Connection fields
    connection_host: Option<String>,
    connection_port: Option<i32>,
    connection_database: Option<String>,
    connection_username: Option<String>,
    connection_password: Option<String>,
    connection_trust_cert: Option<bool>,
    connection_encrypt: Option<bool>,
) -> Result<Space, String> {
    let space = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.create_space(CreateSpaceInput { 
            name, 
            color, 
            icon,
            connection_host: connection_host.clone(),
            connection_port,
            connection_database: connection_database.clone(),
            connection_username: connection_username.clone(),
            connection_password: connection_password.clone(),
            connection_trust_cert,
            connection_encrypt,
        }).map_err(|e| e.to_string())?
    };
    
    // If connection is configured, register it with the MssqlConnectionManager
    if let (Some(host), Some(database)) = (&space.connection_host, &space.connection_database) {
        let config = ConnectionConfig::new(
            space.name.clone(),
            host.clone(),
            space.connection_port.unwrap_or(1433) as u16,
            database.clone(),
            space.connection_username.clone().unwrap_or_default(),
            connection_password.unwrap_or_default(),
        );
        let mut config = config;
        config.id = space.id.clone(); // Use space ID as connection ID
        config.trust_certificate = space.connection_trust_cert;
        config.encrypt = space.connection_encrypt;
        
        let _ = state.mssql_manager.add_connection(config).await;
    }
    
    Ok(space)
}

/// Get all spaces
#[command]
pub fn get_spaces(state: State<'_, AppState>) -> Result<Vec<Space>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_all_spaces().map_err(|e| e.to_string())
}

/// Update the last active tab for a space
#[command]
pub fn update_space_last_active_tab(
    state: State<'_, AppState>,
    space_id: String,
    tab_id: Option<String>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_space_last_active_tab(&space_id, tab_id.as_deref())
        .map_err(|e| e.to_string())
}

/// Get a single space by ID
#[command]
pub fn get_space(state: State<'_, AppState>, id: String) -> Result<Option<Space>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_space(&id).map_err(|e| e.to_string())
}

/// Update an existing space (including connection)
#[command]
pub async fn update_space(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    sort_order: Option<i32>,
    // Connection fields
    connection_host: Option<String>,
    connection_port: Option<i32>,
    connection_database: Option<String>,
    connection_username: Option<String>,
    connection_password: Option<String>,
    connection_trust_cert: Option<bool>,
    connection_encrypt: Option<bool>,
) -> Result<Option<Space>, String> {
    let space = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_space(&id, UpdateSpaceInput { 
            name, 
            color, 
            icon, 
            sort_order,
            connection_host: connection_host.clone(),
            connection_port,
            connection_database: connection_database.clone(),
            connection_username: connection_username.clone(),
            connection_password: connection_password.clone(),
            connection_trust_cert,
            connection_encrypt,
        }).map_err(|e| e.to_string())?
    };
    
    // Update connection manager if space has connection
    if let Some(ref space) = space {
        // Disconnect existing connection if any
        let _ = state.mssql_manager.disconnect(&id).await;
        
        // Re-register if connection is configured
        if let (Some(host), Some(database)) = (&space.connection_host, &space.connection_database) {
            // Get password from DB (it's not sent back from get)
            let password = {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                db.get_space_password(&id).map_err(|e| e.to_string())?.unwrap_or_default()
            };
            
            let config = ConnectionConfig::new(
                space.name.clone(),
                host.clone(),
                space.connection_port.unwrap_or(1433) as u16,
                database.clone(),
                space.connection_username.clone().unwrap_or_default(),
                password,
            );
            let mut config = config;
            config.id = space.id.clone();
            config.trust_certificate = space.connection_trust_cert;
            config.encrypt = space.connection_encrypt;
            
            let _ = state.mssql_manager.add_connection(config).await;
        }
    }
    
    Ok(space)
}

/// Delete a space by ID
#[command]
pub async fn delete_space(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    // Disconnect any active connection
    let _ = state.mssql_manager.disconnect(&id).await;
    let _ = state.mssql_manager.remove_connection(&id).await;
    
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_space(&id).map_err(|e| e.to_string())
}

/// Reorder spaces
#[command]
pub fn reorder_spaces(state: State<'_, AppState>, space_ids: Vec<String>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reorder_spaces(&space_ids).map_err(|e| e.to_string())
}

// ============================================================================
// Tab Commands (T007)
// ============================================================================

/// Create a new tab
#[command]
pub fn create_tab(
    state: State<'_, AppState>,
    space_id: String,
    title: String,
    tab_type: String,
    content: Option<String>,
    metadata: Option<String>,
    database: Option<String>,
) -> Result<Tab, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let tab_type = TabType::from_str(&tab_type).unwrap_or(TabType::Query);
    db.create_tab(CreateTabInput {
        space_id,
        title,
        tab_type,
        content,
        metadata,
        database,
    })
    .map_err(|e| e.to_string())
}

/// Get all tabs for a space
#[command]
pub fn get_tabs_by_space(state: State<'_, AppState>, space_id: String) -> Result<Vec<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_tabs_by_space(&space_id).map_err(|e| e.to_string())
}

/// Get a single tab by ID
#[command]
pub fn get_tab(state: State<'_, AppState>, id: String) -> Result<Option<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_tab(&id).map_err(|e| e.to_string())
}

/// Update a tab
#[command]
pub fn update_tab(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    content: Option<String>,
    metadata: Option<String>,
    database: Option<String>,
    sort_order: Option<i32>,
) -> Result<Option<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_tab(&id, UpdateTabInput { title, content, metadata, database, sort_order })
        .map_err(|e| e.to_string())
}

/// Update just the database selection for a tab
#[command]
pub fn update_tab_database(
    state: State<'_, AppState>,
    id: String,
    database: Option<String>,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_tab_database(&id, database.as_deref()).map_err(|e| e.to_string())
}

/// Auto-save tab content (optimized for frequent saves)
#[command]
pub fn autosave_tab_content(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.autosave_tab_content(&id, &content).map_err(|e| e.to_string())
}

/// Toggle the pinned status of a tab
#[command]
pub fn toggle_tab_pinned(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.toggle_tab_pinned(&id).map_err(|e| e.to_string())
}

/// Delete a tab by ID
#[command]
pub fn delete_tab(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_tab(&id).map_err(|e| e.to_string())
}

/// Search active tabs by title or content
#[command]
pub fn search_tabs(state: State<'_, AppState>, query: String) -> Result<Vec<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.search_tabs(&query).map_err(|e| e.to_string())
}

/// Reorder tabs within a space
#[command]
pub fn reorder_tabs(
    state: State<'_, AppState>,
    space_id: String,
    tab_ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reorder_tabs(&space_id, &tab_ids).map_err(|e| e.to_string())
}

/// Move a tab to a different space
#[command]
pub fn move_tab_to_space(
    state: State<'_, AppState>,
    tab_id: String,
    new_space_id: String,
) -> Result<Option<Tab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.move_tab_to_space(&tab_id, &new_space_id).map_err(|e| e.to_string())
}

// ============================================================================
// Folder Commands - Arc Browser-style tab organization
// ============================================================================

/// Create a new folder
#[command]
pub fn create_folder(
    state: State<'_, AppState>,
    space_id: String,
    name: String,
) -> Result<TabFolder, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.create_folder(CreateFolderInput { space_id, name })
        .map_err(|e| e.to_string())
}

/// Get all folders for a space
#[command]
pub fn get_folders_by_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<Vec<TabFolder>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_folders_by_space(&space_id).map_err(|e| e.to_string())
}

/// Update a folder
#[command]
pub fn update_folder(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    is_expanded: Option<bool>,
    sort_order: Option<i32>,
) -> Result<Option<TabFolder>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_folder(&id, UpdateFolderInput { name, is_expanded, sort_order })
        .map_err(|e| e.to_string())
}

/// Delete a folder (tabs become ungrouped)
#[command]
pub fn delete_folder(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_folder(&id).map_err(|e| e.to_string())
}

/// Add a tab to a folder (also pins the tab)
#[command]
pub fn add_tab_to_folder(
    state: State<'_, AppState>,
    tab_id: String,
    folder_id: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.add_tab_to_folder(&tab_id, &folder_id).map_err(|e| e.to_string())
}

/// Remove a tab from its folder
#[command]
pub fn remove_tab_from_folder(
    state: State<'_, AppState>,
    tab_id: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_tab_from_folder(&tab_id).map_err(|e| e.to_string())
}

/// Reorder folders within a space
#[command]
pub fn reorder_folders(
    state: State<'_, AppState>,
    space_id: String,
    folder_ids: Vec<String>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reorder_folders(&space_id, &folder_ids).map_err(|e| e.to_string())
}

/// Create a folder and add tabs to it atomically
#[command]
pub fn create_folder_from_tabs(
    state: State<'_, AppState>,
    space_id: String,
    name: String,
    tab_ids: Vec<String>,
) -> Result<TabFolder, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.create_folder_from_tabs(&space_id, &name, &tab_ids).map_err(|e| e.to_string())
}

// ============================================================================
// Connection Commands (T018) - Now focused on space-based connections
// ============================================================================

/// Connect to a space's database (uses space ID as connection ID)
#[command]
pub async fn connect_to_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<bool, String> {
    // Get the space
    let space = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_space(&space_id).map_err(|e| e.to_string())?
    };
    
    let space = match space {
        Some(s) => s,
        None => return Err("Space not found".to_string()),
    };
    
    // Check if space has connection configured
    if !space.has_connection() {
        return Err("Space has no connection configured".to_string());
    }
    
    // Get password from DB (not serialized in Space)
    let password = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_space_password(&space_id).map_err(|e| e.to_string())?.unwrap_or_default()
    };
    
    // Register connection if not exists
    if state.mssql_manager.get_connection(&space_id).await.is_none() {
        let config = ConnectionConfig::new(
            space.name.clone(),
            space.connection_host.clone().unwrap_or_default(),
            space.connection_port.unwrap_or(1433) as u16,
            space.connection_database.clone().unwrap_or_default(),
            space.connection_username.clone().unwrap_or_default(),
            password,
        );
        let mut config = config;
        config.id = space_id.clone();
        config.trust_certificate = space.connection_trust_cert;
        config.encrypt = space.connection_encrypt;
        
        state.mssql_manager.add_connection(config).await.map_err(|e| e.to_string())?;
    }
    
    // Connect
    state.mssql_manager.connect(&space_id).await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// Disconnect from a space's database
#[command]
pub async fn disconnect_from_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<bool, String> {
    state.mssql_manager.disconnect(&space_id).await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// Get connection status for a space
#[command]
pub async fn get_space_connection_status(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<Option<ConnectionInfo>, String> {
    Ok(state.mssql_manager.get_connection(&space_id).await)
}

/// Get list of databases from a space's server connection
#[command]
pub async fn get_space_databases(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<Vec<String>, String> {
    // Ensure connected
    if !state.mssql_manager.is_healthy(&space_id).await {
        // Try to connect first
        let _connected = connect_to_space(state.clone(), space_id.clone()).await?;
    }
    
    state.mssql_manager.get_databases(&space_id)
        .await
        .map_err(|e| e.to_string())
}

/// Legacy: Create a new database connection (kept for flexibility)
#[command]
pub async fn create_connection(
    state: State<'_, AppState>,
    name: String,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
    space_id: Option<String>,
    trust_certificate: Option<bool>,
    encrypt: Option<bool>,
) -> Result<ConnectionInfo, String> {
    let mut config = ConnectionConfig::new(name, host, port, database, username, password);
    config.space_id = space_id;
    config.trust_certificate = trust_certificate.unwrap_or(true);
    config.encrypt = encrypt.unwrap_or(false);
    
    let _id = state.mssql_manager.add_connection(config.clone())
        .await
        .map_err(|e| e.to_string())?;
    
    Ok(ConnectionInfo::from(&config))
}

/// Test a database connection
#[command]
pub async fn test_connection(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
    trust_certificate: Option<bool>,
    encrypt: Option<bool>,
) -> Result<bool, String> {
    let mut config = ConnectionConfig::new(
        "test".to_string(),
        host,
        port,
        database,
        username,
        password,
    );
    config.trust_certificate = trust_certificate.unwrap_or(true);
    config.encrypt = encrypt.unwrap_or(false);
    
    state.mssql_manager.test_connection(&config)
        .await
        .map_err(|e| e.to_string())
}

/// Get all connections
#[command]
pub async fn get_connections(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, String> {
    Ok(state.mssql_manager.list_connections().await)
}

/// Get connections for a specific space
#[command]
pub async fn get_connections_by_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<Vec<ConnectionInfo>, String> {
    Ok(state.mssql_manager.get_connections_by_space(&space_id).await)
}

/// Get a single connection by ID
#[command]
pub async fn get_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ConnectionInfo>, String> {
    Ok(state.mssql_manager.get_connection(&id).await)
}

/// Update a connection
#[command]
pub async fn update_connection(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    database: Option<String>,
    username: Option<String>,
    password: Option<String>,
    space_id: Option<Option<String>>,
    trust_certificate: Option<bool>,
    encrypt: Option<bool>,
) -> Result<ConnectionInfo, String> {
    let updates = ConnectionConfigUpdate {
        name,
        host,
        port,
        database,
        username,
        password,
        trust_certificate,
        encrypt,
        space_id,
    };
    
    state.mssql_manager.update_connection(&id, updates)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a connection
#[command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    state.mssql_manager.remove_connection(&id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Connect to a database
#[command]
pub async fn connect_database(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<bool, String> {
    state.mssql_manager.connect(&connection_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Disconnect from a database
#[command]
pub async fn disconnect_database(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<bool, String> {
    state.mssql_manager.disconnect(&connection_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Get list of databases from a connection
#[command]
pub async fn get_connection_databases(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<String>, String> {
    state.mssql_manager.get_databases(&connection_id)
        .await
        .map_err(|e| e.to_string())
}

/// Check if a connection is healthy
#[command]
pub async fn check_connection_health(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<bool, String> {
    Ok(state.mssql_manager.is_healthy(&connection_id).await)
}

// ============================================================================
// Query Execution Commands (T019)
// ============================================================================

/// Execute a SQL query with optional database context and selected text
/// Supports batch execution - if the query contains multiple statements (separated by GO or semicolons),
/// executes them sequentially and returns multiple results
#[command]
pub async fn execute_query(
    state: State<'_, AppState>,
    connection_id: String,
    query: String,
    database: Option<String>,
    selected_text: Option<String>,
) -> Result<Vec<QueryResult>, String> {
    // Use selected_text if provided, otherwise use full query
    let query_to_execute = selected_text.as_ref().unwrap_or(&query);
    let is_selection = selected_text.is_some();

    state.query_engine.execute_query(&connection_id, query_to_execute, database.as_deref(), is_selection)
        .await
        .map_err(|e| e.to_string())
}

/// Cancel a running query
#[command]
pub async fn cancel_query(
    state: State<'_, AppState>,
    query_id: String,
) -> Result<bool, String> {
    println!("[CMD] cancel_query command called with query_id={}", query_id);
    let result = state.query_engine.cancel_query(&query_id).await;
    println!("[CMD] cancel_query result={} for query_id={}", result, query_id);
    Ok(result)
}

/// Cancel all running queries for a connection (space)
#[command]
pub async fn cancel_queries_for_connection(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<usize, String> {
    println!("[CMD] cancel_queries_for_connection called with connection_id={}", connection_id);
    let count = state.query_engine.cancel_all_for_connection(&connection_id).await;
    println!("[CMD] cancel_queries_for_connection cancelled {} queries", count);
    Ok(count)
}

/// Get the status of a query
#[command]
pub async fn get_query_status(
    state: State<'_, AppState>,
    query_id: String,
) -> Result<Option<QueryInfo>, String> {
    Ok(state.query_engine.get_query_status(&query_id).await)
}

// ============================================================================
// Schema Metadata Commands (T025)
// ============================================================================

/// Get complete schema information for a database (tables, views, columns, routines)
/// Uses caching to avoid repeated queries - schema is cached per connection/database
#[command]
pub async fn get_schema_info(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    schema_filter: Option<String>,
    force_refresh: Option<bool>,
) -> Result<SchemaInfo, String> {
    // Check cache first unless force refresh requested
    if !force_refresh.unwrap_or(false) {
        if let Some(cached) = state.schema_manager.get_cached_schema(&connection_id, &database).await {
            return Ok(cached);
        }
    }
    
    // Fetch fresh schema
    state.schema_manager.fetch_schema(&connection_id, &database, schema_filter.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Get columns for a specific table (more efficient than fetching full schema)
#[command]
pub async fn get_table_columns(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    schema_name: String,
    table_name: String,
) -> Result<Vec<SchemaColumnInfo>, String> {
    state.schema_manager.get_table_columns(&connection_id, &database, &schema_name, &table_name)
        .await
        .map_err(|e| e.to_string())
}

/// Force refresh schema cache for a connection/database
#[command]
pub async fn refresh_schema(
    state: State<'_, AppState>,
    connection_id: String,
    database: Option<String>,
) -> Result<(), String> {
    state.schema_manager.invalidate_cache(&connection_id, database.as_deref()).await;
    Ok(())
}

// ============================================================================
// Export Commands (T034, T035, T036)
// ============================================================================

/// Export query results to CSV file
/// Returns export progress with file path and statistics
#[command]
pub async fn export_to_csv(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
    columns: Vec<crate::db::query::ColumnInfo>,
    rows: Vec<Vec<crate::db::query::CellValue>>,
    options: Option<ExportOptions>,
) -> Result<ExportProgress, String> {
    let export_id = uuid::Uuid::new_v4().to_string();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    
    // Store cancel flag
    {
        let mut flags = state.export_cancel_flags.write().await;
        flags.insert(export_id.clone(), Arc::clone(&cancel_flag));
    }

    let path = PathBuf::from(&file_path);
    let options = options.unwrap_or_default();
    let exporter = CsvExporter::new(options);
    
    // Create progress channel
    let (tx, mut rx) = mpsc::channel::<ExportProgress>(10);
    
    // Spawn progress emitter
    let app_handle = app.clone();
    let export_id_clone = export_id.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = app_handle.emit(&format!("export-progress-{}", export_id_clone), &progress);
        }
    });
    
    // Perform export
    let result = exporter
        .export_to_file_with_progress(&path, &columns, &rows, cancel_flag.clone(), tx)
        .await
        .map_err(|e| e.to_string())?;
    
    // Clean up cancel flag
    {
        let mut flags = state.export_cancel_flags.write().await;
        flags.remove(&export_id);
    }
    
    Ok(result)
}

/// Export query results to JSON file
/// Returns export progress with file path and statistics
#[command]
pub async fn export_to_json(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
    columns: Vec<crate::db::query::ColumnInfo>,
    rows: Vec<Vec<crate::db::query::CellValue>>,
    options: Option<ExportOptions>,
) -> Result<ExportProgress, String> {
    let export_id = uuid::Uuid::new_v4().to_string();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    
    // Store cancel flag
    {
        let mut flags = state.export_cancel_flags.write().await;
        flags.insert(export_id.clone(), Arc::clone(&cancel_flag));
    }

    let path = PathBuf::from(&file_path);
    let options = options.unwrap_or_else(|| ExportOptions {
        pretty_print: true,
        ..Default::default()
    });
    let exporter = JsonExporter::new(options);
    
    // Create progress channel
    let (tx, mut rx) = mpsc::channel::<ExportProgress>(10);
    
    // Spawn progress emitter
    let app_handle = app.clone();
    let export_id_clone = export_id.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = app_handle.emit(&format!("export-progress-{}", export_id_clone), &progress);
        }
    });
    
    // Perform export
    let result = exporter
        .export_to_file_with_progress(&path, &columns, &rows, cancel_flag.clone(), tx)
        .await
        .map_err(|e| e.to_string())?;
    
    // Clean up cancel flag
    {
        let mut flags = state.export_cancel_flags.write().await;
        flags.remove(&export_id);
    }
    
    Ok(result)
}

/// Export query results to string (CSV or JSON) for clipboard copy
/// Useful for quick copy without file dialog
#[command]
pub async fn export_to_string(
    format: String,
    columns: Vec<crate::db::query::ColumnInfo>,
    rows: Vec<Vec<crate::db::query::CellValue>>,
    options: Option<ExportOptions>,
) -> Result<String, String> {
    let options = options.unwrap_or_default();
    
    match format.to_lowercase().as_str() {
        "csv" => {
            let exporter = CsvExporter::new(options);
            exporter.export_to_string(&columns, &rows).map_err(|e| e.to_string())
        }
        "json" => {
            let exporter = JsonExporter::new(ExportOptions {
                pretty_print: true,
                ..options
            });
            exporter.export_to_string(&columns, &rows).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unsupported export format: {}", format)),
    }
}

/// Cancel an ongoing export operation
#[command]
pub async fn cancel_export(
    state: State<'_, AppState>,
    export_id: String,
) -> Result<bool, String> {
    let flags = state.export_cancel_flags.read().await;
    if let Some(flag) = flags.get(&export_id) {
        flag.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false)
    }
}

// ============================================================================
// SQL File Import/Export Commands
// ============================================================================

/// Export tab content to a SQL file
#[command]
pub fn export_tab_as_sql(
    state: State<'_, AppState>,
    tab_id: String,
    file_path: String,
    content: Option<String>,
) -> Result<(), String> {
    let final_content = if let Some(c) = content {
        c
    } else {
        let tab = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.get_tab(&tab_id).map_err(|e| e.to_string())?
        };

        let tab = tab.ok_or_else(|| "Tab not found".to_string())?;
        tab.content.unwrap_or_default()
    };

    std::fs::write(PathBuf::from(&file_path), final_content)
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok(())
}

/// Import SQL file as a new tab
#[command]
pub fn import_sql_file_as_tab(
    state: State<'_, AppState>,
    space_id: String,
    file_path: String,
    title: Option<String>,
) -> Result<Tab, String> {
    let path = PathBuf::from(&file_path);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let default_title = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Query")
        .to_string();

    let tab_title = title.unwrap_or(default_title);

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.create_tab(CreateTabInput {
        space_id,
        title: tab_title,
        tab_type: TabType::Query,
        content: Some(content),
        metadata: None,
        database: None,
    }).map_err(|e| e.to_string())
}

// ============================================================================
// Snippet Commands (T046)
// ============================================================================

/// Get all snippets
#[command]
pub fn get_snippets(state: State<'_, AppState>) -> Result<Vec<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let snippets = db.get_all_snippets().map_err(|e| e.to_string())?;
    println!("[get_snippets] Returning {} snippets", snippets.len());
    Ok(snippets)
}

/// Get only enabled snippets (for editor use)
#[command]
pub fn get_enabled_snippets(state: State<'_, AppState>) -> Result<Vec<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_enabled_snippets().map_err(|e| e.to_string())
}

/// Get a single snippet by ID
#[command]
pub fn get_snippet(state: State<'_, AppState>, id: String) -> Result<Option<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_snippet(&id).map_err(|e| e.to_string())
}

/// Get a snippet by trigger text
#[command]
pub fn get_snippet_by_trigger(state: State<'_, AppState>, trigger: String) -> Result<Option<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_snippet_by_trigger(&trigger).map_err(|e| e.to_string())
}

/// Create a new user-defined snippet
#[command]
pub fn create_snippet(
    state: State<'_, AppState>,
    trigger: String,
    name: String,
    content: String,
    description: Option<String>,
    category: Option<String>,
) -> Result<Snippet, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.create_snippet(CreateSnippetInput {
        trigger,
        name,
        content,
        description,
        category,
    }).map_err(|e| e.to_string())
}

/// Update an existing snippet
#[command]
pub fn update_snippet(
    state: State<'_, AppState>,
    id: String,
    trigger: Option<String>,
    name: Option<String>,
    content: Option<String>,
    description: Option<String>,
    category: Option<String>,
    enabled: Option<bool>,
) -> Result<Option<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_snippet(&id, UpdateSnippetInput {
        trigger,
        name,
        content,
        description,
        category,
        enabled,
    }).map_err(|e| e.to_string())
}

/// Delete a snippet (only user-defined snippets can be deleted)
#[command]
pub fn delete_snippet(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_snippet(&id).map_err(|e| e.to_string())
}

/// Reset a builtin snippet to its default content
#[command]
pub fn reset_builtin_snippet(state: State<'_, AppState>, id: String) -> Result<Option<Snippet>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reset_builtin_snippet(&id).map_err(|e| e.to_string())
}

/// Import snippets from external source (bulk import)
/// Returns the number of snippets imported
#[command]
pub fn import_snippets(
    state: State<'_, AppState>,
    snippets: Vec<CreateSnippetInput>,
) -> Result<usize, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.import_snippets(snippets).map_err(|e| e.to_string())
}

// ============================================================================
// Archive/History Commands - Arc Browser-style tab history
// ============================================================================

/// Archive a tab (move from active to archived)
#[command]
pub async fn archive_tab(
    state: State<'_, AppState>,
    tab_id: String,
) -> Result<ArchivedTab, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.archive_tab(&tab_id).map_err(|e| e.to_string())
}

/// Restore an archived tab back to active tabs
#[command]
pub async fn restore_archived_tab(
    state: State<'_, AppState>,
    archived_id: String,
    target_space_id: Option<String>,
) -> Result<Tab, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // If no target space provided, use the original space from the archived tab
    let target_space = if let Some(space_id) = target_space_id {
        space_id
    } else {
        // Get the original space_id from the archived tab
        db.with_connection(|conn| {
            conn.query_row(
                "SELECT space_id FROM archived_tabs WHERE id = ?",
                [&archived_id],
                |row| row.get::<_, Option<String>>(0),
            )
        })
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Archived tab has no associated space".to_string())?
    };

    db.restore_archived_tab(&archived_id, &target_space)
        .map_err(|e| e.to_string())
}

/// Search archived tabs using FTS5 full-text search
#[command]
pub fn search_archived_tabs(
    state: State<'_, AppState>,
    query: String,
    space_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ArchiveSearchResult>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.search_archived_tabs(&query, space_id.as_deref(), limit)
        .map_err(|e| e.to_string())
}

/// Get archived tabs with optional space filter and pagination
#[command]
pub fn get_archived_tabs(
    state: State<'_, AppState>,
    space_id: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<ArchivedTab>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_archived_tabs(space_id.as_deref(), limit, offset)
        .map_err(|e| e.to_string())
}

/// Get count of archived tabs with optional space filter
#[command]
pub fn get_archived_tabs_count(
    state: State<'_, AppState>,
    space_id: Option<String>,
) -> Result<usize, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_archived_tabs_count(space_id.as_deref())
        .map_err(|e| e.to_string())
}

/// Permanently delete an archived tab
#[command]
pub fn delete_archived_tab(
    state: State<'_, AppState>,
    archived_id: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_archived_tab(&archived_id)
        .map_err(|e| e.to_string())
}

/// Update last_accessed_at timestamp for a tab (activity tracking)
#[command]
pub fn touch_tab(
    state: State<'_, AppState>,
    tab_id: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.touch_tab(&tab_id).map_err(|e| e.to_string())
}

/// Get auto-archive settings
#[command]
pub fn get_auto_archive_settings(
    state: State<'_, AppState>,
) -> Result<AutoArchiveSettings, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_auto_archive_settings().map_err(|e| e.to_string())
}

/// Update auto-archive settings
#[command]
pub fn update_auto_archive_settings(
    state: State<'_, AppState>,
    enabled: bool,
    days_inactive: i32,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_auto_archive_settings(enabled, days_inactive)
        .map_err(|e| e.to_string())
}

// ============================================================================
// App Settings Commands
// ============================================================================

/// Get app settings (validation, last opened workspace/tab)
#[command]
pub fn get_app_settings(
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_app_settings().map_err(|e| e.to_string())
}

/// Update app settings
#[command]
pub fn update_app_settings(
    state: State<'_, AppState>,
    validation_enabled: bool,
    last_space_id: Option<String>,
    last_tab_id: Option<String>,
    enable_sticky_notes: bool,
    max_result_rows: i32,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let settings = AppSettings {
        validation_enabled,
        last_space_id,
        last_tab_id,
        enable_sticky_notes,
        max_result_rows,
    };
    db.update_app_settings(&settings).map_err(|e| e.to_string())
}

// ============================================================================
// Database Management Commands
// ============================================================================

/// Export the entire application database to a file
#[command]
pub fn export_database(
    state: State<'_, AppState>,
    destination: String,
) -> Result<(), String> {
    export_db(&state, &destination)
}

/// Import an application database from a file and restart the application
#[command]
pub fn import_database(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    source: String,
) -> Result<(), String> {
    import_db(&app_handle, &state, &source)
}
