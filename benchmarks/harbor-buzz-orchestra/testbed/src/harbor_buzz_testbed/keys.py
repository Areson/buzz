"""Nostr keygen and NIP-OA owner attestation for trial agents."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass

import coincurve


@dataclass(frozen=True, slots=True)
class NostrKeypair:
    secret_key: str  # hex
    pubkey: str  # x-only hex


def generate_keypair() -> NostrKeypair:
    """Generate a fresh secp256k1 keypair in Nostr hex form."""
    key = coincurve.PrivateKey()
    return NostrKeypair(
        secret_key=key.to_hex(),
        pubkey=key.public_key_xonly.format().hex(),
    )


def compute_auth_tag(
    owner_secret_key: str, agent_pubkey: str, conditions: str = ""
) -> str:
    """Compute the NIP-OA ``["auth", ...]`` tag authorising an agent key.

    Mirrors crates/buzz-sdk/src/nip_oa.rs:
    sig = schnorr(SHA256("nostr:agent-auth:" || agent_pubkey || ":" || conditions),
    owner_secret_key). Returns the tag as a JSON string.
    """
    owner = coincurve.PrivateKey(bytes.fromhex(owner_secret_key))
    preimage = f"nostr:agent-auth:{agent_pubkey}:{conditions}".encode()
    signature = owner.sign_schnorr(hashlib.sha256(preimage).digest())
    return json.dumps(
        [
            "auth",
            owner.public_key_xonly.format().hex(),
            conditions,
            signature.hex(),
        ],
        separators=(",", ":"),
    )
