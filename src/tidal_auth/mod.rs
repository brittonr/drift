//! Single-writer Unix-socket Tidal authorization service.
//! Broker errors contain fixed categories, never upstream bodies or tokens.
pub mod core;
use anyhow::{anyhow, Result};
use chrono::Utc;
use core::{Access, Credentials, Request, MAX_FRAME_BYTES};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

pub const SOCKET_ENV: &str = "DRIFT_TIDAL_AUTH_SOCKET";
const IO_TIMEOUT_SECONDS: u64 = 10;
const HTTP_TIMEOUT_SECONDS: u64 = 7;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const PERMISSION_BITS: u32 = 0o7777;

fn current_uid() -> u32 {
    // SAFETY: geteuid has no arguments, pointer access, or failure case.
    unsafe { libc::geteuid() }
}

pub struct CredentialGuard {
    file: File,
    path: PathBuf,
}
impl CredentialGuard {
    pub fn for_config_root(root: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let directory = root.join("drift");
        match fs::DirBuilder::new()
            .mode(PRIVATE_DIRECTORY_MODE)
            .create(&directory)
        {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(anyhow!("credential_directory_unavailable")),
        }
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(&directory)?;
        if opened.metadata()?.uid() != current_uid() {
            return Err(anyhow!("credential_directory_not_owned"));
        }
        opened.set_permissions(fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
        Self::acquire(&directory)
    }
    pub fn acquire(directory: &Path) -> Result<Self> {
        private_directory(directory)?;
        let path = directory.join("tidal-auth.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| anyhow!("broker_lock_failed"))?;
        file.try_lock()
            .map_err(|_| anyhow!("broker_already_running"))?;
        let guard = Self { file, path };
        guard.check()?;
        Ok(guard)
    }
    /// Reject a replaced or unsafe lock before credential effects.
    pub fn check(&self) -> Result<()> {
        let opened = self.file.metadata()?;
        let named = fs::symlink_metadata(&self.path)?;
        if !opened.is_file()
            || opened.uid() != current_uid()
            || opened.mode() & PERMISSION_BITS != PRIVATE_FILE_MODE
            || opened.nlink() != 1
            || opened.ino() != named.ino()
            || opened.dev() != named.dev()
        {
            return Err(anyhow!("credential_lock_changed"));
        }
        Ok(())
    }
}
const FINGERPRINT_HEX_BYTES: usize = 128;
const TOKEN_URL: &str = "https://auth.tidal.com/v1/oauth2/token";
const LEGACY_CLIENT_ID: &str = "dN2N95wCyEBTllu4";

pub fn configured_socket() -> Result<Option<PathBuf>> {
    match std::env::var_os(SOCKET_ENV) {
        None => Ok(None),
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(anyhow!("invalid_broker_socket"));
            }
            Ok(Some(path))
        }
    }
}

fn private_directory(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| anyhow!("broker_directory_unavailable"))?;
    if !metadata.is_dir()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & PERMISSION_BITS != PRIVATE_DIRECTORY_MODE
    {
        return Err(anyhow!("broker_directory_not_private"));
    }
    Ok(())
}

fn read_credentials(path: &Path) -> Result<Credentials> {
    let metadata = fs::symlink_metadata(path).map_err(|_| anyhow!("credentials_unavailable"))?;
    if !metadata.is_file()
        || metadata.uid() != current_uid()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & PERMISSION_BITS != PRIVATE_FILE_MODE
    {
        return Err(anyhow!("credentials_not_private"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid_credentials_path"))?;
    if metadata.uid()
        != fs::metadata(parent)
            .map_err(|_| anyhow!("credentials_unavailable"))?
            .uid()
    {
        return Err(anyhow!("credentials_owner_mismatch"));
    }
    let mut bytes = Vec::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| anyhow!("credentials_unavailable"))?
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("credentials_unavailable"))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(anyhow!("credentials_oversized"));
    }
    let value: Credentials =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid_credentials"))?;
    value.validate().map_err(|category| anyhow!(category))?;
    Ok(value)
}

