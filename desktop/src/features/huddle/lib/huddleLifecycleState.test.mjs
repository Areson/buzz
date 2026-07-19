import assert from "node:assert/strict";
import test from "node:test";

import {
  huddleStalenessDelayMs,
  reconstructHuddleState,
} from "./huddleLifecycleState.ts";

const HUDDLE_ID = "huddle-id";
const CREATOR = "a".repeat(64);
const PARTICIPANT = "b".repeat(64);
const NOW_SECONDS = 2_000_000;

function lifecycleEvent(kind, overrides = {}) {
  return {
    id: `${kind}-${overrides.created_at ?? NOW_SECONDS}`,
    pubkey: CREATOR,
    created_at: NOW_SECONDS,
    kind,
    tags: [],
    content: JSON.stringify({ ephemeral_channel_id: HUDDLE_ID }),
    sig: "",
    ...overrides,
  };
}

test("reconstructHuddleState ends an explicitly ended huddle", () => {
  const state = reconstructHuddleState(
    [lifecycleEvent(48100), lifecycleEvent(48103)],
    HUDDLE_ID,
    NOW_SECONDS * 1000,
  );

  assert.equal(state.ended, true);
  assert.equal(state.startCreatedAt, NOW_SECONDS);
});

test("reconstructHuddleState ends a fully drained huddle", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48101, { tags: [["p", PARTICIPANT]] }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS + 1,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS + 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    (NOW_SECONDS + 1) * 1000,
  );

  assert.equal(state.ended, true);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState ends a stale huddle and retains its start time", () => {
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
  const state = reconstructHuddleState(
    [lifecycleEvent(48100, { created_at: startCreatedAt })],
    HUDDLE_ID,
    NOW_SECONDS * 1000,
  );

  assert.equal(state.ended, true);
  assert.equal(state.startCreatedAt, startCreatedAt);
  assert.deepEqual([...state.participants], [CREATOR]);
});

test("reconstructHuddleState keeps real joins active when START aged out", () => {
  const state = reconstructHuddleState(
    [lifecycleEvent(48101, { tags: [["p", PARTICIPANT]] })],
    HUDDLE_ID,
    NOW_SECONDS * 1000,
  );

  assert.equal(state.ended, false);
  assert.equal(state.startCreatedAt, null);
  assert.deepEqual([...state.participants], [PARTICIPANT]);
});

test("reconstructHuddleState does not resurrect after an end event", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48103, { created_at: NOW_SECONDS + 1 }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS + 2,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    HUDDLE_ID,
    (NOW_SECONDS + 2) * 1000,
  );

  assert.equal(state.ended, true);
  assert.deepEqual([...state.participants], [CREATOR]);
});

test("huddleStalenessDelayMs schedules just past the stale boundary", () => {
  assert.equal(
    huddleStalenessDelayMs(NOW_SECONDS - 60 * 60 + 10, NOW_SECONDS * 1000),
    10_001,
  );
  assert.equal(huddleStalenessDelayMs(null, NOW_SECONDS * 1000), null);
});
