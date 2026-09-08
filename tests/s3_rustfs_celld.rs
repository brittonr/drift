//! Isolated live qualification. Never connects to an existing storage service.
//! Run with --ignored and explicit fixture binary paths from the runbook.
#![cfg(feature = "s3")]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use drift::storage::s3::{bind_blob, S3Storage};
use drift::storage::settings::S3Config;
use drift::storage::wal::{ReplicationOp, WalEntry};
use object_store::aws::{AmazonS3Builder, S3ConditionalPut};
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions, UpdateVersion};

const ACCESS_KEY: &str = "drift-fixture-key";
const SECRET_KEY: &str = "drift-fixture-secret-not-for-deployment";
const TOKEN: &str = "drift-fixture-token-not-for-deployment";
const REGION: &str = "us-east-1";
const USER: &str = "fixture-user";
const BLOB_BUCKET: &str = "drift-fixture-blobs";
const FLEET_BUCKET: &str = "drift-fixture-celld";
const READY_ATTEMPTS: usize = 120;
const READY_DELAY: Duration = Duration::from_millis(250);
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const LEASE_SETTLE: Duration = Duration::from_secs(12);
const ENTRY_TIME: u64 = 100;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn binary(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(
        std::env::var(name).with_context(|| format!("set {name} to the fixture binary"))?,
    );
    ensure!(
        path.is_absolute() && path.is_file(),
        "fixture binary path is invalid"
    );
    Ok(path)
}

fn address() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}

fn spawn(mut command: Command, directory: &Path, log_name: &str) -> Result<Process> {
    let log = std::fs::File::create(directory.join(log_name))?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    Ok(Process(command.spawn()?))
}

async fn ready(url: &str, process: &mut Process) -> Result<()> {
    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
    for _ in 0..READY_ATTEMPTS {
        ensure!(
            process.0.try_wait()?.is_none(),
            "fixture process exited before readiness"
        );
        if let Ok(response) = client.get(url).send().await {
            if response.status().is_success()
                || response.status() == reqwest::StatusCode::UNAUTHORIZED
            {
                return Ok(());
            }
        }
        tokio::time::sleep(READY_DELAY).await;
    }
    anyhow::bail!("fixture readiness deadline exceeded")
}

fn celld_command(binary: &Path, root: &Path, endpoint: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .current_dir(root)
        .env("TMPDIR", root)
        .env("AWS_ACCESS_KEY_ID", ACCESS_KEY)
        .env("AWS_SECRET_ACCESS_KEY", SECRET_KEY)
        .env("AWS_REGION", REGION)
        .env("CELLD_DURABILITY", "bucket")
        .env("CELLD_OUTPUT_GATE", "1")
        .env("CELLD_STORAGE_PROBE", "1")
        .env("CELLD_VAR_DRIFT_USER", USER)
        .env("CELLD_VAR_DRIFT_TOKEN", TOKEN)
        .args([
            "--bucket",
            &format!("s3://{FLEET_BUCKET}"),
            "--endpoint",
            endpoint,
            "--region",
            REGION,
        ]);
    command
}

