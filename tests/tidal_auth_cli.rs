//! Native CLI checks with synthetic credentials only. No OAuth request is needed.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_drift-tidal-auth");
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const PERMISSION_BITS: u32 = 0o7777;
const START_TIMEOUT_SECONDS: u64 = 10;
const POLL_MILLISECONDS: u64 = 20;
const FIXTURE_ACCESS: &str = "synthetic-cli-access";
const FIXTURE_REFRESH: &str = "synthetic-cli-refresh";

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_export_omits_refresh_and_identity() {
    let credentials_root = tempfile::tempdir().unwrap();
    let socket_root = tempfile::tempdir().unwrap();
    for root in [credentials_root.path(), socket_root.path()] {
        fs::set_permissions(root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();
    }
    let path = credentials_root.path().join("credentials.json");
    let socket = socket_root.path().join("broker.sock");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "access_token": FIXTURE_ACCESS,
        "refresh_token": FIXTURE_REFRESH,
        "token_type": "Bearer",
        "user_id": 1,
        "expires_at": "2099-01-01T00:00:00Z"
    }))
    .unwrap();
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE)).unwrap();
    let mut server = Server(
        Command::new(BINARY)
            .arg("serve")
            .arg(&socket)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(START_TIMEOUT_SECONDS);
    while !fs::metadata(&socket)
        .is_ok_and(|metadata| metadata.permissions().mode() & PERMISSION_BITS == FILE_MODE)
    {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "fixture broker exited"
        );
        assert!(Instant::now() < deadline, "fixture broker startup deadline");
        std::thread::sleep(Duration::from_millis(POLL_MILLISECONDS));
    }
    let output = Command::new(BINARY)
        .arg("get")
        .arg(&socket)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let access: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(access["access_token"], FIXTURE_ACCESS);
    assert!(access.get("expires_at").is_some());
    assert!(access.get("refresh_token").is_none());
    assert!(access.get("user_id").is_none());
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "fresh authorization must not rotate"
    );
}

#[test]
fn malformed_invocation_has_only_fixed_error_output() {
    let output = Command::new(BINARY)
        .arg("invalid")
        .arg(FIXTURE_ACCESS)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"tidal_authorization_unavailable\n");
}
