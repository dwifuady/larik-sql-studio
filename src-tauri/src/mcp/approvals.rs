// Human-in-the-loop approval queue for MCP write access.
//
// Spaces exposed with `write` access hand the saved login's own permissions to
// the agent. To keep a human in the loop, every `run_query` on such a space is
// parked here and surfaced as a dialog in the running Larik window; the MCP
// request blocks until the user approves or denies, or the wait times out.
// Every outcome is audited by the caller: granted → the normal ok/error row,
// denied/timed out → a `denied` row.
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, UserAttentionType};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::oneshot;

use crate::mcp::McpState;
/// How long an MCP request waits for a human decision before auto-denying.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// One approval request, as shown in the dialog and emitted to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub connection_id: String,
    pub space_name: String,
    pub driver: String,
    /// Statement text, truncated by the caller before registering.
    pub statement: String,
    pub access_level: String,
    pub created_at: String,
}

pub struct Pending {
    #[allow(dead_code)] // kept for inspection/debugging of the queue
    request: ApprovalRequest,
    tx: oneshot::Sender<bool>,
}

/// Park a request and remember how to wake the waiting MCP handler.
/// Entries whose waiter vanished (client disconnected mid-request) are purged
/// here so abandoned calls cannot accumulate.
pub(crate) fn register(state: &McpState, request: ApprovalRequest, tx: oneshot::Sender<bool>) {
    state
        .pending_approvals
        .lock()
        .retain(|_, pending| !pending.tx.is_closed());
    state
        .pending_approvals
        .lock()
        .insert(request.id.clone(), Pending { request, tx });
}

/// Deliver a user decision. Returns `false` when the id is unknown or the
/// waiter is already gone (e.g. timed out a moment earlier).
pub fn resolve(state: &McpState, id: &str, approved: bool) -> bool {
    state
        .pending_approvals
        .lock()
        .remove(id)
        .is_some_and(|pending| pending.tx.send(approved).is_ok())
}

/// Currently pending requests (for UI rehydration after a reload).
pub fn list(state: &McpState) -> Vec<ApprovalRequest> {
    state
        .pending_approvals
        .lock()
        .values()
        .map(|pending| pending.request.clone())
        .collect()
}

/// Register, surface in the UI, and wait for the decision.
///
/// `Ok(true)` — user approved. `Ok(false)` — user denied (or the waiter was
/// dropped during shutdown). `Err(message)` — nobody answered in time (auto-
/// deny; the queue entry is cleaned up) or the UI is not attached.
pub(crate) async fn request_approval(
    state: &McpState,
    request: ApprovalRequest,
    timeout: Duration,
) -> Result<bool, String> {
    let preview: String = request
        .statement
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(140)
        .collect();

    {
        let handle = state.app_handle.lock();
        let Some(handle) = handle.as_ref() else {
            return Err("approval prompt unavailable: Larik UI is not attached".to_string());
        };
        if let Err(e) = handle.emit("mcp-approval-request", &request) {
            // The queue still holds the entry, so a rehydrated dialog (or the
            // pending list) can still answer; log and keep waiting.
            eprintln!("[MCP] failed to emit approval event: {e}");
        }

        // The user may not be looking at the Larik window: raise a native
        // Windows toast and flash the taskbar (window not focused) so the
        // request cannot go unnoticed. Failure to notify must not break the
        // approval flow — the in-app dialog is the decision surface.
        if let Err(e) = handle
            .notification()
            .builder()
            .title("MCP approval requested")
            .body(format!("{} on '{}'", preview, request.space_name))
            .show()
        {
            eprintln!("[MCP] failed to show approval notification: {e}");
        }
        if let Some(window) = handle.get_webview_window("main") {
            if !window.is_focused().unwrap_or(true) {
                let _ = window.request_user_attention(Some(UserAttentionType::Critical));
            }
        }
    }

    println!(
        "[MCP] approval requested [{}] on '{}': {}",
        request.id, request.space_name, preview
    );

    let rx = {
        let (tx, rx) = oneshot::channel();
        register(state, request.clone(), tx);
        rx
    };

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(approved)) => {
            println!(
                "[MCP] approval [{}] {}",
                request.id,
                if approved { "granted" } else { "denied" }
            );
            Ok(approved)
        }
        // Responder dropped (app shutting down) — treat as a denial.
        Ok(Err(_)) => Ok(false),
        Err(_) => {
            state.pending_approvals.lock().remove(&request.id);
            println!("[MCP] approval [{}] timed out", request.id);
            Err(format!(
                "timed out after {}s waiting for approval in Larik (auto-denied)",
                timeout.as_secs()
            ))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn req(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.to_string(),
            connection_id: "c1".to_string(),
            space_name: "Local".to_string(),
            driver: "mssql".to_string(),
            statement: "DELETE FROM users".to_string(),
            access_level: "write".to_string(),
            created_at: "2026-10-13T20:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn register_resolve_roundtrip_grants() {
        let state = Arc::new(McpState::for_tests());
        let (tx, rx) = oneshot::channel();
        register(&state, req("a1"), tx);
        assert_eq!(list(&state).len(), 1);
        assert_eq!(list(&state)[0].space_name, "Local");
        assert!(resolve(&state, "a1", true));
        assert_eq!(rx.await.unwrap(), true);
        assert!(list(&state).is_empty());
    }

    #[tokio::test]
    async fn resolve_denies_and_unknown_ids_return_false() {
        let state = Arc::new(McpState::for_tests());
        assert!(!resolve(&state, "ghost", true));

        let (tx, rx) = oneshot::channel();
        register(&state, req("a2"), tx);
        assert!(resolve(&state, "a2", false));
        assert_eq!(rx.await.unwrap(), false);
        // second resolve: entry already gone
        assert!(!resolve(&state, "a2", true));
    }

    #[tokio::test]
    async fn multiple_pendings_are_listed_independently() {
        let state = Arc::new(McpState::for_tests());
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        register(&state, req("a3"), tx1);
        register(&state, req("a4"), tx2);
        let pending = list(&state);
        assert_eq!(pending.len(), 2);
        assert!(resolve(&state, "a4", true));
        assert_eq!(list(&state).len(), 1);
        assert_eq!(list(&state)[0].id, "a3");
    }
}
