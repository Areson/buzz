//! Mesh owner identity for Buzz-spawned nodes.
//!
//! Mesh's idiomatic peer admission is its ownership/trust layer: each node is
//! attested by an ed25519 *owner keypair* (`mesh-llm auth init`), peers
//! exchange signed node-ownership certificates during gossip, and a node
//! running `--trust-policy allowlist` only admits peers whose verified owner
//! ID is in its trust list (`policy_accepts_peer`). Invite tokens are dial
//! metadata only — this layer is the cryptographic gate.
//!
//! Buzz is the membership authority: it knows which pubkeys are community
//! members. This module gives every Buzz-managed node a local owner identity
//! (auto-initialized once, cached under the app data dir), and the runtime
//! layer wires the trust allowlist from owner IDs that other members publish
//! through Buzz's membership-gated kind:30621 status events. Net effect:
//! only nodes owned by Buzz members are admitted into the mesh, enforced by
//! mesh itself at gossip — not just by who can reach whom.

use std::path::PathBuf;

use tokio::process::Command;

/// Location of this desktop's mesh owner keystore. Lives next to the node
/// binary cache; not versioned — identity survives node upgrades.
pub fn owner_key_path() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("no platform data dir available")?;
    Ok(base.join("buzz").join("mesh-node").join("owner.key"))
}

/// Ensure the owner keystore exists (generating it on first use) and return
/// `(keystore_path, owner_id_hex)`.
///
/// Uses `mesh-llm auth init --no-passphrase`: the keystore protects a mesh
/// compute identity, not funds or messages; it lives in the same app-data
/// scope as Buzz's own secret files (0o600 fallback path) and must be
/// usable by an unattended spawn.
pub async fn ensure_owner_identity(binary: &std::path::Path) -> Result<(PathBuf, String), String> {
    let key_path = owner_key_path()?;
    if let Some(parent) = key_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("owner key dir create failed: {e}"))?;
    }
    if !key_path.is_file() {
        let output = Command::new(binary)
            .args(["auth", "init", "--no-passphrase", "--owner-key"])
            .arg(&key_path)
            .output()
            .await
            .map_err(|e| format!("mesh owner key init failed to run: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "mesh owner key init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    let owner_id = owner_id_from_status(binary, &key_path).await?;
    Ok((key_path, owner_id))
}

/// Read the owner ID out of `mesh-llm auth status`.
async fn owner_id_from_status(
    binary: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<String, String> {
    let output = Command::new(binary)
        .args(["auth", "status", "--owner-key"])
        .arg(key_path)
        .output()
        .await
        .map_err(|e| format!("mesh auth status failed to run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "mesh auth status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    // mesh-llm prints human-readable auth output on stderr (stdout is
    // reserved for machine formats); parse both to stay robust.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_owner_id(&stdout)
        .or_else(|| parse_owner_id(&stderr))
        .ok_or_else(|| "mesh auth status did not report an owner ID".to_string())
}

/// Extract the owner ID from `mesh-llm auth status` output.
fn parse_owner_id(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        if !label.trim().eq_ignore_ascii_case("owner id") {
            return None;
        }
        let value = value.trim();
        (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| value.to_ascii_lowercase())
    })
}

/// Test-only re-export of the parser for sibling test modules.
#[cfg(test)]
pub(crate) fn test_parse_owner_id(text: &str) -> Option<String> {
    parse_owner_id(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_id_from_auth_status_output() {
        let out = "Owner keystore:  /tmp/owner.key\nStatus:          present\nEncrypted:       no\nOwner ID:        32056F9A207C01ABF02AD6B2A095533F117A880DCA317609A666D59AB5D5BD59\nSigning key:     2ce7ac5e32f3ad4c2fd52367687b220cd65538c3a27474ed5809bcf8ea066fce\n";
        assert_eq!(
            parse_owner_id(out).as_deref(),
            Some("32056f9a207c01abf02ad6b2a095533f117a880dca317609a666d59ab5d5bd59")
        );
    }

    #[test]
    fn rejects_missing_or_malformed_owner_id() {
        assert_eq!(parse_owner_id("Status: present\n"), None);
        assert_eq!(parse_owner_id("Owner ID: nothex\n"), None);
        assert_eq!(parse_owner_id("Owner ID: 1234\n"), None);
    }
}
