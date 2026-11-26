use std::path::PathBuf;
use std::sync::Arc;

use larik_core::mcp::mcp_pipe;
use larik_core::store::StudioStore;
use larik_mcp::build_default_catalog;
use larik_mcp::context::StudioContext;
use larik_mcp::server;

fn parse_cli_args() -> (Option<PathBuf>, Option<String>) {
    let mut args = std::env::args().skip(1);
    let mut state_dir: Option<PathBuf> = None;
    let mut pipe_override: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state" => {
                if let Some(val) = args.next() {
                    if !val.trim().is_empty() {
                        state_dir = Some(PathBuf::from(val));
                    }
                }
            }
            "--pipe" => {
                if let Some(val) = args.next() {
                    if !val.trim().is_empty() {
                        pipe_override = Some(val);
                    }
                }
            }
            _ => {}
        }
    }

    (state_dir, pipe_override)
}

#[tokio::main]
async fn main() {
    let (state_dir, pipe_override) = parse_cli_args();

    let store = match state_dir {
        Some(dir) => {
            if let Err(err) = std::fs::create_dir_all(&dir) {
                eprintln!("larik-mcp: failed to prepare state directory {}: {err}", dir.display());
                std::process::exit(1);
            }
            StudioStore::open_at(dir.join("studio.db"))
        }
        None => StudioStore::open_default(),
    };

    let store = match store {
        Ok(s) => Arc::new(s),
        Err(err) => {
            eprintln!("larik-mcp: failed to open studio database: {err}");
            std::process::exit(1);
        }
    };

    let pipe_name = pipe_override.unwrap_or_else(mcp_pipe::pipe_name);
    let catalog = build_default_catalog();
    let ctx = Arc::new(StudioContext::new(store, catalog, pipe_name));

    if let Err(err) = server::run_stdio(ctx).await {
        eprintln!("larik-mcp: stdio transport error: {err}");
        std::process::exit(1);
    }
}
