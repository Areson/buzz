//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod bridge;
pub mod events;
pub mod git;
pub mod media;
pub mod nip05;

// Re-export imeta helpers used by ingest pipeline.
pub use crate::handlers::imeta::{validate_imeta_tags, verify_imeta_blobs};

// ── Shared helpers (used by media.rs, bridge.rs) ──────────────────────────────

use axum::{http::StatusCode, response::Json};

/// Standard error envelope.
pub(crate) fn api_error(status: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": msg })))
}

pub(crate) fn internal_error(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Internal error: {msg}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

#[allow(dead_code)]
pub(crate) fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    api_error(StatusCode::NOT_FOUND, msg)
}

/// Relay membership enforcement — single gate for all authenticated entry points.
///
/// Moved here from the deleted `relay_members` module. Called by `media.rs`, `bridge.rs`,
/// `git/transport.rs`, and `audio/handler.rs`.
pub mod relay_members {
    use axum::{http::StatusCode, response::Json};
    use tracing::debug;

    use crate::state::AppState;

    /// Outcome of the transport-neutral membership check.
    ///
    /// Distinguishes the four meaningful states without forcing the caller to
    /// produce an HTTP response — used by both the HTTP wrapper
    /// [`enforce_relay_membership`] and by non-HTTP gates (e.g. the iroh-relay
    /// `AccessConfig::Restricted` callback).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MembershipDecision {
        /// The relay does not enforce membership (`require_relay_membership = false`).
        OpenRelay,
        /// The caller's pubkey is in `relay_members` directly.
        Member,
        /// The caller is an agent and its NIP-OA owner is in `relay_members`.
        /// The owner pubkey is included so callers can audit or backfill.
        ViaOwner(nostr::PublicKey),
        /// The caller is not a relay member and no valid NIP-OA delegation applies.
        Denied,
    }

    /// Transport-neutral relay membership check.
    ///
    /// Returns a [`MembershipDecision`] without producing an HTTP response.
    /// HTTP callers should prefer [`enforce_relay_membership`], which maps
    /// `Denied → 403 JSON`. Non-HTTP callers (e.g. iroh-relay access checks)
    /// inspect the decision directly.
    ///
    /// `Err` is reserved for **infrastructure failures** (database errors,
    /// invalid pubkey bytes) — never for the authorization decision itself.
    pub async fn check_relay_membership(
        state: &AppState,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<MembershipDecision, String> {
        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay);
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let is_member = state
            .db
            .is_relay_member(&pubkey_hex)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;

        if is_member {
            return Ok(MembershipDecision::Member);
        }

        if state.config.allow_nip_oa_auth {
            if let Some(tag_json) = auth_tag_header {
                let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes)
                    .map_err(|e| format!("invalid agent pubkey for NIP-OA check: {e}"))?;

                match sprout_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
                    Ok(owner_pubkey) => {
                        let owner_hex = owner_pubkey.to_hex();
                        let owner_is_member =
                            state.db.is_relay_member(&owner_hex).await.map_err(|e| {
                                format!("relay membership check (owner) failed: {e}")
                            })?;

                        if owner_is_member {
                            debug!(
                                agent = %pubkey_hex,
                                owner = %owner_hex,
                                "NIP-OA membership granted via owner"
                            );
                            return Ok(MembershipDecision::ViaOwner(owner_pubkey));
                        }
                    }
                    Err(e) => {
                        debug!(agent = %pubkey_hex, "NIP-OA auth tag invalid: {e}");
                    }
                }
            }
        }

        Ok(MembershipDecision::Denied)
    }

    /// Enforce relay membership for a pubkey, with NIP-OA agent delegation fallback.
    ///
    /// Thin HTTP-layer wrapper around [`check_relay_membership`] that converts
    /// `Denied → 403 JSON` and infra errors → 500 envelope.
    ///
    /// Returns `Ok(Some(owner_pubkey))` when the agent is not a direct member but
    /// its NIP-OA owner *is* — access is granted via delegation.
    ///
    /// On open relays (`require_relay_membership = false`), returns `Ok(None)`
    /// immediately — no membership check is performed. Callers that need NIP-OA
    /// owner extraction on open relays should call [`extract_nip_oa_owner`] directly.
    ///
    /// Returns `Ok(None)` when the caller is a direct member (closed relay) or when
    /// no NIP-OA tag is present/applicable (open relay without auth tag).
    pub async fn enforce_relay_membership(
        state: &AppState,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        match check_relay_membership(state, pubkey_bytes, auth_tag_header).await {
            Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => Ok(None),
            Ok(MembershipDecision::ViaOwner(owner)) => Ok(Some(owner)),
            Ok(MembershipDecision::Denied) => Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "relay_membership_required",
                    "message": "You must be a relay member to access this relay"
                })),
            )),
            Err(e) => {
                tracing::error!("relay membership check errored: {e}");
                Err(super::internal_error(&e))
            }
        }
    }

    /// Extract NIP-OA owner from an auth tag without membership enforcement.
    ///
    /// Used on open relays (`require_relay_membership = false`) to opportunistically
    /// extract the owner pubkey for agent→owner backfill. The NIP-OA signature is
    /// cryptographically self-proving, so no feature flag is needed — if the tag
    /// verifies, the owner relationship is authentic. Returns `None` if the tag
    /// is absent or invalid.
    pub fn extract_nip_oa_owner(
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Option<nostr::PublicKey> {
        let tag_json = auth_tag_header?;
        let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes).ok()?;
        match sprout_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
            Ok(owner) => Some(owner),
            Err(e) => {
                debug!("extract_nip_oa_owner: invalid auth tag: {e}");
                None
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use nostr::Keys;
        use sprout_sdk::nip_oa::compute_auth_tag;

        /// Valid NIP-OA auth tag → returns Some(owner_pubkey).
        #[test]
        fn valid_nip_oa_returns_owner() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "")
                .expect("compute_auth_tag must succeed");

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&tag_json));

            assert_eq!(result, Some(owner_keys.public_key()));
        }

        /// No auth tag → returns None.
        #[test]
        fn no_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), None);

            assert_eq!(result, None);
        }

        /// Invalid auth tag → returns None.
        #[test]
        fn invalid_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some("not valid json"));

            assert_eq!(result, None);
        }
    }
}
