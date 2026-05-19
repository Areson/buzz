//! Canonical URL builder for NIP-98 `u`-tag signing and verification.
//!
//! Both the **signer** (e.g. desktop iroh-relay bearer-token producer) and the
//! **verifier** (e.g. the iroh-relay `AccessConfig::Restricted` callback) must
//! compute the same canonical URL string. Any drift between the two sides
//! produces a `URL mismatch` rejection on every single connection, which is
//! the canonical NIP-98 deploy bug.
//!
//! This module centralises the canonicalisation rules so they cannot drift:
//!
//! 1. Scheme/host are lowercased by [`url::Url`].
//! 2. `localhost` and `::1` collapse to `127.0.0.1` (so dev signers that bind
//!    `[::]` and verifiers that see `127.0.0.1` agree).
//! 3. Query and fragment are stripped (NIP-98 signs the URL "path identity",
//!    not transient query parameters).
//! 4. Trailing slashes on the path are collapsed to a single canonical form.
//! 5. Path-prefix joins are suffix-aware: `base=https://h/iroh` joined with
//!    `path=/relay` yields `https://h/iroh/relay`, NOT `https://h/relay`
//!    (which is what [`url::Url::join`] would produce).
//!
//! The single canonical-string format is consumed by both
//! [`crate::verify_nip98_event`] and external signers; the round-trip test
//! pins them together.

use url::Url;

/// Build the canonical NIP-98 `u`-tag value for a request, joining a base URL
/// with a (potentially absolute) path while preserving any base path prefix.
///
/// Returns `None` if `base` is not a parseable URL.
///
/// # Examples
///
/// Plain join:
///
/// ```
/// use sprout_auth::nip98_canonical_url;
/// assert_eq!(
///     nip98_canonical_url("https://relay.example.com", "/iroh/relay").as_deref(),
///     Some("https://relay.example.com/iroh/relay"),
/// );
/// ```
///
/// Path-prefix preservation (the typical reverse-proxy case):
///
/// ```
/// use sprout_auth::nip98_canonical_url;
/// assert_eq!(
///     nip98_canonical_url("https://relay.example.com/iroh", "/relay").as_deref(),
///     Some("https://relay.example.com/iroh/relay"),
/// );
/// ```
pub fn nip98_canonical_url(base: &str, path: &str) -> Option<String> {
    let mut parsed = Url::parse(base).ok()?;

    // localhost collapse — `Url::host_str()` returns the canonicalised host
    // string, which for IPv6 omits brackets (`"::1"`) but for v4-mapped or
    // alternate IPv6 spellings may yield different forms. We compare against
    // the parsed `Host` enum where available to catch all loopback shapes.
    let is_loopback = match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        None => false,
    };
    if is_loopback {
        parsed.set_host(Some("127.0.0.1")).ok()?;
    }

    // Suffix-join: append `path` to the base's path, keeping the prefix.
    let base_path = parsed.path().trim_end_matches('/').to_string();
    let suffix = path.trim_start_matches('/');
    let joined = if suffix.is_empty() {
        base_path
    } else if base_path.is_empty() {
        format!("/{suffix}")
    } else {
        format!("{base_path}/{suffix}")
    };
    let collapsed = joined.trim_end_matches('/').to_string();
    let final_path = if collapsed.is_empty() {
        "/".to_string()
    } else {
        collapsed
    };
    parsed.set_path(&final_path);

    // Strip query + fragment — NIP-98 signs path identity, not transient args.
    parsed.set_query(None);
    parsed.set_fragment(None);

    Some(parsed.to_string())
}

/// Build a canonical NIP-98 `u`-tag value from a fully-qualified URL.
///
/// Useful on the verifier side, where the caller has already reconstructed
/// the full request URL (e.g. from `X-Forwarded-Proto` + `Host` + path) and
/// only needs canonicalisation.
pub fn nip98_canonicalize(url: &str) -> Option<String> {
    nip98_canonical_url(url, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    #[test]
    fn plain_join_no_base_path() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com", "/iroh/relay").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn suffix_join_preserves_base_path_prefix() {
        // The classic Plan v4 deploy bug: signer reads `iroh_relay_url` from
        // NIP-11 as `https://host/iroh` and joins path `/relay`. `Url::join`
        // would discard the `/iroh` prefix; the canonical helper must keep it.
        assert_eq!(
            nip98_canonical_url("https://relay.example.com/iroh", "/relay").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn trailing_slash_on_base_collapsed() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com/iroh/", "/relay").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn trailing_slash_on_path_collapsed() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com/iroh", "/relay/").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn localhost_collapses_to_loopback() {
        assert_eq!(
            nip98_canonical_url("http://localhost:3000", "/iroh/relay").as_deref(),
            Some("http://127.0.0.1:3000/iroh/relay"),
        );
    }

    #[test]
    fn ipv6_loopback_collapses_to_loopback() {
        assert_eq!(
            nip98_canonical_url("http://[::1]:3000", "/iroh/relay").as_deref(),
            Some("http://127.0.0.1:3000/iroh/relay"),
        );
    }

    #[test]
    fn explicit_port_is_preserved() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com:8443", "/iroh/relay").as_deref(),
            Some("https://relay.example.com:8443/iroh/relay"),
        );
    }

    #[test]
    fn query_and_fragment_stripped() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com/iroh?foo=bar#x", "/relay").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn scheme_and_host_lowercased() {
        assert_eq!(
            nip98_canonical_url("HTTPS://Relay.Example.COM", "/iroh/relay").as_deref(),
            Some("https://relay.example.com/iroh/relay"),
        );
    }

    #[test]
    fn empty_path_yields_root() {
        assert_eq!(
            nip98_canonical_url("https://relay.example.com", "").as_deref(),
            Some("https://relay.example.com/"),
        );
    }

    #[test]
    fn invalid_base_returns_none() {
        assert!(nip98_canonical_url("not a url", "/iroh/relay").is_none());
    }

    #[test]
    fn canonicalize_full_url_round_trip() {
        let canonical = nip98_canonicalize("https://relay.example.com:8443/iroh/relay?x=1#y")
            .expect("canonicalize must succeed");
        assert_eq!(canonical, "https://relay.example.com:8443/iroh/relay");
    }

    /// **Critical round-trip test:** signs an event using the canonical helper,
    /// then verifies it through [`crate::verify_nip98_event`]. If they drift,
    /// every connection in production deny-loops.
    #[test]
    fn round_trip_with_verify_nip98_event() {
        let keys = Keys::generate();
        let canonical = nip98_canonical_url("https://relay.example.com/iroh", "/relay").unwrap();

        let event = EventBuilder::new(
            Kind::HttpAuth,
            "",
            vec![
                Tag::parse(&["u", &canonical]).unwrap(),
                Tag::parse(&["method", "GET"]).unwrap(),
            ],
        )
        .custom_created_at(Timestamp::now())
        .sign_with_keys(&keys)
        .unwrap();
        let json = serde_json::to_string(&event).unwrap();

        // Verifier reconstructs the exact same canonical URL — different inputs,
        // same string.
        let verifier_url =
            nip98_canonical_url("https://relay.example.com/iroh/", "/relay/").unwrap();

        let result = crate::verify_nip98_event(&json, &verifier_url, "GET", None);
        assert!(result.is_ok(), "round-trip verify failed: {:?}", result);
        assert_eq!(result.unwrap(), keys.public_key());
    }
}