fn save_credentials(path: &Path, value: &Credentials) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid_credentials_path"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| anyhow!("credential_write_failed"))?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))
        .map_err(|_| anyhow!("credential_write_failed"))?;
    serde_json::to_writer(&mut file, value).map_err(|_| anyhow!("credential_write_failed"))?;
    file.as_file()
        .sync_all()
        .map_err(|_| anyhow!("credential_write_failed"))?;
    file.persist(path)
        .map_err(|_| anyhow!("credential_write_failed"))?;
    File::open(parent)?
        .sync_all()
        .map_err(|_| anyhow!("credential_write_failed"))?;
    Ok(())
}

fn fingerprint(value: &Credentials) -> String {
    blake3::hash(value.refresh_token.as_bytes())
        .to_hex()
        .to_string()
        + blake3::hash(value.access_token.as_bytes())
            .to_hex()
            .as_ref()
}

#[async_trait::async_trait]
trait TokenRefresher {
    async fn renew(&self, refresh_token: &str, client_id: &str) -> Result<Vec<u8>>;
}
struct HttpRefresher {
    client: reqwest::Client,
}
#[async_trait::async_trait]
impl TokenRefresher for HttpRefresher {
    async fn renew(&self, refresh_token: &str, client_id: &str) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
            ])
            .send()
            .await
            .map_err(|_| anyhow!("renewal_transport_failed"))?;
        if !response.status().is_success() {
            return Err(anyhow!("renewal_rejected"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("renewal_transport_failed"))?
        {
            if chunk.len() > MAX_FRAME_BYTES.saturating_sub(body.len()) {
                return Err(anyhow!("oversized_reply"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

struct Broker<R = HttpRefresher> {
    credentials: Credentials,
    path: PathBuf,
    pending: PathBuf,
    lock: CredentialGuard,
    refresher: R,
}
impl Broker<HttpRefresher> {
    pub fn open(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("invalid_credentials_path"))?;
        private_directory(parent)?;
        let lock = CredentialGuard::acquire(parent)?;
        let credentials = read_credentials(path)?;
        let pending = parent.join("tidal-auth.pending");
        if pending.exists() {
            let metadata = pending.symlink_metadata()?;
            if !metadata.is_file()
                || metadata.uid() != current_uid()
                || metadata.mode() & PERMISSION_BITS != PRIVATE_FILE_MODE
                || metadata.nlink() != 1
                || metadata.len() != FINGERPRINT_HEX_BYTES as u64
            {
                return Err(anyhow!("invalid_renewal_intent"));
            }
            let prior =
                fs::read_to_string(&pending).map_err(|_| anyhow!("invalid_renewal_intent"))?;
            if !prior
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(anyhow!("invalid_renewal_intent"));
            }
            if prior == fingerprint(&credentials) {
                return Err(anyhow!("renewal_outcome_uncertain"));
            }
            // A different durable credential proves the replacement completed.
            fs::remove_file(&pending)?;
            File::open(parent)?.sync_all()?;
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
            .build()
            .map_err(|_| anyhow!("http_configuration_failed"))?;
        Ok(Self {
            credentials,
            path: path.to_owned(),
            pending,
            lock,
            refresher: HttpRefresher { client: http },
        })
    }
}
impl<R: TokenRefresher> Broker<R> {
    pub async fn handle(&mut self, request: Request) -> Result<Access> {
        self.lock.check()?;
        if self.pending.exists() {
            return Err(anyhow!("renewal_outcome_uncertain"));
        }
        if self
            .credentials
            .needs_refresh(&request, Utc::now())
            .map_err(|category| anyhow!(category))?
        {
            let mut intent = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(PRIVATE_FILE_MODE)
                .open(&self.pending)
                .map_err(|_| anyhow!("renewal_intent_failed"))?;
            intent.write_all(fingerprint(&self.credentials).as_bytes())?;
            intent.sync_all()?;
            File::open(self.pending.parent().unwrap())?.sync_all()?;
            let body = self
                .refresher
                .renew(
                    &self.credentials.refresh_token,
                    self.credentials
                        .client_id
                        .as_deref()
                        .unwrap_or(LEGACY_CLIENT_ID),
                )
                .await?;
            let next = core::renewed(&self.credentials, &body, Utc::now())
                .map_err(|category| anyhow!(category))?;
            self.lock.check()?;
            save_credentials(&self.path, &next)?;
            self.credentials = next;
            fs::remove_file(&self.pending)?;
            File::open(self.pending.parent().unwrap())?.sync_all()?;
        }
        Ok(self.credentials.access())
    }
}

pub fn blocking_request(socket: &Path, request: &Request) -> Result<Access> {
    let socket = socket.to_owned();
    let bytes = request_bytes(request)?;
    // A dedicated thread permits callers with or without an existing Tokio runtime.
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| anyhow!("broker_client_failed"))?
            .block_on(exchange(
                &socket,
                bytes,
                Duration::from_secs(IO_TIMEOUT_SECONDS),
            ))
    })
    .join()
    .map_err(|_| anyhow!("broker_client_failed"))?
}

fn request_bytes(request: &Request) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(request).map_err(|_| anyhow!("invalid_request"))?;
    if bytes.len() >= MAX_FRAME_BYTES {
        return Err(anyhow!("oversized_request"));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

async fn exchange(socket: &Path, bytes: Vec<u8>, timeout: Duration) -> Result<Access> {
    private_directory(
        socket
            .parent()
            .ok_or_else(|| anyhow!("invalid_socket_path"))?,
    )?;
    let metadata = fs::symlink_metadata(socket).map_err(|_| anyhow!("broker_unavailable"))?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != current_uid()
        || metadata.mode() & PERMISSION_BITS != PRIVATE_FILE_MODE
    {
        return Err(anyhow!("unsafe_socket_path"));
    }
    tokio::time::timeout(timeout, async {
        let mut connection = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|_| anyhow!("broker_unavailable"))?;
        if connection
            .peer_cred()
            .map_err(|_| anyhow!("broker_unavailable"))?
            .uid()
            != current_uid()
        {
            return Err(anyhow!("unsafe_socket_peer"));
        }
        connection
            .write_all(&bytes)
            .await
            .map_err(|_| anyhow!("broker_unavailable"))?;
        let mut body = Vec::new();
        tokio::io::BufReader::new(connection)
            .take((MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut body)
            .await
            .map_err(|_| anyhow!("broker_unavailable"))?;
        if body.len() > MAX_FRAME_BYTES {
            return Err(anyhow!("oversized_reply"));
        }
        if body.last() != Some(&b'\n') {
            return Err(anyhow!("incomplete_reply"));
        }
        let access: Access =
            serde_json::from_slice(&body).map_err(|_| anyhow!("broker_rejected_request"))?;
        access
            .validate(Utc::now())
            .map_err(|category| anyhow!(category))?;
        Ok(access)
    })
    .await
    .map_err(|_| anyhow!("broker_deadline_exceeded"))?
}

pub async fn request(socket: PathBuf, request: Request) -> Result<Access> {
    exchange(
        &socket,
        request_bytes(&request)?,
        Duration::from_secs(IO_TIMEOUT_SECONDS),
    )
    .await
}

pub async fn serve(path: &Path, socket: &Path) -> Result<()> {
    private_directory(
        socket
            .parent()
            .ok_or_else(|| anyhow!("invalid_socket_path"))?,
    )?;
    let mut broker = Broker::open(path)?;
    let socket_guard = CredentialGuard::acquire(
        socket
            .parent()
            .ok_or_else(|| anyhow!("invalid_socket_path"))?,
    )?;
    if let Ok(metadata) = socket.symlink_metadata() {
        if !metadata.file_type().is_socket() || metadata.uid() != current_uid() {
            return Err(anyhow!("unsafe_socket_path"));
        }
        // The retained socket lock excludes a running broker at this endpoint.
        fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket).map_err(|_| anyhow!("broker_socket_unavailable"))?;
    fs::set_permissions(socket, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    loop {
        let (mut connection, _) = listener.accept().await?;
        socket_guard.check()?;
        if connection.peer_cred()?.uid() != current_uid() {
            continue;
        }
        let mut body = Vec::new();
        let read = tokio::time::timeout(
            Duration::from_secs(IO_TIMEOUT_SECONDS),
            tokio::io::BufReader::new(&mut connection)
                .take((MAX_FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut body),
        )
        .await;
        let output = if matches!(read, Ok(Ok(_))) && body.len() <= MAX_FRAME_BYTES {
            if let Ok(request) = serde_json::from_slice::<Request>(&body) {
                broker
                    .handle(request)
                    .await
                    .ok()
                    .and_then(|value| serde_json::to_vec(&value).ok())
            } else {
                None
            }
        } else {
            None
        };
        let mut output =
            output.unwrap_or_else(|| b"{\"error\":\"authorization_unavailable\"}".to_vec());
        output.push(b'\n');
        let _ = tokio::time::timeout(
            Duration::from_secs(IO_TIMEOUT_SECONDS),
            connection.write_all(&output),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> PathBuf {
        fs::set_permissions(root, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE)).unwrap();
        let path = root.join("credentials.json");
        let value = Credentials {
            access_token: "access-fixture".into(),
            refresh_token: "refresh-fixture".into(),
            token_type: "Bearer".into(),
            user_id: 1,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(core::MAX_LIFETIME_SECONDS)),
            client_id: None,
        };
        save_credentials(&path, &value).unwrap();
        path
    }
    #[tokio::test]
    async fn one_writer_and_access_only_reply() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let mut broker = Broker::open(&path).unwrap();
        assert!(Broker::open(&path).is_err());
        let reply = broker.handle(Request::Get).await.unwrap();
        assert!(!serde_json::to_string(&reply).unwrap().contains("refresh"));
        assert_eq!(reply.access_token, "access-fixture");
        drop(broker);
        assert!(Broker::open(&path).is_ok());
    }
    struct FakeRefresher {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl TokenRefresher for FakeRefresher {
        async fn renew(&self, _: &str, _: &str) -> Result<Vec<u8>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                return Err(anyhow!("fixture_transport_failure"));
            }
            Ok(br#"{"access_token":"rotated-access","refresh_token":"rotated-refresh","token_type":"Bearer","expires_in":3600}"#.to_vec())
        }
    }
    fn fake_broker(
        path: &Path,
        fail: bool,
    ) -> (
        Broker<FakeRefresher>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let broker = Broker::open(path).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Broker {
                credentials: broker.credentials,
                path: broker.path,
                pending: broker.pending,
                lock: broker.lock,
                refresher: FakeRefresher {
                    calls: calls.clone(),
                    fail,
                },
            },
            calls,
        )
    }
    #[tokio::test]
    async fn queued_rejections_rotate_once_and_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let (mut broker, calls) = fake_broker(&path, false);
        for _ in ["first-client", "second-client"] {
            let access = broker
                .handle(Request::Refresh {
                    rejected_access_token: "access-fixture".into(),
                })
                .await
                .unwrap();
            assert_eq!(access.access_token, "rotated-access");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!broker.pending.exists());
        drop(broker);
        let mut restarted = Broker::open(&path).unwrap();
        assert_eq!(
            restarted.handle(Request::Get).await.unwrap().access_token,
            "rotated-access"
        );
    }
    #[tokio::test]
    async fn failed_refresh_blocks_retries_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let (mut broker, calls) = fake_broker(&path, true);
        assert!(broker
            .handle(Request::Refresh {
                rejected_access_token: "access-fixture".into()
            })
            .await
            .is_err());
        assert!(broker.handle(Request::Get).await.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(broker);
        assert!(Broker::open(&path).is_err());
        assert_eq!(
            read_credentials(&path).unwrap().access_token,
            "access-fixture"
        );
    }
    #[tokio::test]
    async fn native_socket_rejects_bad_frames_and_returns_access_only() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let socket_root = root.path().join("socket");
        fs::create_dir(&socket_root).unwrap();
        fs::set_permissions(
            &socket_root,
            fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
        )
        .unwrap();
        let socket = socket_root.join("auth.sock");
        let server_path = path.clone();
        let server_socket = socket.clone();
        let server = tokio::spawn(async move { serve(&server_path, &server_socket).await });
        const WAIT_MILLISECONDS: u64 = 10;
        const MAX_WAITS: usize = 100;
        for _ in 0..MAX_WAITS {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(WAIT_MILLISECONDS)).await;
        }
        let result = request(socket.clone(), Request::Get).await.unwrap();
        assert_eq!(result.access_token, "access-fixture");
        let tui = crate::service::tidal::TidalClient::new_for_socket(Some(socket.clone()))
            .await
            .unwrap();
        assert!(tui.config.as_ref().unwrap().refresh_token.is_empty());
        assert!(tui.save_config().await.is_err());
        let sync_socket = socket.clone();
        let sync_user = tokio::task::spawn_blocking(move || {
            crate::sync::api::SyncApiClient::load_for_socket(Some(sync_socket))
                .map(|client| client.user_id())
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(sync_user, 1);
        assert!(CredentialGuard::acquire(root.path()).is_err());
        let mut connection = tokio::net::UnixStream::connect(&socket).await.unwrap();
        connection
            .write_all(b"{\"operation\":\"erase\"}\n")
            .await
            .unwrap();
        let mut reply = Vec::new();
        connection.read_to_end(&mut reply).await.unwrap();
        assert!(String::from_utf8(reply)
            .unwrap()
            .contains("authorization_unavailable"));
        server.abort();
        let _ = server.await;
        assert!(
            crate::service::tidal::TidalClient::new_for_socket(Some(socket.clone()))
                .await
                .is_err()
        );
        let sync_socket = socket.clone();
        assert!(tokio::task::spawn_blocking(move || {
            crate::sync::api::SyncApiClient::load_for_socket(Some(sync_socket))
        })
        .await
        .unwrap()
        .is_err());
        assert!(Broker::open(&path).is_ok());
    }
    #[tokio::test]
    async fn replaced_lock_rejects_before_remote_refresh() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let (mut broker, calls) = fake_broker(&path, false);
        assert!(broker.lock.check().is_ok());
        fs::remove_file(&broker.lock.path).unwrap();
        let replacement = CredentialGuard::acquire(root.path()).unwrap();
        assert!(replacement.check().is_ok());
        assert!(broker.lock.check().is_err());
        assert!(broker
            .handle(Request::Refresh {
                rejected_access_token: "access-fixture".into()
            })
            .await
            .is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn drip_feed_cannot_extend_the_client_deadline() {
        const DEADLINE_MILLISECONDS: u64 = 100;
        const DRIP_MILLISECONDS: u64 = 10;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(
            root.path(),
            fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
        )
        .unwrap();
        let socket = root.path().join("drip.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(PRIVATE_FILE_MODE)).unwrap();
        let server = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            loop {
                if connection.write_all(b" ").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(DRIP_MILLISECONDS)).await;
            }
        });
        let error = exchange(
            &socket,
            request_bytes(&Request::Get).unwrap(),
            Duration::from_millis(DEADLINE_MILLISECONDS),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "broker_deadline_exceeded");
        server.await.unwrap();
    }

    #[test]
    fn uncertain_refresh_and_unsafe_files_reject() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path());
        let value = read_credentials(&path).unwrap();
        let pending = root.path().join("tidal-auth.pending");
        fs::write(&pending, fingerprint(&value)).unwrap();
        fs::set_permissions(&pending, fs::Permissions::from_mode(PRIVATE_FILE_MODE)).unwrap();
        assert_eq!(
            Broker::open(&path).err().unwrap().to_string(),
            "renewal_outcome_uncertain"
        );
        // A persisted replacement followed by a crash before intent removal is recoverable.
        let mut replaced = value.clone();
        replaced.access_token = "durable-new-access".into();
        save_credentials(&path, &replaced).unwrap();
        assert!(Broker::open(&path).is_ok());
        assert!(!pending.exists());
        const PUBLIC_FILE_MODE: u32 = 0o644;
        fs::set_permissions(&path, fs::Permissions::from_mode(PUBLIC_FILE_MODE)).unwrap();
        assert!(Broker::open(&path).is_err());
        assert!(blocking_request(&root.path().join("absent"), &Request::Get).is_err());
    }
}
