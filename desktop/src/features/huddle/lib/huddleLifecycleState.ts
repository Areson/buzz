import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import { HUDDLE_JOINABLE_WINDOW_SECONDS } from "./huddleCardState";

export type HuddleLifecycleState = {
  ended: boolean;
  participants: Set<string>;
  startCreatedAt: number | null;
  staleDeadlineMs: number | null;
};

type ReconstructHuddleOptions = {
  historyMayBeTruncated?: boolean;
  isCurrentHuddle?: boolean;
  nowMs?: number;
};

type SelectActiveHuddleOptions = {
  activeEphemeralChannelId?: string | null;
  historyMayBeTruncated?: boolean;
  nowMs?: number;
};

export const HUDDLE_EVENT_HISTORY_LIMIT = 100;

export type ActiveHuddleLifecycleState = {
  ephemeralChannelId: string;
  state: HuddleLifecycleState;
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
 * An inferred huddle with no START event stays non-terminal because the
 * subscription window may have truncated an older participant JOIN. A retained
 * START makes an empty reconstructed participant set conclusive.
 */
export function reconstructHuddleState(
  events: Iterable<RelayEvent>,
  ephemeralChannelId: string,
  options: ReconstructHuddleOptions = {},
): HuddleLifecycleState {
  const matchingEvents = [...events].filter(
    (event) => huddleEventChannelId(event) === ephemeralChannelId,
  );
  const startEvent = matchingEvents
    .filter((event) => event.kind === KIND_HUDDLE_STARTED)
    .sort(
      (left, right) =>
        left.created_at - right.created_at || left.id.localeCompare(right.id),
    )
    .at(-1);
  const participantEvents = matchingEvents
    .filter(
      (event) =>
        event.kind === KIND_HUDDLE_PARTICIPANT_JOINED ||
        event.kind === KIND_HUDDLE_PARTICIPANT_LEFT,
    )
    .sort(
      (left, right) =>
        left.created_at - right.created_at ||
        left.kind - right.kind ||
        left.id.localeCompare(right.id),
    );
  const participants = new Set<string>(
    startEvent?.pubkey ? [startEvent.pubkey] : [],
  );
  const explicitlyEnded = matchingEvents.some(
    (event) => event.kind === KIND_HUDDLE_ENDED,
  );
  const startCreatedAt = startEvent?.created_at ?? null;

  // START is client-signed while participant transitions are relay-signed, so
  // their created_at values are not one causal clock. Seed the creator from
  // START, then fold only relay participant transitions in their own order.
  if (!explicitlyEnded) {
    for (const event of participantEvents) {
      switch (event.kind) {
        case KIND_HUDDLE_PARTICIPANT_JOINED: {
          const pubkey = lifecycleParticipant(event);
          if (pubkey) participants.add(pubkey);
          break;
        }
        case KIND_HUDDLE_PARTICIPANT_LEFT: {
          const pubkey = lifecycleParticipant(event);
          if (pubkey) participants.delete(pubkey);
          break;
        }
      }
    }
  }

  // An empty set is conclusive only when START is retained and the replay did
  // not hit its limit. Under clock skew, START can survive in a truncated
  // window even when an earlier relay-signed JOIN did not.
  const drained =
    startCreatedAt !== null &&
    !options.historyMayBeTruncated &&
    participants.size === 0;
  // START age is only a fallback for a huddle that never produced newer
  // lifecycle evidence. The relay TTL is renewable, so a later JOIN/LEFT or
  // the locally current huddle must not be expired from START time alone.
  const staleDeadlineMs =
    startCreatedAt !== null &&
    participantEvents.length === 0 &&
    !options.isCurrentHuddle &&
    !explicitlyEnded &&
    !drained
      ? (startCreatedAt + HUDDLE_JOINABLE_WINDOW_SECONDS) * 1000 + 1
      : null;
  const nowMs = options.nowMs ?? Date.now();

  return {
    ended:
      explicitlyEnded ||
      drained ||
      (staleDeadlineMs !== null && nowMs >= staleDeadlineMs),
    participants,
    startCreatedAt,
    staleDeadlineMs,
  };
}

/**
 * Select the channel header's huddle without falling back past a newer ended
 * session. Retained START events are the session boundaries; participant and
 * END timestamps never compete with a different client's START timestamp.
 */
export function selectActiveHuddleState(
  events: Iterable<RelayEvent>,
  options: SelectActiveHuddleOptions = {},
): ActiveHuddleLifecycleState | null {
  const allEvents = [...events];
  const historyMayBeTruncated =
    options.historyMayBeTruncated ??
    allEvents.length >= HUDDLE_EVENT_HISTORY_LIMIT;
  const eventsByHuddle = new Map<string, RelayEvent[]>();
  for (const event of allEvents) {
    const ephemeralChannelId = huddleEventChannelId(event);
    if (!ephemeralChannelId) continue;
    const huddleEvents = eventsByHuddle.get(ephemeralChannelId) ?? [];
    huddleEvents.push(event);
    eventsByHuddle.set(ephemeralChannelId, huddleEvents);
  }

  const candidates = [...eventsByHuddle.entries()].map(
    ([ephemeralChannelId, huddleEvents]) => ({
      ephemeralChannelId,
      state: reconstructHuddleState(huddleEvents, ephemeralChannelId, {
        historyMayBeTruncated,
        isCurrentHuddle:
          options.activeEphemeralChannelId === ephemeralChannelId,
        nowMs: options.nowMs,
      }),
      lastEventCreatedAt: Math.max(
        ...huddleEvents.map((event) => event.created_at),
      ),
    }),
  );

  const current = candidates.find(
    ({ ephemeralChannelId, state }) =>
      ephemeralChannelId === options.activeEphemeralChannelId && !state.ended,
  );
  if (current) {
    return {
      ephemeralChannelId: current.ephemeralChannelId,
      state: current.state,
    };
  }

  // A retained START is a session boundary. Choose the newest session before
  // applying terminal state so an older incomplete history cannot reappear
  // after a newer huddle ends. Only use relay activity as a fallback when the
  // subscription window contains no START at all.
  const startedCandidates = candidates.filter(
    ({ state }) => state.startCreatedAt !== null,
  );
  const candidatePool =
    startedCandidates.length > 0 ? startedCandidates : candidates;
  candidatePool.sort(
    (left, right) =>
      (right.state.startCreatedAt ?? right.lastEventCreatedAt) -
        (left.state.startCreatedAt ?? left.lastEventCreatedAt) ||
      right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
  );

  const selected = candidatePool[0];
  if (!selected || selected.state.ended) return null;
  return {
    ephemeralChannelId: selected.ephemeralChannelId,
    state: selected.state,
  };
}

/** Delay until an unconfirmed START crosses the shared stale boundary. */
export function huddleStalenessDelayMs(
  staleDeadlineMs: number | null,
  nowMs = Date.now(),
): number | null {
  if (staleDeadlineMs === null) return null;
  return Math.max(0, staleDeadlineMs - nowMs);
}
