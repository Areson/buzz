import { useEffect, useState } from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Mesh-LLM offer envelope as carried in the `content` field of a kind:31990
 * event. Keep in sync with the Rust `sprout_core::mesh_llm::MeshLlmOffer`.
 */
export interface MeshLlmOffer {
  v: number;
  d_tag: string;
  endpoint_id: string;
  iroh_relay_url: string;
  caps: {
    max_vram_mb?: number | null;
    max_ram_mb?: number | null;
    max_concurrency?: number | null;
  };
  models: Array<{
    id: string;
    label?: string | null;
    context_tokens?: number | null;
  }>;
  extra?: unknown;
}

/**
 * A kind:31990 offer paired with the *Nostr* pubkey that signed it (so the
 * UI can show 'Alice is offering Llama 3 8B') and the event's `created_at`
 * (for sorting and freshness display).
 */
export interface ResolvedOffer {
  offer: MeshLlmOffer;
  pubkey: string;
  createdAt: number;
  d_tag: string;
}

function extractDTag(event: RelayEvent): string | null {
  for (const tag of event.tags) {
    if (tag.length >= 2 && tag[0] === "d") return tag[1];
  }
  return null;
}

/**
 * Subscribe to live mesh-LLM offers from the connected relay.
 *
 * Returns the de-duplicated set of *currently-active* offers (keyed by
 * `(pubkey, d_tag)` per NIP-33). An event with empty `content` is treated
 * as 'offer withdrawn' and removes the corresponding entry — this is the
 * NIP-33 delete-by-replace idiom the Rust publisher emits when the user
 * toggles compute-sharing off.
 */
export function useMeshLlmOffers(): {
  offers: ResolvedOffer[];
  error: string | null;
} {
  const [offers, setOffers] = useState<Map<string, ResolvedOffer>>(new Map());
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unsub: (() => Promise<void>) | null = null;

    function onEvent(event: RelayEvent) {
      if (cancelled) return;
      const dTag = extractDTag(event);
      if (!dTag) return;
      const key = `${event.pubkey}:${dTag}`;

      // Empty content = NIP-33 delete-by-replace.
      if (event.content.trim() === "") {
        setOffers((prev) => {
          if (!prev.has(key)) return prev;
          const next = new Map(prev);
          next.delete(key);
          return next;
        });
        return;
      }

      let parsed: MeshLlmOffer;
      try {
        parsed = JSON.parse(event.content) as MeshLlmOffer;
      } catch {
        // Skip malformed offers silently; one bad publisher must not
        // poison the list.
        return;
      }
      setOffers((prev) => {
        const existing = prev.get(key);
        if (existing && existing.createdAt >= event.created_at) {
          // We already have a fresher version under the same address.
          return prev;
        }
        const next = new Map(prev);
        next.set(key, {
          offer: parsed,
          pubkey: event.pubkey,
          createdAt: event.created_at,
          d_tag: dTag,
        });
        return next;
      });
    }

    (async () => {
      try {
        const u = await relayClient.subscribeToMeshLlmOffers(onEvent);
        if (cancelled) {
          void u();
        } else {
          unsub = u;
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();

    return () => {
      cancelled = true;
      if (unsub) void unsub();
    };
  }, []);

  // Sort newest first for the UI.
  const list = Array.from(offers.values()).sort(
    (a, b) => b.createdAt - a.createdAt,
  );
  return { offers: list, error };
}
