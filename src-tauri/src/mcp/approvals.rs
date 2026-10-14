// Human-in-the-loop approval queue for MCP write access.
//
// Spaces exposed with `write` access hand the saved login's own permissions to
// the agent. To keep a human in the loop, every `run_query` on such a space is
// parked here and surfaced as a dialog in the running Larik window; the MCP
// request blocks until the user approves or denies, or the wait times out.
//
// The full request lifecycle is persisted in `mcp_approvals` (pending →
// approved/denied/timeout, plus abandoned when the MCP client vanished and
// orphaned when the app died before a decision), so even requests nobody ever
// answered leave a trace. Every outcome is also audited by the caller: granted
// → the normal ok/error row, denied/timed out → a `denied` row.

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

/// What a wait produced, for the audit trail.
#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    pub approved: bool,
    /// Human time: prompt shown → decision (user click or auto-deny).
    pub latency_ms: u64,
    pub decided_at: String,
    pub reason: &'static str,
}

pub struct Pending {
    request: ApprovalRequest,
    tx: oneshot::Sender<bool>,
}

/// Persisted status values for `mcp_approvals.status`.
const STATUS_PENDING: &str = "pending";
const STATUS_APPROVED: &str = "approved";
const STATUS_DENIED: &str = "denied";
const STATUS_TIMEOUT: &str = "timeout";
const STATUS_ABANDONED: &str = "abandoned";
const STATUS_ORPHANED: &str = "orphaned";

async fn db_insert_pending(state: &McpState, request: &ApprovalRequest) {
    let db = state.db.lock().await;
    if let Err(e) = db.insert_mcp_approval(request, STATUS_PENDING) {
        eprintln!("[MCP] failed to persist approval request: {e}");
    }
}

async fn db_update_status(state: &McpState, id: &str, status: &str, decided_at: &str) {
    let db = state.db.lock().await;
    if let Err(e) = db.update_mcp_approval_status(id, status, decided_at) {
        eprintln!("[MCP] failed to update approval status: {e}");
    }
}

/// Park a request, persist it as `pending`, and remember how to wake the
/// waiting MCP handler. Entries whose waiter vanished (client disconnected
/// mid-request) are purged here and marked `abandoned` so abandoned calls
/// cannot accumulate silently.
async fn register(state: &McpState, request: ApprovalRequest, tx: oneshot::Sender<bool>) {
    // Purge earlier waiters whose client vanished and mark them `abandoned`.
    let abandoned: Vec<String> = {
        let mut queue = state.pending_approvals.lock();
        let mut abandoned = Vec::new();
        queue.retain(|id, pending| {
            if pending.tx.is_closed() {
                abandoned.push(id.clone());
                false
            } else {
                true
            }
        });
        abandoned
    };
    for id in &abandoned {
        db_update_status(state, id, STATUS_ABANDONED, "").await;
    }

    // Persist first (status `pending`), then park in memory — unless this
    // request's own waiter is already gone (client disconnected between
    // park and now), in which case it goes straight to `abandoned`.
    db_insert_pending(state, &request).await;
    let closed = tx.is_closed();
    {
        let mut queue = state.pending_approvals.lock();
        if closed {
            queue.remove(&request.id);
        } else {
            queue.insert(
                request.id.clone(),
                Pending {
                    request: request.clone(),
                    tx,
                },
            );
        }
    }
    if closed {
        db_update_status(state, &request.id, STATUS_ABANDONED, "").await;
    }
}

