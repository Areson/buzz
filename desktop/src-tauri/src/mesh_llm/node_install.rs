//! Download-on-first-use install of the `mesh-llm` node binary.
//!
//! Buzz does not compile the mesh node in (that costs ~52 MB of binary);
//! instead the official release artifact is fetched the first time a user
//! starts a mesh node, sha256-verified against the published sidecar
//! checksum, and cached under Buzz's data dir. This mirrors how mesh-llm
//! itself treats its native inference runtime (versioned tarball + checksum
//! + cache) and how Buzz treats models: pay for the capability when you use
//! it, not in the app download.
//!
//! The binary additionally carries an ed25519 release attestation that the
//! node self-verifies and reports via `/api/status` (`release_attestation`),
//! which the runtime layer surfaces after spawn.

use std::path::PathBuf;

use serde::Serialize;
use sha2::Digest as _;
use tauri::Emitter as _;

/// Pinned mesh-llm release. Bump deliberately (with the flag/API surface
/// re-checked) rather than tracking "latest" — the spawn flags and
/// management API below are validated against this version.
pub const MESH_NODE_VERSION: &str = "v0.72.2";

const RELEASE_BASE: &str = "https://github.com/Mesh-LLM/mesh-llm/releases/download";

/// Tauri event emitted with download progress so the UI can show
/// "downloading mesh runtime…" the same way model downloads do.
pub const INSTALL_PROGRESS_EVENT: &str = "mesh-node-install-progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInstallProgress {
    pub version: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub phase: &'static str,
}

/// Release asset name for the current platform, or an explanation of why the
/// platform is unsupported.
fn release_asset() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("mesh-llm-aarch64-apple-darwin.tar.gz"),
        ("linux", "x86_64") => Ok("mesh-llm-x86_64-unknown-linux-gnu.tar.gz"),
        ("linux", "aarch64") => Ok("mesh-llm-aarch64-unknown-linux-gnu.tar.gz"),
        ("windows", "x86_64") => Ok("mesh-llm-x86_64-pc-windows-msvc.zip"),
        (os, arch) => Err(format!(
            "mesh-llm has no release build for {os}/{arch}; see https://github.com/Mesh-LLM/mesh-llm/releases"
        )),
    }
}

fn node_binary_name() -> &'static str {
    if cfg!(windows) {
        "mesh-llm.exe"
    } else {
        "mesh-llm"
    }
}

/// Versioned cache directory for the node binary.
pub fn node_install_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("no platform data dir available")?;
    Ok(base.join("buzz").join("mesh-node").join(MESH_NODE_VERSION))
}

/// Path the installed node binary is expected at (may not exist yet).
pub fn installed_node_path() -> Result<PathBuf, String> {
    Ok(node_install_dir()?.join(node_binary_name()))
}

/// True if the pinned node version is already installed.
pub fn node_installed() -> bool {
    installed_node_path().map(|p| p.is_file()).unwrap_or(false)
}

/// Ensure the pinned mesh-llm node binary is installed, downloading and
/// verifying it if needed. Returns the binary path. Progress is emitted as
/// [`INSTALL_PROGRESS_EVENT`] app events when an `AppHandle` is provided.
pub async fn ensure_node_installed(app: Option<&tauri::AppHandle>) -> Result<PathBuf, String> {
    let path = installed_node_path()?;
    if path.is_file() {
        return Ok(path);
    }

    let asset = release_asset()?;
    let url = format!("{RELEASE_BASE}/{MESH_NODE_VERSION}/{asset}");
    let checksum_url = format!("{url}.sha256");

    let client = reqwest::Client::new();

    // Published checksum first — fail before the big download if missing.
    let checksum_body = client
        .get(&checksum_url)
        .send()
        .await
        .map_err(|e| format!("mesh node checksum fetch failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("mesh node checksum fetch failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("mesh node checksum read failed: {e}"))?;
    let expected_sha = checksum_body
        .split_whitespace()
        .next()
        .filter(|s| s.len() == 64)
        .ok_or("mesh node checksum sidecar is malformed")?
        .to_ascii_lowercase();

    // Stream the archive to a temp file next to the final location (same
    // filesystem → atomic-ish rename), hashing as we go.
    let dir = node_install_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("mesh node cache dir create failed: {e}"))?;
    let archive_path = dir.join(format!("{asset}.partial"));

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("mesh node download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("mesh node download failed: {e}"))?;
    let total_bytes = response.content_length();

    let mut hasher = sha2::Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit: u64 = 0;
    {
        let mut file = tokio::fs::File::create(&archive_path)
            .await
            .map_err(|e| format!("mesh node archive create failed: {e}"))?;
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt as _;
        use tokio::io::AsyncWriteExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("mesh node download failed: {e}"))?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("mesh node archive write failed: {e}"))?;
            // Emit at most every 2 MB so the event stream stays light.
            if downloaded - last_emit >= 2 * 1024 * 1024 {
                last_emit = downloaded;
                emit_progress(app, downloaded, total_bytes, "downloading");
            }
        }
        file.flush()
            .await
            .map_err(|e| format!("mesh node archive flush failed: {e}"))?;
    }

    let actual_sha = hex::encode(hasher.finalize());
    if actual_sha != expected_sha {
        let _ = tokio::fs::remove_file(&archive_path).await;
        return Err(format!(
            "mesh node download checksum mismatch (expected {expected_sha}, got {actual_sha})"
        ));
    }
    emit_progress(app, downloaded, total_bytes, "extracting");

    // Extract on a blocking thread (tar/zip are sync APIs).
    let archive_for_extract = archive_path.clone();
    let dir_for_extract = dir.clone();
    let is_zip = asset.ends_with(".zip");
    tokio::task::spawn_blocking(move || {
        extract_node_binary(&archive_for_extract, &dir_for_extract, is_zip)
    })
    .await
    .map_err(|e| format!("mesh node extract task failed: {e}"))??;

    let _ = tokio::fs::remove_file(&archive_path).await;
    if !path.is_file() {
        return Err("mesh node archive did not contain the mesh-llm binary".to_string());
    }
    emit_progress(app, downloaded, total_bytes, "installed");
    Ok(path)
}

