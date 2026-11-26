//! Named-pipe client talking to the running `larik-sql-studio` desktop studio for interactive
//! write approvals and `open_in_studio`.

use std::time::Duration;

use larik_core::mcp::{
    McpApprovalDecision, McpApprovalRequest, McpOpenRequest, McpOpenResponse, mcp_pipe,
};

#[cfg(windows)]
async fn round_trip_line(
    pipe_name: &str,
    request_json: &str,
    read_timeout: Duration,
) -> Option<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ClientOptions;

    const ERROR_PIPE_BUSY: i32 = 231;

    let deadline = tokio::time::Instant::now() + mcp_pipe::CONNECT_TIMEOUT;
    let client = loop {
        match ClientOptions::new().open(pipe_name) {
            Ok(client) => break client,
            Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                if tokio::time::Instant::now() >= deadline {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(35)).await;
            }
            Err(_) => {
                // Pipe does not exist (studio closed) or permission error.
                return None;
            }
        }
    };

    let (read_half, mut write_half) = tokio::io::split(client);
    let mut line_out = String::with_capacity(request_json.len() + 1);
    line_out.push_str(request_json);
    line_out.push('\n');

    if tokio::time::timeout(
        Duration::from_secs(5),
        write_half.write_all(line_out.as_bytes()),
    )
    .await
    .map_or(true, |res| res.is_err())
    {
        return None;
    }
    let _ = write_half.flush().await;

    let mut reader = BufReader::new(read_half);
    let mut reply = String::new();
    match tokio::time::timeout(read_timeout, reader.read_line(&mut reply)).await {
        Ok(Ok(n)) if n > 0 => Some(reply),
        _ => None,
    }
}

#[cfg(not(windows))]
async fn round_trip_line(
    _pipe_name: &str,
    _request_json: &str,
    _read_timeout: Duration,
) -> Option<String> {
    None
}

/// Sends a write-approval request to the running studio. Returns `None` if the studio is closed
/// or the pipe disconnects before a decision arrives.
pub async fn request_approval(
    pipe_name: &str,
    request: &McpApprovalRequest,
) -> Option<McpApprovalDecision> {
    let payload = serde_json::to_string(request).ok()?;
    let reply = round_trip_line(pipe_name, &payload, mcp_pipe::CLIENT_APPROVAL_TIMEOUT).await?;
    serde_json::from_str::<McpApprovalDecision>(reply.trim()).ok()
}

/// Sends an `open_in_studio` request to the running studio. Returns `None` if the studio is closed.
pub async fn request_open_in_studio(
    pipe_name: &str,
    request: &McpOpenRequest,
) -> Option<McpOpenResponse> {
    let payload = serde_json::to_string(request).ok()?;
    let reply = round_trip_line(pipe_name, &payload, mcp_pipe::OPEN_REPLY_TIMEOUT).await?;
    serde_json::from_str::<McpOpenResponse>(reply.trim()).ok()
}
