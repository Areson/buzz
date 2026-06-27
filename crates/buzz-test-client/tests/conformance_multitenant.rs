//! Multi-tenant conformance harness.
//!
//! This file mirrors the obligation table in `docs/multi-tenant-conformance.md`
//! **one row per module**. It is the executable form of the conformance
//! contract: the current single-community relay is the wire-parity *oracle*, and
//! these tests prove two things the rewrite must never break:
//!
//!   1. **N=1 parity** — with one configured host → one default community, every
//!      existing client observes byte-identical behavior. This is asserted by the
//!      *existing* e2e suites (`e2e_relay`, `e2e_rest_api`, `e2e_media`, …) run
//!      with `RELAY_URL` pointed at the new relay; no new test is needed here,
//!      only the documented obligation that those suites stay green unchanged.
//!
//!   2. **A/B isolation** — with two hosts → two communities on the *same* relay
//!      deployment, no tenant-observable state crosses the boundary. These are
//!      the new tests below. They require a running multi-tenant relay with two
//!      host mappings, so they are `#[ignore]` by default and selected with
//!      `--ignored`.
//!
//! # Running the A/B isolation suite
//!
//! ```text
//! RELAY_URL_A=http://a.localhost:3000 \
//! RELAY_URL_B=http://b.localhost:3000 \
//! cargo test -p buzz-test-client --test conformance_multitenant -- --ignored
//! ```
//!
//! Both URLs MUST address the same relay process (same pod, same Postgres, same
//! Redis); only the `Host` header differs. That is the whole point: one binary,
//! one DB, two communities, provably isolated by `community_id` derived from the
//! host — never from caller input.
//!
//! # Status of each row
//!
//! A row is `todo!()`-stubbed until the lane it depends on lands on the
//! integration branch. The stub is intentional and load-bearing: it names the
//! exact obligation so the lane owner fills in *their* row, and a green run can
//! never be faked by an empty body. Lane ownership is noted per module.

#![allow(clippy::todo, unused)]

use std::time::Duration;

/// Host A's base URL (community A). Defaults to a local two-host relay.
fn url_a() -> String {
    std::env::var("RELAY_URL_A").unwrap_or_else(|_| "http://a.localhost:3000".to_string())
}

/// Host B's base URL (community B), same relay process, different host.
fn url_b() -> String {
    std::env::var("RELAY_URL_B").unwrap_or_else(|_| "http://b.localhost:3000".to_string())
}

/// Marker for a conformance obligation whose lane has not yet landed on the
/// integration branch. Centralizes the "not yet wired" panic so the harvest of
/// remaining work is one grep: `rg pending_lane conformance_multitenant.rs`.
#[track_caller]
fn pending_lane(lane: &str, obligation: &str) -> ! {
    todo!("conformance pending [{lane}]: {obligation}");
}

// ---------------------------------------------------------------------------
// Row zero: request community binding (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod row_zero_host_binding {
    use super::*;

    /// Obligation: an unknown/unmapped host fails closed with a *generic*
    /// rejection and never falls through to a default tenant.
    #[tokio::test]
    #[ignore]
    async fn unmapped_host_fails_closed_generically() {
        pending_lane(
            "relay-wiring",
            "unmapped host → generic rejection, no default tenant, no host echo",
        );
    }

    /// Obligation: a client-supplied `h` tag / token community stamp can never
    /// override the host-derived community; a disagreeing stamp is rejected.
    #[tokio::test]
    #[ignore]
    async fn client_supplied_community_cannot_override_host() {
        pending_lane(
            "relay-wiring",
            "token/h-tag community disagreeing with resolve_host(host) → reject",
        );
    }
}

// ---------------------------------------------------------------------------
// NIP-11 relay info and relay `self` (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod nip11_relay_info {
    use super::*;

    /// Obligation: unauthenticated NIP-11 reads must not become an enumeration
    /// oracle for other communities; `RelayInfo::build` takes only static +
    /// host-scoped inputs (the static-input lint backs this at compile/CI time).
    #[tokio::test]
    #[ignore]
    async fn nip11_is_not_a_cross_community_enumeration_oracle() {
        pending_lane(
            "relay-wiring",
            "NIP-11 from host A reveals nothing about community B's existence/config",
        );
    }
}

