//! Explicit offline upgrade. No automatic replacement of user databases.
use std::path::PathBuf;

use anyhow::{ensure, Context, Result};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: drift-db-upgrade SOURCE NEW_DESTINATION (stop all database users first)";
    let source = PathBuf::from(args.next().context(usage)?);
    let destination = PathBuf::from(args.next().context(usage)?);
    ensure!(args.next().is_none(), usage);
    drift::database_upgrade::upgrade_copy(&source, &destination)?;
    println!("Upgrade verified. The source remains unchanged. The new file is not active.");
    Ok(())
}
