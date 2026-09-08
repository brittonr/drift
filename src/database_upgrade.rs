//! Offline database migration. Never changes or replaces the source file.
use std::fs::File;
use std::path::Path;

use anyhow::{ensure, Context, Result};

/// Copy a stopped database into a new, private, validated v3-format file.
/// The original file remains the rollback copy. Existing destinations are rejected.
pub fn upgrade_copy(source: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.try_exists()?, "destination already exists");
    let mut input = File::open(source).context("cannot open source database")?;
    ensure!(input.metadata()?.is_file(), "source must be a regular file");
    input
        .try_lock()
        .context("source is in use; stop all database users")?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output =
        tempfile::NamedTempFile::new_in(parent).context("cannot create private migration file")?;
    let copied = std::io::copy(&mut input, output.as_file_mut())?;
    ensure!(copied > 0, "source database is empty");
    output.as_file().sync_all()?;
    const LEGACY_FILE_FORMAT: u8 = 2;
    match redb::Database::open(output.path()) {
        Ok(current) => drop(current),
        Err(redb::DatabaseError::UpgradeRequired(LEGACY_FILE_FORMAT)) => {
            let mut legacy = redb_legacy::Database::open(output.path())
                .context("source is not a supported legacy database")?;
            legacy.upgrade().context("database format upgrade failed")?;
        }
        Err(error) => return Err(error).context("source is not a supported database"),
    }
    {
        let mut current =
            redb::Database::open(output.path()).context("upgraded database cannot be opened")?;
        ensure!(
            current.check_integrity()?,
            "upgraded database failed integrity check"
        );
    }
    output.as_file().sync_all()?;
    output
        .persist_noclobber(destination)
        .context("cannot publish upgraded database; destination must not exist")?;
    #[cfg(unix)]
    File::open(parent)?
        .sync_all()
        .context("database published, but directory sync failed")?;
    // Keep the source lock until the new file is durable.
    drop(input);
    Ok(())
}