// ---------------------------------------------------------------------------
// API tokens and NIP-98 replay (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod api_tokens_nip98_replay {
    use super::*;

    /// Obligation: token hash uniqueness/lookup is `(community_id, token_hash)`;
    /// a token minted in A does not authorize the same hash in B.
    #[tokio::test]
    #[ignore]
    async fn token_minted_in_a_does_not_authorize_in_b() {
        pending_lane(
            "buzz-auth",
            "identical token_hash in A and B → A's token rejected against B",
        );
    }

    /// Obligation: NIP-98 replay seen-set is shared (any-pod) AND community
    /// scoped: a nonce spent in A is still spendable in B, but a replay within A
    /// is rejected from any pod.
    #[tokio::test]
    #[ignore]
    async fn nip98_replay_seenset_is_shared_and_community_scoped() {
        pending_lane(
            "buzz-auth",
            "replay key (community_id, event_id) in shared store; u-host must match req.community",
        );
    }
}

// ---------------------------------------------------------------------------
// Membership / allowlist / archived identities (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod membership_allowlist {
    use super::*;

    /// Obligation: membership/allowlist/archive keyed `(community_id, pubkey)`;
    /// archiving a key in A cannot hide/archive it in B; errors stay generic.
    #[tokio::test]
    #[ignore]
    async fn archive_in_a_does_not_affect_b() {
        pending_lane(
            "buzz-auth",
            "archived_identities (community_id, pubkey) — A's archive invisible to B",
        );
    }
}

// ---------------------------------------------------------------------------
// Users / profiles / NIP-05 / user search (Sami+Quinn — auth+search)
// ---------------------------------------------------------------------------
mod users_profiles_nip05 {
    use super::*;

    /// Obligation: same pubkey can hold a different profile per community; kind:0
    /// replacement is scoped by `(community_id, pubkey)`.
    #[tokio::test]
    #[ignore]
    async fn same_pubkey_distinct_profiles_in_two_communities() {
        pending_lane(
            "buzz-auth",
            "kind:0 replace scoped by (community_id, pubkey); no cross-community inheritance",
        );
    }

    /// Obligation: the same NIP-05 local part can exist on two hosts; lookup only
    /// resolves handles for the requested host/community.
    #[tokio::test]
    #[ignore]
    async fn same_nip05_local_part_on_two_hosts_is_independent() {
        pending_lane(
            "buzz-auth",
            "NIP-05 unique (community_id, lower(handle)); host A lookup never resolves B's",
        );
    }
}

// ---------------------------------------------------------------------------
// Channel-less global events and DMs (Mari+Max — db+pubsub)
// ---------------------------------------------------------------------------
mod channelless_global_events_dms {
    use super::*;

    /// Obligation: same event id / d-tag / pubkey can co-exist in two
    /// communities; direct `GET /api/events/{id}` and REQ filter by community
    /// first; NIP-33 uniqueness is `(community_id, kind, pubkey, d_tag)`.
    #[tokio::test]
    #[ignore]
    async fn same_event_id_and_dtag_coexist_across_communities() {
        pending_lane(
            "buzz-db",
            "same id/d-tag in A and B both retrievable, each scoped; no cross-fetch",
        );
    }

    /// Obligation: a DM `#p` in A does not cross-deliver to B.
    #[tokio::test]
    #[ignore]
    async fn dm_does_not_cross_deliver_between_communities() {
        pending_lane(
            "buzz-pubsub",
            "DM addressed in A never fans out to the same pubkey's B subscription",
        );
    }
}

// ---------------------------------------------------------------------------
// Channels and channel membership (Mari — buzz-db)
// ---------------------------------------------------------------------------
mod channels_membership {
    use super::*;

    /// Obligation: the same channel UUID legitimately co-exists in two
    /// communities (DB PK `(community_id, id)`); an `h` tag resolving to a
    /// channel in another community is rejected generically.
    #[tokio::test]
    #[ignore]
    async fn same_channel_uuid_in_two_communities_is_isolated() {
        pending_lane(
            "buzz-db",
            "channel UUID U exists in A and B; member/post in A never touches B's U",
        );
    }
}

// ---------------------------------------------------------------------------
// Workflows / runs / approvals / webhooks / schedules (Mari+Max)
// ---------------------------------------------------------------------------
mod workflows {
    use super::*;

    /// Obligation: identical workflow UUID / approval-token hash in two
    /// communities are independent; trigger evaluation only sees same-community
    /// events; schedule execution is isolated (at-most-once per community).
    #[tokio::test]
    #[ignore]
    async fn identical_workflow_and_approval_token_are_isolated() {
        pending_lane(
            "buzz-db",
            "same workflow UUID + approval hash in A and B act only within their community",
        );
    }
}

// ---------------------------------------------------------------------------
// Search / FTS (Quinn — buzz-search)
// ---------------------------------------------------------------------------
mod search_fts {
    use super::*;

