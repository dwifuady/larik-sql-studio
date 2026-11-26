pub mod approval_channel;
pub mod context;
pub mod server;
pub mod tools;

use std::sync::Arc;

use larik_data::ProviderCatalog;
use larik_provider_demo::DemoProvider;
use larik_provider_mongodb::MongoProvider;
use larik_provider_mssql::SqlServerProvider;
use larik_provider_postgres::PostgresProvider;

pub fn build_default_catalog() -> ProviderCatalog {
    let mut catalog = ProviderCatalog::new();
    catalog.register(Arc::new(DemoProvider));
    catalog.register(Arc::new(SqlServerProvider));
    catalog.register(Arc::new(PostgresProvider));
    catalog.register(Arc::new(MongoProvider));
    catalog
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use larik_core::StudioState;
    use larik_core::mcp::{
        McpAccess, McpApprovalDecision, McpOpenResponse, McpPipeIncoming, mcp_approval_status,
        mcp_log_status,
    };
    use larik_core::run_log::{run_actor, run_status};
    use larik_core::secrets::InMemorySecretBackend;
    use larik_core::store::StudioStore;
    use serde_json::{Value, json};

    use crate::build_default_catalog;
    use crate::context::StudioContext;
    use crate::server::handle_request;

    fn test_context(pipe_name: String) -> (Arc<StudioContext>, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("larik_mcp_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("studio.db");
        let catalog = build_default_catalog();
        let store = Arc::new(
            StudioStore::open_at_with_secrets(db, Arc::new(InMemorySecretBackend::default()))
                .unwrap(),
        );
        let state = StudioState::seed_default(&catalog);
        let demo_space_id = state.spaces[0].id.clone();
        store.save(&state, &catalog).unwrap();
        let ctx = Arc::new(StudioContext::new(store, catalog, pipe_name));
        (ctx, demo_space_id, dir)
    }

    async fn call(ctx: &StudioContext, tool: &str, args: Value) -> (bool, String) {
        let resp = handle_request(
            ctx,
            json!(1),
            "tools/call",
            &json!({ "name": tool, "arguments": args }),
        )
        .await;
        let is_err = resp["result"]["isError"].as_bool().unwrap_or(true);
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        (is_err, text)
    }

    #[tokio::test]
    async fn fails_closed_when_global_mcp_is_off() {
        let (ctx, space_id, dir) = test_context(r"\\.\pipe\larik.test.unused.1".into());
        ctx.store
            .set_space_mcp_access(&space_id, McpAccess::Read)
            .unwrap();

        let (is_err, msg) = call(&ctx, "list_connections", json!({})).await;
        assert!(is_err);
        assert!(msg.contains("turned off"));

        let logs = ctx.store.get_mcp_log(10);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, mcp_log_status::DENIED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_only_workspace_lists_schema_runs_select_and_refuses_writes() {
        let (ctx, space_id, dir) = test_context(r"\\.\pipe\larik.test.unused.2".into());
        ctx.store.set_mcp_enabled(true).unwrap();
        ctx.store
            .set_space_mcp_access(&space_id, McpAccess::Read)
            .unwrap();

        let (is_err, text) = call(&ctx, "list_connections", json!({})).await;
        assert!(!is_err, "unexpected error: {text}");
        let conns: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(conns.as_array().unwrap().len(), 1);
        assert_eq!(conns[0]["id"], space_id);
        assert_eq!(conns[0]["access"], "read");

        let (is_err, text) = call(&ctx, "list_databases", json!({ "connection": space_id })).await;
        assert!(!is_err, "unexpected error: {text}");
        let dbs: Value = serde_json::from_str(&text).unwrap();
        assert!(dbs.as_array().unwrap().len() >= 2);

        let (is_err, text) = call(
            &ctx,
            "get_schema",
            json!({
                "connection": space_id,
                "database": "larik_prod",
                "table": "dbo.orders",
                "include_routines": true
            }),
        )
        .await;
        assert!(!is_err, "unexpected error: {text}");
        let schema: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(schema["tables"].as_array().unwrap().len(), 1);
        assert_eq!(schema["tables"][0]["name"], "orders");
        assert!(schema["routines"].as_array().unwrap().len() >= 2);

        let (is_err, text) = call(
            &ctx,
            "run_query",
            json!({
                "connection": space_id,
                "database": "larik_prod",
                "query": "SELECT TOP 5 * FROM dbo.customers",
                "description": "Inspect top customers",
                "reference": "PROJ-42"
            }),
        )
        .await;
        assert!(!is_err, "unexpected error: {text}");
        let qr: Value = serde_json::from_str(&text).unwrap();
        assert!(!qr["resultSets"].as_array().unwrap().is_empty());

        let runs = ctx.store.get_run_log(Some(&space_id), 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].actor, run_actor::AGENT);
        assert_eq!(runs[0].status, run_status::OK);
        assert_eq!(runs[0].description.as_deref(), Some("Inspect top customers"));
        assert_eq!(runs[0].reference.as_deref(), Some("PROJ-42"));

        let (is_err, msg) = call(
            &ctx,
            "run_query",
            json!({
                "connection": space_id,
                "query": "DELETE FROM dbo.customers"
            }),
        )
        .await;
        assert!(is_err);
        assert!(msg.contains("Read only"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_workspace_requires_description_and_fails_closed_when_studio_closed() {
        let pipe_name = format!(r"\\.\pipe\larik.test.closed.{}", uuid::Uuid::new_v4());
        let (ctx, space_id, dir) = test_context(pipe_name);
        ctx.store.set_mcp_enabled(true).unwrap();
        ctx.store
            .set_space_mcp_access(&space_id, McpAccess::Write)
            .unwrap();

        let (is_err, msg) = call(
            &ctx,
            "run_query",
            json!({
                "connection": space_id,
                "query": "SELECT 1"
            }),
        )
        .await;
        assert!(is_err);
        assert!(msg.contains("description"));

        let (is_err, msg) = call(
            &ctx,
            "run_query",
            json!({
                "connection": space_id,
                "query": "SELECT 1",
                "description": "Verify connectivity",
                "reference": "INC-9"
            }),
        )
        .await;
        assert!(is_err);
        assert!(msg.contains("not running"));

        let runs = ctx.store.get_run_log(Some(&space_id), 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, run_status::REFUSED);
        assert_eq!(runs[0].reference.as_deref(), Some("INC-9"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn write_approval_and_open_in_studio_round_trip_over_named_pipe() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::windows::named_pipe::ServerOptions;

        let pipe_name = format!(r"\\.\pipe\larik.test.live.{}", uuid::Uuid::new_v4());
        let (ctx, space_id, dir) = test_context(pipe_name.clone());
        ctx.store.set_mcp_enabled(true).unwrap();
        ctx.store
            .set_space_mcp_access(&space_id, McpAccess::Write)
            .unwrap();

        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
            .unwrap();
        let server_pipe = pipe_name.clone();
        let server_task = tokio::spawn(async move {
            for _ in 0..2 {
                server.connect().await.unwrap();
                let connected = server;
                server = ServerOptions::new().create(&server_pipe).unwrap();

                let (r, mut w) = tokio::io::split(connected);
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();

                let reply = match McpPipeIncoming::parse_line(line.trim()).unwrap() {
                    McpPipeIncoming::Approval(req) => {
                        assert_eq!(req.reference.as_deref(), Some("FIX-101"));
                        serde_json::to_string(&McpApprovalDecision::approved(req.id)).unwrap()
                    }
                    McpPipeIncoming::Open(req) => {
                        assert_eq!(req.statement, "SELECT 1 AS one");
                        serde_json::to_string(&McpOpenResponse::ok(req.id)).unwrap()
                    }
                };
                w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
                w.flush().await.unwrap();
            }
        });

        let (is_err, text) = call(
            &ctx,
            "run_query",
            json!({
                "connection": space_id,
                "database": "larik_prod",
                "query": "SELECT TOP 3 * FROM dbo.orders",
                "description": "Check recent orders",
                "reference": "FIX-101"
            }),
        )
        .await;
        assert!(!is_err, "approved write failed: {text}");

        let runs = ctx.store.get_run_log(Some(&space_id), 10);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, run_status::OK);
        assert_eq!(runs[0].reference.as_deref(), Some("FIX-101"));

        let (is_err, text) = call(
            &ctx,
            "open_in_studio",
            json!({
                "connection": space_id,
                "query": "SELECT 1 AS one"
            }),
        )
        .await;
        assert!(!is_err, "open_in_studio failed: {text}");
        let opened: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(opened["opened"], true);

        server_task.await.unwrap();

        let logs = ctx.store.get_mcp_log(10);
        assert!(logs.iter().any(|l| l.tool == "run_query" && l.status == mcp_log_status::OK));
        assert!(logs.iter().any(|l| l.tool == "open_in_studio" && l.status == mcp_log_status::OK));

        assert_eq!(ctx.store.sweep_mcp_approvals(), 0);
        let _ = mcp_approval_status::APPROVED;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_catalog_registers_all_builtin_providers_with_read_only_policies() {
        let catalog = build_default_catalog();
        for id in ["demo", "mssql", "postgres", "mongodb"] {
            let provider = catalog
                .find(id)
                .unwrap_or_else(|| panic!("expected built-in provider '{id}' in catalog"));
            assert!(
                provider.read_only_policy().is_some(),
                "expected provider '{id}' to provide a fail-closed read-only policy"
            );
        }
    }
}
