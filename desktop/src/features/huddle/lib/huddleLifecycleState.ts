import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import {
  HUDDLE_JOINABLE_WINDOW_SECONDS,
  isHuddleStartStale,
} from "./huddleCardState";

export type HuddleLifecycleState = {
  ended: boolean;
  participants: Set<string>;
  startCreatedAt: number | null;
};

export function huddleEventChannelId(event: RelayEvent): string | null {
  try {
    const parsed = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
    };
    return typeof parsed.ephemeral_channel_id === "string"
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return (
    event.tags.find(
      (tag) => tag[0] === "p" && typeof tag[1] === "string",
    )?.[1] ??
    event.pubkey ??
    null
  );
}

/**
 * Reconstruct one huddle from its lifecycle events.
 *
 * An inferred huddle with no START event stays active while at least one JOIN
 * remains in the window. This preserves late-mount recovery after START ages
 * out of the relay subscription without inventing a participant for a fully
 * drained huddle.
 */
export function reconstructHuddleState(
  events: Iterable<RelayEvent>,
  ephemeralChannelId: string,
  nowMs = Date.now(),
): HuddleLifecycleState {
  const sorted = [...events]
    .filter((event) => huddleEventChannelId(event) === ephemeralChannelId)
    .sort(
      (left, right) =>
        left.created_at - right.created_at ||
        left.kind - right.kind ||
        left.id.localeCompare(right.id),
    );
  let participants = new Set<string>();
  let explicitlyEnded = false;
  let startCreatedAt: number | null = null;

  for (const event of sorted) {
    switch (event.kind) {
      case KIND_HUDDLE_STARTED:
        if (explicitlyEnded) break;
        startCreatedAt = event.created_at;
        participants = new Set(event.pubkey ? [event.pubkey] : []);
        break;
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        if (explicitlyEnded) break;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.add(pubkey);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        if (explicitlyEnded) break;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.delete(pubkey);
        break;
      }
      case KIND_HUDDLE_ENDED:
        explicitlyEnded = true;
        break;
    }
  }

  return {
    ended:
      explicitlyEnded ||
      participants.size === 0 ||
      (startCreatedAt !== null && isHuddleStartStale(startCreatedAt, nowMs)),
    participants,
    startCreatedAt,
  };
}

/** Delay until a fresh START crosses the shared joinable-window boundary. */
export function huddleStalenessDelayMs(
  startCreatedAt: number | null,
  nowMs = Date.now(),
): number | null {
  if (startCreatedAt === null) return null;
  return Math.max(
    0,
    (startCreatedAt + HUDDLE_JOINABLE_WINDOW_SECONDS) * 1000 - nowMs + 1,
  );
}
