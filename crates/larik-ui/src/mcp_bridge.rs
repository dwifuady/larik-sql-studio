use std::sync::Arc;
use std::time::Duration;

use larik_core::mcp::{
    McpApprovalDecision, McpApprovalRequest, McpOpenRequest, McpOpenResponse, McpPipeIncoming,
    mcp_approval_status, mcp_pipe,
};
use larik_core::store::StudioStore;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot};

pub enum McpBridgeCommand {
    Approve {
        request: McpApprovalRequest,
        reply: oneshot::Sender<McpApprovalDecision>,
    },
    Open {
        request: McpOpenRequest,
        reply: oneshot::Sender<McpOpenResponse>,
    },
}

pub fn start_mcp_pipe_server(
    runtime: &Arc<Runtime>,
    store: Arc<StudioStore>,
) -> mpsc::UnboundedReceiver<McpBridgeCommand> {
    let (tx, rx) = mpsc::unbounded_channel();
    let pipe_name = mcp_pipe::pipe_name();

    runtime.spawn(async move {
        let _ = store.sweep_mcp_approvals();
        run_pipe_loop(pipe_name, tx).await;
    });

    rx
}

#[cfg(windows)]
async fn run_pipe_loop(pipe_name: String, tx: mpsc::UnboundedSender<McpBridgeCommand>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut server = match ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
    {
        Ok(s) => s,
        Err(err) => {
            eprintln!("larik-sql-studio: could not create MCP pipe '{pipe_name}': {err}");
            return;
        }
    };

    loop {
        if server.connect().await.is_err() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }

        let connected = server;
        server = match ServerOptions::new().create(&pipe_name) {
            Ok(next) => next,
            Err(err) => {
                eprintln!("larik-sql-studio: could not recreate MCP pipe '{pipe_name}': {err}");
                break;
            }
        };

        let tx_conn = tx.clone();
        tokio::spawn(async move {
            let (read_half, mut write_half) = tokio::io::split(connected);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();

            let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await
            else {
                return;
            };
            if n == 0 {
                return;
            }

            let reply_json = match McpPipeIncoming::parse_line(line.trim()) {
                Ok(McpPipeIncoming::Approval(req)) => {
                    let req_id = req.id.clone();
                    let (reply_tx, reply_rx) = oneshot::channel();
                    if tx_conn
                        .send(McpBridgeCommand::Approve {
                            request: req,
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        serde_json::to_string(&McpApprovalDecision::denied(
                            req_id,
                            mcp_approval_status::ORPHANED,
                            "Larik SQL Studio is closing, so the statement was refused.",
                        ))
                        .ok()
                    } else {
                        let decision = match tokio::time::timeout(
                            mcp_pipe::APPROVAL_TIMEOUT + Duration::from_secs(10),
                            reply_rx,
                        )
                        .await
                        {
                            Ok(Ok(d)) => d,
                            Ok(Err(_)) => McpApprovalDecision::denied(
                                req_id,
                                mcp_approval_status::DENIED,
                                "The approval prompt closed without a decision.",
                            ),
                            Err(_) => McpApprovalDecision::denied(
                                req_id,
                                mcp_approval_status::TIMEOUT,
                                "The approval prompt timed out after two minutes, so the statement was refused.",
                            ),
                        };
                        serde_json::to_string(&decision).ok()
                    }
                }
                Ok(McpPipeIncoming::Open(req)) => {
                    let req_id = req.id.clone();
                    let (reply_tx, reply_rx) = oneshot::channel();
                    if tx_conn
                        .send(McpBridgeCommand::Open {
                            request: req,
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        serde_json::to_string(&McpOpenResponse::err(
                            req_id,
                            "Larik SQL Studio is closing.",
                        ))
                        .ok()
                    } else {
                        let response = match tokio::time::timeout(
                            mcp_pipe::OPEN_REPLY_TIMEOUT,
                            reply_rx,
                        )
                        .await
                        {
                            Ok(Ok(r)) => r,
                            _ => McpOpenResponse::err(
                                req_id,
                                "Larik SQL Studio did not respond in time.",
                            ),
                        };
                        serde_json::to_string(&response).ok()
                    }
                }
                Err(_) => None,
            };

            if let Some(mut out) = reply_json {
                out.push('\n');
                let _ = write_half.write_all(out.as_bytes()).await;
                let _ = write_half.flush().await;
            }
        });
    }
}

#[cfg(not(windows))]
async fn run_pipe_loop(_pipe_name: String, _tx: mpsc::UnboundedSender<McpBridgeCommand>) {
    std::future::pending::<()>().await;
}
