//! Filesystem adapter for current and pre-rename Tidal credentials.
use super::TidalConfig;
use anyhow::{Context, Result};
use std::io::{ErrorKind, Write};
use std::path::Path;

const APPLICATION_DIRECTORY: &str = "drift";
const LEGACY_DIRECTORY: &str = "tidal-tui";
const CREDENTIAL_FILE: &str = "credentials.json";

pub(super) fn load(root: &Path) -> Result<Option<TidalConfig>> {
    let candidates = [
        root.join(APPLICATION_DIRECTORY).join(CREDENTIAL_FILE),
        root.join(LEGACY_DIRECTORY).join(CREDENTIAL_FILE),
        root.join("upmpdcli/tidal/oauth2.credentials.json"),
    ];
    for path in candidates {
        match std::fs::read(&path) {
            Ok(bytes) => {
                // A malformed preferred account must not select a different account.
                return serde_json::from_slice(&bytes)
                    .context("Cannot parse the selected Tidal credential file")
                    .map(Some);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("Cannot read the selected Tidal credential file")
            }
        }
    }
    Ok(None)
}

pub(super) fn save(root: &Path, config: &TidalConfig) -> Result<()> {
    let directory = root.join(APPLICATION_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    // NamedTempFile uses mode 0600 on Unix. Rename avoids truncated token files.
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), config)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(directory.join(CREDENTIAL_FILE))?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(token: &str) -> TidalConfig {
        TidalConfig {
            access_token: token.into(),
            refresh_token: "fixture-refresh".into(),
            token_type: "Bearer".into(),
            user_id: 1,
            expires_at: None,
        }
    }
    fn put(root: &Path, directory: &str, token: &str) {
        let path = root.join(directory);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join(CREDENTIAL_FILE),
            serde_json::to_vec(&fixture(token)).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn loads_legacy_without_copying_or_changing_it() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), LEGACY_DIRECTORY, "legacy");
        assert_eq!(load(root.path()).unwrap().unwrap().access_token, "legacy");
        assert!(!root.path().join(APPLICATION_DIRECTORY).exists());
    }
    #[test]
    fn current_credentials_take_precedence() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), LEGACY_DIRECTORY, "legacy");
        save(root.path(), &fixture("current")).unwrap();
        assert_eq!(load(root.path()).unwrap().unwrap().access_token, "current");
    }
    #[test]
    fn malformed_preferred_file_does_not_fall_back() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), LEGACY_DIRECTORY, "legacy");
        save(root.path(), &fixture("current")).unwrap();
        std::fs::write(
            root.path()
                .join(APPLICATION_DIRECTORY)
                .join(CREDENTIAL_FILE),
            b"invalid",
        )
        .unwrap();
        assert!(load(root.path()).is_err());
    }
    #[test]
    fn missing_credentials_and_other_provider_are_not_accounts() {
        let root = tempfile::tempdir().unwrap();
        assert!(load(root.path()).unwrap().is_none());
        let foreign = root.path().join("upmpdcli/qobuz");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(
            foreign.join("oauth2.credentials.json"),
            serde_json::to_vec(&fixture("foreign")).unwrap(),
        )
        .unwrap();
        assert!(load(root.path()).unwrap().is_none());
    }
    #[test]
    fn blocked_destination_preserves_legacy_credentials() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), LEGACY_DIRECTORY, "legacy");
        std::fs::write(root.path().join(APPLICATION_DIRECTORY), b"not a directory").unwrap();
        assert!(save(root.path(), &fixture("replacement")).is_err());
        let original =
            std::fs::read(root.path().join(LEGACY_DIRECTORY).join(CREDENTIAL_FILE)).unwrap();
        assert_eq!(
            serde_json::from_slice::<TidalConfig>(&original)
                .unwrap()
                .access_token,
            "legacy"
        );
    }
    #[cfg(unix)]
    #[test]
    fn replacement_is_owner_only_even_when_old_file_was_public() {
        use std::os::unix::fs::PermissionsExt;
        const PUBLIC_MODE: u32 = 0o644;
        const PRIVATE_MODE: u32 = 0o600;
        const MODE_MASK: u32 = 0o777;
        let root = tempfile::tempdir().unwrap();
        save(root.path(), &fixture("old")).unwrap();
        let path = root
            .path()
            .join(APPLICATION_DIRECTORY)
            .join(CREDENTIAL_FILE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(PUBLIC_MODE)).unwrap();
        save(root.path(), &fixture("new")).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & MODE_MASK,
            PRIVATE_MODE
        );
        assert_eq!(load(root.path()).unwrap().unwrap().access_token, "new");
    }
}
