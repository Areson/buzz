/**
 * Unit tests for the OnboardingRelayConnectionErrorCard phase-3 reconnect contract.
 *
 * Tests the guard logic that ties relay connection state to markSuccess():
 * - phase-3 path (reconnect() returns false): card marks success when relay
 *   transitions to "connected" while a reconnect was attempted.
 * - disconnected state never triggers success.
 * - no prior reconnect attempt: connected state is ignored.
 *
 * These tests exercise the observable contract without React rendering by
 * simulating the component's ref/effect logic as a pure state machine.
 */

import assert from "node:assert/strict";
import test from "node:test";

/**
 * Minimal simulation of the OnboardingRelayConnectionErrorCard guard logic.
 *
 * Models the three-variable contract:
 *   hadActiveReconnect (ref) — set true when a reconnect is started
 *   relayConnectionState     — driven by useRelayConnection
 *   hasSuccess (state)       — becomes true when guard fires markSuccess
 */
function makeCard() {
  let hadActiveReconnect = false;
  let hasSuccess = false;
  let markSuccessCallCount = 0;

  const markSuccess = () => {
    hasSuccess = true;
    markSuccessCallCount++;
  };

  // Simulates what runConnectivityAction does: arm the ref, call reconnect,
  // then either mark success synchronously (phase 1) or leave ref armed (phase 3).
  const startReconnect = async (reconnectResult) => {
    hadActiveReconnect = true;
    const didReconnect = await reconnectResult;
    if (didReconnect !== false) {
      // Phase 1 sync success — clear ref and mark immediately.
      hadActiveReconnect = false;
      markSuccess();
    }
    // Phase 3: ref stays armed, waiting for connection-state effect.
  };

  // Simulates the useEffect(() => { if (connected && ref) markSuccess() })
  const driveConnectionState = (state) => {
    if (state === "connected" && hadActiveReconnect) {
      hadActiveReconnect = false;
      markSuccess();
    }
  };

  return {
    startReconnect,
    driveConnectionState,
    get hasSuccess() {
      return hasSuccess;
    },
    get markSuccessCallCount() {
      return markSuccessCallCount;
    },
  };
}

// ── Phase 3: async success via connection-state ───────────────────────────────

test("phase-3 path — reconnect() returns false, connected fires, markSuccess called once", async () => {
  const card = makeCard();

  // Simulate reconnect() entering phase 3 (returns false).
  await card.startReconnect(Promise.resolve(false));

  // Controller eventually drives relay to connected.
  card.driveConnectionState("connected");

  assert.equal(card.hasSuccess, true, "card marks success on connected");
  assert.equal(card.markSuccessCallCount, 1, "markSuccess called exactly once");
});

test("phase-3 path — disconnected state does not mark success", async () => {
  const card = makeCard();

  await card.startReconnect(Promise.resolve(false));

  // Drive through several non-connected states.
  card.driveConnectionState("disconnected");
  card.driveConnectionState("reconnecting");
  card.driveConnectionState("stalled");

  assert.equal(card.hasSuccess, false, "no success on non-connected states");
  assert.equal(card.markSuccessCallCount, 0, "markSuccess never called");
});

test("no reconnect attempt — connected state does not mark success", async () => {
  const card = makeCard();

  // Drive to connected without any reconnect attempt.
  card.driveConnectionState("connected");

  assert.equal(
    card.hasSuccess,
    false,
    "pre-existing connected state does not mark success",
  );
  assert.equal(card.markSuccessCallCount, 0, "markSuccess never called");
});

test("phase-1 sync success — markSuccess fires immediately, ref cleared, later connected is ignored", async () => {
  const card = makeCard();

  // Phase 1: reconnect() returns true (sync success).
  await card.startReconnect(Promise.resolve(true));

  assert.equal(card.hasSuccess, true, "success marked synchronously");
  assert.equal(card.markSuccessCallCount, 1, "called once");

  // A subsequent connected transition should NOT call markSuccess again.
  card.driveConnectionState("connected");

  assert.equal(
    card.markSuccessCallCount,
    1,
    "no double-success on later connected",
  );
});

test("phase-3 path — connected fires multiple times, markSuccess still called only once", async () => {
  const card = makeCard();

  await card.startReconnect(Promise.resolve(false));

  card.driveConnectionState("connected");
  card.driveConnectionState("connected"); // spurious second emission

  assert.equal(
    card.markSuccessCallCount,
    1,
    "ref cleared after first connected, no double-success",
  );
});