fn deletion(id: &str) -> WalEntry {
    WalEntry {
        op: ReplicationOp::DeletePlaylist {
            playlist_id: id.into(),
        },
        created_at_ms: ENTRY_TIME,
        attempts: 0,
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated RustFS, Celld, mc, and esbuild fixture binaries"]
async fn rustfs_and_celld_durability_and_denial() -> Result<()> {
    let rustfs = binary("DRIFT_TEST_RUSTFS_BIN")?;
    let celld = binary("DRIFT_TEST_CELLD_BIN")?;
    let mc = binary("DRIFT_TEST_MC_BIN")?;
    let esbuild = binary("DRIFT_TEST_ESBUILD_BIN")?;
    let root = tempfile::tempdir()?;
    let result = async {
        let volume = root.path().join("volume");
        std::fs::create_dir(&volume)?;
        let s3_address = address()?;
        let s3_endpoint = format!("http://{s3_address}");
        let mut command = Command::new(rustfs);
        command
            .env_clear()
            .current_dir(root.path())
            .env("RUSTFS_ACCESS_KEY", ACCESS_KEY)
            .env("RUSTFS_SECRET_KEY", SECRET_KEY)
            .env("RUSTFS_REGION", REGION)
            .args(["server", "--address", &s3_address])
            .arg(&volume);
        let mut rustfs = spawn(command, root.path(), "rustfs.log")?;
        ready(&format!("{s3_endpoint}/health"), &mut rustfs).await?;
        let output = Command::new(mc)
            .env_clear()
            .env("HOME", root.path())
            .env("MC_CONFIG_DIR", root.path().join("mc"))
            .env(
                "MC_HOST_fixture",
                format!("http://{ACCESS_KEY}:{SECRET_KEY}@{s3_address}"),
            )
            .args([
                "mb",
                &format!("fixture/{BLOB_BUCKET}"),
                &format!("fixture/{FLEET_BUCKET}"),
            ])
            .output()?;
        std::fs::write(root.path().join("bucket-create.log"), &output.stderr)?;
        ensure!(output.status.success(), "fixture bucket creation failed");

        // Verify RustFS applies conditions, rather than merely accepting the headers.
        let store = AmazonS3Builder::new()
            .with_endpoint(&s3_endpoint)
            .with_region(REGION)
            .with_bucket_name(BLOB_BUCKET)
            .with_access_key_id(ACCESS_KEY)
            .with_secret_access_key(SECRET_KEY)
            .with_allow_http(true)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .build()?;
        let key = object_store::path::Path::from("conditional-probe");
        let create = PutOptions {
            mode: PutMode::Create,
            ..PutOptions::default()
        };
        let first = store
            .put_opts(&key, b"first".to_vec().into(), create.clone())
            .await?;
        assert!(matches!(
            store.put_opts(&key, b"wrong".to_vec().into(), create).await,
            Err(object_store::Error::AlreadyExists { .. }
                | object_store::Error::Precondition { .. })
        ));
        let update = PutOptions {
            mode: PutMode::Update(UpdateVersion {
                e_tag: first.e_tag,
                version: None,
            }),
            ..PutOptions::default()
        };
        store
            .put_opts(&key, b"second".to_vec().into(), update.clone())
            .await?;
        assert!(matches!(
            store.put_opts(&key, b"stale".to_vec().into(), update).await,
            Err(object_store::Error::AlreadyExists { .. }
                | object_store::Error::Precondition { .. })
        ));
        assert_eq!(store.get(&key).await?.bytes().await?.as_ref(), b"second");
        store.delete(&key).await?;

        // This integration binary has one test. No production credential names are changed.
        std::env::set_var("DRIFT_FIXTURE_ACCESS", ACCESS_KEY);
        std::env::set_var("DRIFT_FIXTURE_SECRET", SECRET_KEY);
        std::env::set_var("DRIFT_FIXTURE_TOKEN", TOKEN);
        std::env::set_var("DRIFT_FIXTURE_WRONG_TOKEN", "wrong-fixture-token");
        let mut config = S3Config {
            endpoint: s3_endpoint.clone(),
            bucket: BLOB_BUCKET.into(),
            region: REGION.into(),
            access_key_env: "DRIFT_FIXTURE_ACCESS".into(),
            secret_key_env: "DRIFT_FIXTURE_SECRET".into(),
            celld_token_env: "DRIFT_FIXTURE_TOKEN".into(),
            allow_http: true,
            ..S3Config::default()
        };
        let remote = S3Storage::new(&config, USER)?;
        let first = deletion("first");
        let second = deletion("second");
        let (left, right) = tokio::join!(
            remote.replicate("phone", 0, &first),
            remote.replicate("laptop", 1, &second)
        );
        left?;
        right?;
        let snapshot = remote.snapshot().await?;
        assert!(snapshot.documents.contains_key("playlists/first"));
        assert!(snapshot.documents.contains_key("playlists/second"));

        let project = root.path().join("worker");
        std::fs::create_dir(&project)?;
        std::fs::copy("celld/worker.mjs", project.join("worker.mjs"))?;
        std::fs::copy("celld/wrangler.jsonc", project.join("wrangler.jsonc"))?;
        let mut deploy = Command::new(&celld);
        let output = deploy
            .env_clear()
            .current_dir(root.path())
            .env("TMPDIR", root.path())
            .env("CELLD_ESBUILD", esbuild)
            .env("AWS_ACCESS_KEY_ID", ACCESS_KEY)
            .env("AWS_SECRET_ACCESS_KEY", SECRET_KEY)
            .arg("deploy")
            .arg(&project)
            .args([
                "--bucket",
                &format!("s3://{FLEET_BUCKET}"),
                "--endpoint",
                &s3_endpoint,
                "--region",
                REGION,
            ])
            .output()?;
        std::fs::write(root.path().join("deploy.log"), &output.stderr)?;
        ensure!(output.status.success(), "Celld fixture deploy failed");
        let public = address()?;
        let internal = address()?;
        let mut command = celld_command(&celld, root.path(), &s3_endpoint);
        command
            .env("CELLD_WATCH", root.path().join("node-a"))
            .args([
                "--listen",
                &public,
                "--internal-listen",
                &internal,
                "--advertise",
                &internal,
            ]);
        let mut node = spawn(command, root.path(), "celld-a.log")?;
        let celld_endpoint = format!("http://{public}");
        ready(&format!("{celld_endpoint}/state"), &mut node).await?;
        config.celld_endpoint = Some(celld_endpoint.clone());
        let remote = S3Storage::new(&config, USER)?;
        let path = root.path().join("audio.flac");
        tokio::fs::write(&path, b"fixture audio bytes").await?;
        let mut upload = WalEntry {
            op: ReplicationOp::UploadBlob {
                track_id: "song".into(),
                file_path: path.to_str().context("invalid fixture path")?.into(),
                expected_hash: None,
            },
            ..deletion("unused")
        };
        bind_blob(&mut upload).await?;
        remote.replicate("phone", 1, &upload).await?;
        assert_eq!(
            remote
                .fetch_blob("song")
                .await?
                .context("missing uploaded blob")?,
            b"fixture audio bytes"
        );
        let mut denied = config.clone();
        denied.celld_token_env = "DRIFT_FIXTURE_WRONG_TOKEN".into();
        assert!(S3Storage::new(&denied, USER)?.snapshot().await.is_err());

        drop(node);
        tokio::time::sleep(LEASE_SETTLE).await;
        let mut command = celld_command(&celld, root.path(), &s3_endpoint);
        command
            .env("CELLD_WATCH", root.path().join("fresh-node-b"))
            .args([
                "--listen",
                &public,
                "--internal-listen",
                &internal,
                "--advertise",
                &internal,
            ]);
        let mut restored = spawn(command, root.path(), "celld-b.log")?;
        ready(&format!("{celld_endpoint}/state"), &mut restored).await?;
        let remote = S3Storage::new(&config, USER)?;
        assert_eq!(
            remote
                .fetch_blob("song")
                .await?
                .context("blob missing after cold recovery")?,
            b"fixture audio bytes"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let path = root.keep();
        eprintln!("isolated fixture logs retained at {}", path.display());
    }
    result
}