/// Deliver a user decision. Returns `false` when the id is unknown or the
/// waiter is already gone (e.g. timed out a moment earlier).
pub async fn resolve(state: &McpState, id: &str, approved: bool) -> bool {
    let delivered = state
        .pending_approvals
        .lock()
        .remove(id)
        .is_some_and(|pending| pending.tx.send(approved).is_ok());
    if delivered {
        let decided_at = chrono::Utc::now().to_rfc3339();
        db_update_status(
            state,
            id,
            if approved {
                STATUS_APPROVED
            } else {
                STATUS_DENIED
            },
            &decided_at,
        )
        .await;
    }
    delivered
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

/// Mark requests still `pending` after an app restart as `orphaned` — the
/// process died before any decision, and the waiting MCP client is gone.
pub async fn mark_orphaned(state: &McpState) {
    let db = state.db.lock().await;
    if let Err(e) = db.orphan_pending_mcp_approvals(STATUS_ORPHANED) {
        eprintln!("[MCP] failed to sweep orphaned approvals: {e}");
    }
}

/// Register, surface in the UI, and wait for the decision.
///
/// `Ok(outcome)` — a decision was reached: user approved (`approved: true`),
/// user denied, or the wait timed out (auto-deny). `Err(message)` — no
/// decision was ever possible (UI not attached); nothing waited.
pub(crate) async fn request_approval(
    state: &McpState,
    request: ApprovalRequest,
    timeout: Duration,
) -> Result<ApprovalOutcome, String> {
    let started = std::time::Instant::now();
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
        // register() persists the request as `pending` and marks any
        // abandoned waiters from earlier calls.
        register(state, request.clone(), tx).await;
        rx
    };

    let decided_at = || chrono::Utc::now().to_rfc3339();
    let outcome = |approved: bool, reason: &'static str| ApprovalOutcome {
        approved,
        latency_ms: started.elapsed().as_millis() as u64,
        decided_at: decided_at(),
        reason,
    };

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(approved)) => {
            println!(
                "[MCP] approval [{}] {}",
                request.id,
                if approved { "granted" } else { "denied" }
            );
            db_update_status(
                state,
                &request.id,
                if approved {
                    STATUS_APPROVED
                } else {
                    STATUS_DENIED
                },
                &decided_at(),
            )
            .await;
            Ok(outcome(approved, "user decision"))
        }
        // Responder dropped (app shutting down) — treat as a denial.
        Ok(Err(_)) => {
            db_update_status(state, &request.id, STATUS_DENIED, &decided_at()).await;
            Ok(outcome(false, "approval waiter dropped during shutdown"))
        }
        Err(_) => {
            state.pending_approvals.lock().remove(&request.id);
            println!("[MCP] approval [{}] timed out", request.id);
            db_update_status(state, &request.id, STATUS_TIMEOUT, &decided_at()).await;
            Ok(outcome(
                false,
                "timed out waiting for a decision (auto-denied)",
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
        register(&state, req("a1"), tx).await;
        assert_eq!(list(&state).len(), 1);
        assert_eq!(list(&state)[0].space_name, "Local");
        assert!(resolve(&state, "a1", true).await);
        assert_eq!(rx.await.unwrap(), true);
        assert!(list(&state).is_empty());

        // lifecycle persisted: pending → approved
        let db = state.db.lock().await;
        let rows = db.list_mcp_approvals(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, STATUS_APPROVED);
        assert!(rows[0].decided_at.is_some());
    }

    #[tokio::test]
    async fn resolve_denies_and_unknown_ids_return_false() {
        let state = Arc::new(McpState::for_tests());
        assert!(!resolve(&state, "ghost", true).await);

        let (tx, rx) = oneshot::channel();
        register(&state, req("a2"), tx).await;
        assert!(resolve(&state, "a2", false).await);
        assert_eq!(rx.await.unwrap(), false);
        // second resolve: entry already gone
        assert!(!resolve(&state, "a2", true).await);
    }

    #[tokio::test]
    async fn multiple_pendings_are_listed_independently() {
        let state = Arc::new(McpState::for_tests());
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        register(&state, req("a3"), tx1).await;
        register(&state, req("a4"), tx2).await;
        let pending = list(&state);
        assert_eq!(pending.len(), 2);
        assert!(resolve(&state, "a4", true).await);
        assert_eq!(list(&state).len(), 1);
        assert_eq!(list(&state)[0].id, "a3");
    }

    #[tokio::test]
    async fn abandoned_waiters_are_marked_in_db() {
        let state = Arc::new(McpState::for_tests());
        // waiter dropped immediately = client disconnected mid-request
        let (tx, rx) = oneshot::channel();
        drop(rx);
        register(&state, req("a5"), tx).await;
        let db = state.db.lock().await;
        let rows = db.list_mcp_approvals(10).unwrap();
        assert_eq!(rows[0].status, STATUS_ABANDONED);
    }
}