    /// Obligation: every search `filter` includes `community_id`; same
    /// id/content in A and B return only same-community hits; deleting in A does
    /// not delete the B document. Postgres FTS (search_tsv/GIN), not Typesense.
    #[tokio::test]
    #[ignore]
    async fn search_results_and_deletes_are_community_scoped() {
        pending_lane(
            "buzz-search",
            "FTS query scoped by community_id; delete in A leaves B hit intact",
        );
    }
}

// ---------------------------------------------------------------------------
// Redis pub/sub, presence, typing, cache invalidation (Max — buzz-pubsub)
// ---------------------------------------------------------------------------
mod pubsub_presence_typing {
    use super::*;

    /// Obligation: keys are `buzz:{community}:…`; cross-node fan-out never
    /// delivers an A event to a B subscription, even for the same channel UUID;
    /// the same pubkey can be online in A and away in B independently.
    #[tokio::test]
    #[ignore]
    async fn fanout_and_presence_do_not_cross_communities() {
        pending_lane(
            "buzz-pubsub",
            "event on A's channel UUID never reaches B's subscription on the same UUID",
        );
    }
}

// ---------------------------------------------------------------------------
// Media / Blossom / S3 (Perci — buzz-media)
// ---------------------------------------------------------------------------
mod media_blossom {
    use super::*;

    /// Obligation: public blob `GET/HEAD /media/{sha256.ext}` stays
    /// unauthenticated (N=1 compat, shared CAS bytes). The community boundary is
    /// the metadata/descriptor/upload-auth/quota/audit layer: B's private upload
    /// metadata/errors must not be observable from A, even when the blob bytes
    /// are deduplicated and shared.
    #[tokio::test]
    #[ignore]
    async fn media_metadata_boundary_holds_while_blob_bytes_shared() {
        pending_lane(
            "buzz-media",
            "shared SHA bytes OK; A cannot read B's upload metadata/quota/audit; errors generic",
        );
    }
}

// ---------------------------------------------------------------------------
// Git hosting / NIP-34 / object storage (Perci — buzz-media/git)
// ---------------------------------------------------------------------------
mod git_hosting {
    use super::*;

    /// Obligation: pointer/name keys include community; same owner/repo in two
    /// communities are independent; a push in A does not advance B's pointer.
    #[tokio::test]
    #[ignore]
    async fn same_owner_repo_isolated_push_does_not_cross() {
        pending_lane(
            "buzz-media",
            "repos/{community}/{owner}/{repo}/pointer; push in A leaves B pointer unchanged",
        );
    }
}

// ---------------------------------------------------------------------------
// Mesh / agents / ACP / MCP / CLI (Eva — relay-wiring smoke)
// ---------------------------------------------------------------------------
mod mesh_agents_cli {
    use super::*;

    /// Obligation: one portable key may join multiple communities, but
    /// memberships, DMs, profiles, jobs, and presence do not bleed across them.
    #[tokio::test]
    #[ignore]
    async fn one_key_two_communities_no_bleed() {
        pending_lane(
            "relay-wiring",
            "same key joins A and B; CLI/ACP smoke shows distinct memberships/profiles",
        );
    }
}

// ---------------------------------------------------------------------------
// Audit log and observability (Dawn — buzz-audit)
// ---------------------------------------------------------------------------
mod audit_log {
    use super::*;

    /// Obligation: audit reads verify exactly one community chain
    /// (`(community_id, seq)` / `(community_id, hash)`); error strings must not
    /// leak cross-community IDs, constraint names, or existence facts.
    #[tokio::test]
    #[ignore]
    async fn audit_chain_is_single_community_and_errors_dont_leak() {
        pending_lane(
            "buzz-audit",
            "verify one chain per community; no cross-community id/constraint in error text",
        );
    }
}

// ---------------------------------------------------------------------------
// Migration gate 5: N=1 conformance (Eva — orchestration)
// ---------------------------------------------------------------------------
mod n1_parity {
    //! N=1 parity is asserted by the *existing* e2e suites run against the new
    //! relay with one configured host → one default community. The obligation:
    //! no existing client needs new tags, paths, event fields, CLI flags, or
    //! protocol messages. This module documents the gate; the assertion lives in
    //! the unchanged `e2e_*` suites passing green under `RELAY_URL=<new relay>`.
    //!
    //! Parity runner (driven from the integration job, not a unit test):
    //!   1. Boot new relay, one host → default community, backfill existing data.
    //!   2. Run the full `e2e_*` ignored suite with `RELAY_URL` at the new relay.
    //!   3. Every suite green, unchanged === N=1 parity proven.
}
