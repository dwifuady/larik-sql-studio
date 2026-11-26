//! Policy resolution, per-space concurrency limiting, secret hydration, and audit logging
//! for `larik-mcp`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use larik_core::mcp::{
    MCP_MAX_CONCURRENT_PER_SPACE, McpAccess, McpLogRecord, McpSpaceRecord, mcp_log_status,
};
use larik_core::run_log::RunLogRecord;
use larik_core::store::StudioStore;
use larik_data::{DatabaseConnection, DatabaseProvider, ProviderCatalog};
use tokio::sync::Semaphore;

pub struct StudioContext {
    pub store: Arc<StudioStore>,
    pub catalog: ProviderCatalog,
    pub pipe_name: String,
    space_gates: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl StudioContext {
    pub fn new(store: Arc<StudioStore>, catalog: ProviderCatalog, pipe_name: String) -> Self {
        Self {
            store,
            catalog,
            pipe_name,
            space_gates: Mutex::new(HashMap::new()),
        }
    }

    /// Returns all workspaces exposed to AI agents, failing closed if global MCP is disabled.
    pub fn exposed_spaces(&self) -> Result<Vec<McpSpaceRecord>> {
        if !self.store.is_mcp_enabled() {
            return Err(anyhow!(
                "AI agent access is turned off in Larik SQL Studio. Turn on Settings -> AI agents to expose workspaces."
            ));
        }

        let spaces = self
            .store
            .get_mcp_spaces()
            .into_iter()
            .filter(|s| s.access != McpAccess::Off && s.has_connection)
            .collect();
        Ok(spaces)
    }

    /// Resolves one exposed workspace + its provider, auditing a `denied` entry when refused.
    pub fn require_space(
        &self,
        tool: &str,
        connection_id: &str,
    ) -> Result<(McpSpaceRecord, Arc<dyn DatabaseProvider>)> {
        let trimmed = connection_id.trim();
        if trimmed.is_empty() {
            self.audit_global(
                tool,
                mcp_log_status::DENIED,
                Some("Missing required 'connection' parameter.".into()),
                Duration::ZERO,
                0,
            );
            return Err(anyhow!(
                "Missing required 'connection' parameter. Call list_connections first."
            ));
        }

        let spaces = match self.exposed_spaces() {
            Ok(s) => s,
            Err(err) => {
                self.audit_global(
                    tool,
                    mcp_log_status::DENIED,
                    Some(err.to_string()),
                    Duration::ZERO,
                    0,
                );
                return Err(err);
            }
        };

        let Some(space) = spaces.into_iter().find(|s| s.id == trimmed) else {
            let msg = format!(
                "Connection '{trimmed}' is not exposed to AI agents. Call list_connections to see available connections."
            );
            self.audit_global(
                tool,
                mcp_log_status::DENIED,
                Some(msg.clone()),
                Duration::ZERO,
                0,
            );
            return Err(anyhow!(msg));
        };

        let Some(provider) = self.catalog.find(&space.provider_id) else {
            let msg = format!(
                "Provider '{}' for workspace '{}' is not available in this build.",
                space.provider_id, space.name
            );
            self.audit(
                tool,
                &space,
                None,
                mcp_log_status::ERROR,
                Some(msg.clone()),
                Duration::ZERO,
                0,
                None,
            );
            return Err(anyhow!(msg));
        };

        Ok((space, provider))
    }

    fn semaphore_for(&self, space_id: &str) -> Arc<Semaphore> {
        let mut map = self.space_gates.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(space_id.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(MCP_MAX_CONCURRENT_PER_SPACE)))
            .clone()
    }

    /// Opens an isolated one-shot `DatabaseConnection` for `space` under the per-space concurrency cap,
    /// hydrates secrets from the OS secret backend just-in-time, runs `op`, and closes the connection.
    pub async fn with_connection<T, F, Fut>(
        &self,
        space: &McpSpaceRecord,
        provider: &Arc<dyn DatabaseProvider>,
        op: F,
    ) -> Result<T>
    where
        F: FnOnce(Box<dyn DatabaseConnection>) -> Fut,
        Fut: Future<Output = (Box<dyn DatabaseConnection>, Result<T>)>,
    {
        let sem = self.semaphore_for(&space.id);
        let _permit = sem
            .acquire_owned()
            .await
            .map_err(|_| anyhow!("MCP connection limiter closed."))?;

        let profile = self
            .store
            .load_space_profile_with_secrets(space, &self.catalog);
        let mut conn = provider.create_connection(profile);
        conn.open().await?;

        let (mut conn, outcome) = op(conn).await;
        let _ = conn.close().await;
        outcome
    }

    pub fn audit_global(
        &self,
        tool: &str,
        status: &str,
        reason: Option<String>,
        elapsed: Duration,
        rows: usize,
    ) {
        let rec = McpLogRecord {
            id: 0,
            recorded_at: String::new(),
            tool: tool.to_string(),
            space_id: None,
            space_name: None,
            provider_id: None,
            access_level: McpAccess::Off.as_storage().to_string(),
            statement: None,
            status: status.to_string(),
            reason,
            duration_ms: elapsed.as_millis().min(i64::MAX as u128) as i64,
            rows_returned: rows as i64,
            decision_surface: None,
        };
        let _ = self.store.log_mcp_call(&rec);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn audit(
        &self,
        tool: &str,
        space: &McpSpaceRecord,
        statement: Option<&str>,
        status: &str,
        reason: Option<String>,
        elapsed: Duration,
        rows: usize,
        decision_surface: Option<&str>,
    ) {
        let rec = McpLogRecord {
            id: 0,
            recorded_at: String::new(),
            tool: tool.to_string(),
            space_id: Some(space.id.clone()),
            space_name: Some(space.name.clone()),
            provider_id: Some(space.provider_id.clone()),
            access_level: space.access.as_storage().to_string(),
            statement: statement.map(str::to_string),
            status: status.to_string(),
            reason,
            duration_ms: elapsed.as_millis().min(i64::MAX as u128) as i64,
            rows_returned: rows as i64,
            decision_surface: decision_surface.map(str::to_string),
        };
        let _ = self.store.log_mcp_call(&rec);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_run(
        &self,
        space: &McpSpaceRecord,
        database: Option<&str>,
        statement: &str,
        status: &str,
        reason: Option<String>,
        elapsed: Duration,
        rows_returned: usize,
        row_limit: Option<usize>,
        is_truncated: bool,
        description: Option<String>,
        reference: Option<String>,
    ) {
        let rec = RunLogRecord::new_agent_run(
            Some(space.id.clone()),
            Some(space.name.clone()),
            database.map(str::to_string),
            Some(space.provider_id.clone()),
            statement.to_string(),
            status,
            reason,
            elapsed.as_millis().min(i64::MAX as u128) as i64,
            rows_returned as i64,
            row_limit.map(|v| v as i64),
            is_truncated,
            description,
            reference,
        );
        let _ = self.store.log_run(&rec);
    }
}
