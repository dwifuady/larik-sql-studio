// Embedded MCP (Model Context Protocol) server.
//
// Lets local AI agents (Claude Code, …) use Larik's saved database connections
// over streamable HTTP on 127.0.0.1, gated by two user-controlled switches:
// a global `mcp_enabled` setting (default off) and a per-space access level
// (`off` / `read` / `write`). See MCP_DESIGN.md at the repo root.

pub mod policy;
pub mod server;
pub mod tools;

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::RwLock;

use crate::db::{ConnectionManager, QueryEngine, SchemaMetadataManager};
use crate::storage::DatabaseManager;

/// Default TCP port for the MCP listener.
pub const DEFAULT_PORT: u16 = 7487;

/// Shared state for MCP tool handlers — the same `Arc`s the UI uses, so
/// connection pools, schema caches and settings stay single-sourced.
pub struct McpState {
    pub connections: Arc<ConnectionManager>,
    pub query_engine: Arc<QueryEngine>,
    pub schema_manager: Arc<SchemaMetadataManager>,
    /// The same storage instance `AppState` holds (settings, spaces, audit log).
    pub db: Arc<tokio::sync::Mutex<DatabaseManager>>,
    /// Bearer token; swapped live on regeneration without a restart.
    pub token: RwLock<String>,
}

struct Running {
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tauri::async_runtime::JoinHandle<()>,
}

/// Starts/stops the HTTP listener when the feature toggle flips.
pub struct McpSupervisor {
    state: Arc<McpState>,
    running: Mutex<Option<Running>>,
}

impl McpSupervisor {
    pub fn new(state: Arc<McpState>) -> Self {
        Self {
            state,
            running: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    /// Bind and serve. Bind errors (port taken, …) surface here so the UI can
    /// show them; a silent port fallback would strand the client config.
    pub async fn start(&self) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }
        let port = {
            let db = self.state.db.lock().await;
            db.get_mcp_port().map_err(|e| e.to_string())?
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("cannot bind 127.0.0.1:{port}: {e}"))?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let state = Arc::clone(&self.state);
        let handle = tauri::async_runtime::spawn(async move {
            if let Err(e) = server::serve(listener, state, shutdown_rx).await {
                eprintln!("[MCP] server error: {e}");
            }
        });
        *self.running.lock() = Some(Running {
            shutdown: shutdown_tx,
            handle,
        });
        println!("[MCP] listening on http://127.0.0.1:{port}/mcp");
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(running) = self.running.lock().take() {
            let _ = running.shutdown.send(());
            running.handle.abort();
            println!("[MCP] server stopped");
        }
    }

    pub async fn set_token(&self, token: String) {
        *self.state.token.write().await = token;
    }

    pub fn state(&self) -> Arc<McpState> {
        Arc::clone(&self.state)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
impl McpState {
    /// Test fixture: temp storage, real drivers, token "tok".
    pub fn for_tests() -> Self {
        let temp = std::env::temp_dir();
        let db = DatabaseManager::new(
            temp.join(format!("larik_mcp_state_test_{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let connections = Arc::new(ConnectionManager::new(
            crate::db::DriverRegistry::with_builtin_drivers(),
        ));
        let schema_manager = Arc::new(SchemaMetadataManager::new(
            Arc::clone(&connections),
            Arc::new(
                DatabaseManager::new(
                    temp.join(format!("larik_mcp_schema_test_{}.db", uuid::Uuid::new_v4())),
                )
                .unwrap(),
            ),
        ));
        let query_engine = Arc::new(QueryEngine::new(Arc::clone(&connections)));
        Self {
            connections,
            query_engine,
            schema_manager,
            db: Arc::new(tokio::sync::Mutex::new(db)),
            token: RwLock::new("tok".to_string()),
        }
    }
}
