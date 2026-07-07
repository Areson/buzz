//! Spawned mesh-llm node process + management-API client.
//!
//! Replaces the in-process `mesh_llm_sdk::serve/client::start()` embedding
//! (which statically linked the whole node, ~52 MB) with the same pattern
//! Buzz uses for every other capability binary: a supervised child process
//! driven over its local HTTP surface. The SDK's own embedded handle already
//! spoke HTTP to itself (`GET {console}/api/status`), so this changes where
//! the node lives, not how it is driven:
//!
//!   - inference:      http://127.0.0.1:{api_port}/v1 (OpenAI-compatible)
//!   - management:     http://127.0.0.1:{console_port}/api/status
//!
//! Admission/coordination is mesh-native and unchanged: the node does its
//! own Nostr discovery, invite-token admission, and iroh transport exactly
//! as `mesh-llm serve/client` does standalone.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

/// One spawned mesh-llm node.
pub struct NodeProcess {
    child: Child,
    api_port: u16,
    console_port: u16,
}

/// Mirror of the SDK's `EmbeddedNodeStatus`: raw status payload plus the
/// derived fields callers use.
pub struct NodeStatus {
    pub api_base_url: String,
    pub console_url: String,
    pub invite_token: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub struct NodeSpawnConfig {
    pub binary: PathBuf,
    /// `serve` (share a model) or `client` (consume only).
    pub serve: bool,
    pub model: Option<String>,
    pub api_port: u16,
    pub console_port: u16,
    pub max_vram_gb: Option<f64>,
    pub join_tokens: Vec<String>,
    /// Owner keystore attesting this node (mesh ownership layer). Required:
    /// Buzz-managed nodes always run attested so peers can admit them by
    /// owner ID.
    pub owner_key: PathBuf,
    /// Verified owner IDs admitted at gossip (`--trust-policy allowlist`).
    /// Buzz builds this from owner IDs published by community members via
    /// the membership-gated kind:30621 pipeline; this node's own owner ID
    /// is always included. Mesh enforces the gate cryptographically —
    /// unattested peers and peers with unlisted owners are rejected even
    /// if they hold a valid invite token.
    pub trusted_owner_ids: Vec<String>,
    /// `BUZZ_MANAGED_AGENT` instance stamp (same scheme as managed agents)
    /// so the orphan sweep can reclaim a node left behind by a crashed
    /// desktop without ever touching a user's standalone mesh-llm.
    pub instance_id: Option<String>,
}

impl NodeProcess {
    /// Spawn the node and wait for its management API to come up.
    pub async fn spawn(config: NodeSpawnConfig) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.binary);
        cmd.arg(if config.serve { "serve" } else { "client" });
        if let Some(model) = config.model.as_deref() {
            cmd.arg("--model").arg(model);
        }
        cmd.arg("--port").arg(config.api_port.to_string());
        cmd.arg("--console").arg(config.console_port.to_string());
        // Headless: management API without the web console UI. Private by
        // default (no --publish): joinable only via invite token, matching
        // the embedded config (`publish(false)`, `auto_join(false)`).
        cmd.arg("--headless");
        cmd.arg("--disable-iroh-relays");
        cmd.arg("--mesh-discovery-mode").arg("nostr");
        cmd.arg("--log-format").arg("json");
        if let Some(max_vram) = config.max_vram_gb {
            cmd.arg("--max-vram").arg(max_vram.to_string());
        }
        for token in &config.join_tokens {
            cmd.arg("--join").arg(token);
        }
        // Ownership + trust: mesh's idiomatic admission. This node is
        // attested by the Buzz-managed owner key (--owner-required makes a
        // broken keystore a startup failure, not a silent downgrade), and
        // only peers with verified, allowlisted owners are admitted at
        // gossip. Buzz's membership is the source of the allowlist.
        cmd.arg("--owner-key").arg(&config.owner_key);
        cmd.arg("--owner-required");
        cmd.arg("--trust-policy").arg("allowlist");
        for owner_id in &config.trusted_owner_ids {
            cmd.arg("--trust-owner").arg(owner_id);
        }
        if let Some(instance_id) = config.instance_id.as_deref() {
            cmd.env("BUZZ_MANAGED_AGENT", instance_id);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            // Own process group so teardown can signal the whole tree.
            cmd.process_group(0);
        }

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("mesh-llm node spawn failed: {e}"))?;

        let node = Self {
            child,
            api_port: config.api_port,
            console_port: config.console_port,
        };
        node.wait_ready(Duration::from_secs(60)).await?;
        Ok(node)
    }

    pub fn api_base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.api_port)
    }

    pub fn console_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.console_port)
    }

    async fn wait_ready(&self, timeout: Duration) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let url = format!("{}/api/status", self.console_url());
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("mesh-llm node did not become ready within {timeout:?}");
            }
            if let Ok(response) = reqwest::get(&url).await {
                if response.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Fetch node status from the management API.
    pub async fn status(&self) -> anyhow::Result<NodeStatus> {
        let url = format!("{}/api/status", self.console_url());
        let payload: serde_json::Value = reqwest::get(&url)
            .await
            .map_err(|e| anyhow::anyhow!("mesh node status fetch failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("mesh node status fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("mesh node status parse failed: {e}"))?;
        let invite_token = payload
            .get("token")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        Ok(NodeStatus {
            api_base_url: self.api_base_url(),
            console_url: self.console_url(),
            invite_token,
            payload,
        })
    }

    /// Stop the node process (consuming form).
    pub async fn stop(mut self) -> anyhow::Result<()> {
        self.take_stop().await
    }

    /// Stop the node process in place (for callers that only have `&mut`,
    /// e.g. replacing the node behind a mutex). Safe to call twice.
    pub async fn take_stop(&mut self) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            // Graceful first: SIGTERM the process group, then escalate.
            if let Some(pid) = self.child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGTERM);
                }
                let grace = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
                if grace.is_ok() {
                    return Ok(());
                }
            }
        }
        self.child
            .kill()
            .await
            .map_err(|e| anyhow::anyhow!("mesh node kill failed: {e}"))?;
        Ok(())
    }
}