fn emit_progress(
    app: Option<&tauri::AppHandle>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    phase: &'static str,
) {
    if let Some(app) = app {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            NodeInstallProgress {
                version: MESH_NODE_VERSION.to_string(),
                downloaded_bytes,
                total_bytes,
                phase,
            },
        );
    }
}

/// Pull the `mesh-llm` binary (searched by basename, whatever directory
/// layout the archive uses) out of the downloaded archive into `dir`.
fn extract_node_binary(
    archive: &std::path::Path,
    dir: &std::path::Path,
    is_zip: bool,
) -> Result<(), String> {
    let wanted = node_binary_name();
    let dest = dir.join(wanted);
    if is_zip {
        let file = std::fs::File::open(archive).map_err(|e| format!("archive open failed: {e}"))?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|e| format!("zip archive read failed: {e}"))?;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|e| format!("zip entry read failed: {e}"))?;
            let matches = entry
                .enclosed_name()
                .and_then(|p| p.file_name().map(|n| n == wanted))
                .unwrap_or(false);
            if matches {
                let mut out = std::fs::File::create(&dest)
                    .map_err(|e| format!("binary write failed: {e}"))?;
                std::io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("binary write failed: {e}"))?;
                return Ok(());
            }
        }
        Err("mesh-llm binary not found in zip archive".to_string())
    } else {
        let file = std::fs::File::open(archive).map_err(|e| format!("archive open failed: {e}"))?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        for entry in tar
            .entries()
            .map_err(|e| format!("tar archive read failed: {e}"))?
        {
            let mut entry = entry.map_err(|e| format!("tar entry read failed: {e}"))?;
            let matches = entry
                .path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n == wanted))
                .unwrap_or(false);
            if matches {
                entry
                    .unpack(&dest)
                    .map_err(|e| format!("binary unpack failed: {e}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| format!("binary chmod failed: {e}"))?;
                }
                return Ok(());
            }
        }
        Err("mesh-llm binary not found in tar archive".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_asset_known_for_this_platform() {
        // The dev platforms Buzz builds on must all be mapped.
        release_asset().expect("current platform should have a mesh-llm release asset");
    }

    #[test]
    fn install_dir_is_versioned() {
        let dir = node_install_dir().expect("data dir");
        assert!(dir.ends_with(std::path::Path::new("mesh-node").join(MESH_NODE_VERSION)));
    }

    /// Full first-use flow: download the pinned release, verify the
    /// checksum, extract, spawn a client node, drive its management and
    /// OpenAI APIs, and stop it. Network + ~30 MB download; run with
    /// `cargo test --features mesh-llm -- --ignored mesh_node_e2e`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "downloads the mesh-llm release binary; network-dependent"]
    async fn mesh_node_e2e_download_spawn_openai_surface() {
        let binary = ensure_node_installed(None).await.expect("install");
        assert!(binary.is_file());
        assert!(node_installed());

        // Second call is a cache hit (no re-download): must return fast.
        let started = std::time::Instant::now();
        ensure_node_installed(None).await.expect("cache hit");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));

        let node = crate::mesh_llm::node_process::NodeProcess::spawn(
            crate::mesh_llm::node_process::NodeSpawnConfig {
                binary,
                serve: false,
                model: None,
                api_port: 29337,
                console_port: 23131,
                max_vram_gb: None,
                join_tokens: Vec::new(),
            },
        )
        .await
        .expect("spawn client node");

        let status = node.status().await.expect("status");
        assert_eq!(
            status.payload.get("node_state").and_then(|v| v.as_str()),
            Some("client")
        );
        assert!(status.invite_token.is_some(), "invite token published");
        // Release attestation is self-verified by the node.
        assert_eq!(
            status
                .payload
                .pointer("/release_attestation/verified")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        // OpenAI-compatible surface answers.
        let models: serde_json::Value = reqwest::get(format!("{}/models", node.api_base_url()))
            .await
            .expect("GET /v1/models")
            .json()
            .await
            .expect("models json");
        assert_eq!(models.get("object").and_then(|v| v.as_str()), Some("list"));

        node.stop().await.expect("stop");
    }
}
