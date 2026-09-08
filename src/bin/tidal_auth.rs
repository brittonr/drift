//! Private broker operator entry point. Standard output from `get` is secret.
use anyhow::{anyhow, Result};
use drift::tidal_auth::{blocking_request, core::Request, serve};
use std::io::{IsTerminal, Write};
use std::path::Path;

const GET_ARGUMENTS: usize = 3;
const SERVE_ARGUMENTS: usize = 4;
const ACTION_INDEX: usize = 1;
const SOCKET_INDEX: usize = 2;
const CREDENTIALS_INDEX: usize = 3;

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("tidal_authorization_unavailable");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(ACTION_INDEX).map(String::as_str) {
        Some("get") if args.len() == GET_ARGUMENTS => {
            if std::io::stdout().is_terminal() {
                return Err(anyhow!("terminal_export_refused"));
            }
            let access = blocking_request(Path::new(&args[SOCKET_INDEX]), &Request::Get)?;
            let mut output = std::io::stdout().lock();
            serde_json::to_writer(&mut output, &access.export())?;
            output.write_all(b"\n")?;
            Ok(())
        }
        Some("serve") if args.len() == SERVE_ARGUMENTS => {
            serve(
                Path::new(&args[CREDENTIALS_INDEX]),
                Path::new(&args[SOCKET_INDEX]),
            )
            .await
        }
        _ => Err(anyhow!("invalid_arguments")),
    }
}
