import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Regression test for the agent activity / observer feed "jump back a
 * couple lines" bug.
 *
 * The hook `useStickToBottom` (used by `AgentSessionThreadPanel`)
 * scrolls smoothly to the bottom when new content arrives while the
 * user is already near the bottom. A browser `behavior: "smooth"`
 * scroll emits intermediate `scroll` events whose `scrollTop` is
 * mid-animation — well above `scrollHeight - clientHeight - 100`. The
 * old `onScroll` recomputed `isNearBottom` from raw `scrollTop`, so
 * those intermediate events flipped it to `false` mid-flight. The next
 * content append refused to follow the bottom and the feed appeared
 * to "jump back a couple lines."
 *
 * The fix makes `onScroll` direction-aware: distance < 100 always
 * sticks; otherwise, only `scrollTop` decreasing (user pulling up)
 * unsticks. Intermediate animation events climb toward the bottom, so
 * they leave the sticky bit alone.
 *
 * The two specs below pin both halves of that contract:
 *   1. A mid-animation intermediate scroll event does not unstick.
 *   2. A genuine user scroll-up still unsticks (no over-correction).
 */

type StickFixtureState = {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  isNearBottom: boolean;
  distanceFromBottom: number;
  itemCount: number;
};

type StickFixtureWindow = Window & {
  __BUZZ_E2E_STICK_FIXTURE__: {
    push: (text?: string) => number;
    scrollToBottom: () => void;
    scrollToTop: () => void;
    simulateScroll: (scrollTop: number) => void;
    state: () => StickFixtureState;
  };
};

async function mountFixture(
  page: import("@playwright/test").Page,
  seedItems: number,
): Promise<void> {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_INSTALL_STICK_FIXTURE__ === "function",
  );
  await page.evaluate(async () => {
    const install = window.__BUZZ_E2E_INSTALL_STICK_FIXTURE__;
    if (!install) throw new Error("install hook missing");
    await install();
  });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_MOUNT_STICK_FIXTURE__ === "function",
  );
  await page.evaluate((seeds) => {
    const mount = window.__BUZZ_E2E_MOUNT_STICK_FIXTURE__;
    if (!mount) throw new Error("mount hook missing");
    const handle = mount({ seedItems: seeds });
    (window as unknown as StickFixtureWindow).__BUZZ_E2E_STICK_FIXTURE__ =
      handle;
  }, seedItems);
  // React 19's createRoot registers the handle in a microtask; poll
  // until state() doesn't throw.
  await page.waitForFunction(() => {
    try {
      (
        window as unknown as StickFixtureWindow
      ).__BUZZ_E2E_STICK_FIXTURE__?.state();
      return true;
    } catch {
      return false;
    }
  });
}

async function readState(
  page: import("@playwright/test").Page,
): Promise<StickFixtureState> {
  return page.evaluate(() =>
    (
      window as unknown as StickFixtureWindow
    ).__BUZZ_E2E_STICK_FIXTURE__.state(),
  );
}

test("intermediate smooth-scroll events do not unstick the activity feed", async ({
  page,
}) => {
  // Seed enough items that the scroll range is much larger than the
  // 100 px "near bottom" threshold, leaving room to put scrollTop at
  // a mid-animation value with distance > 100.
  await mountFixture(page, 60);

  const initial = await readState(page);
  expect(initial.isNearBottom).toBe(true);
  expect(initial.distanceFromBottom).toBeLessThan(5);
  // The initial scroll-to-bottom in the hook's useEffect uses a raw
  // assignment, so onScroll doesn't fire — lastScrollTopRef is seeded
  // by that effect (or by a synthetic event below). To make the
  // direction comparison meaningful, fire a baseline onScroll at the
  // current bottom so lastScrollTopRef reflects the resting scrollTop.
  await page.evaluate(() => {
    (
      window as unknown as StickFixtureWindow
    ).__BUZZ_E2E_STICK_FIXTURE__.simulateScroll(
      (
        window as unknown as StickFixtureWindow
      ).__BUZZ_E2E_STICK_FIXTURE__.state().scrollTop,
    );
  });

  // Now simulate the exact race the bug reporter sees: a smooth-scroll
  // is animating *toward* the new bottom (scrollTop climbing upward in
  // absolute terms), but content was appended faster than the
  // animation, so distance from bottom is still > 100. The
  // intermediate scroll event must NOT unstick the feed.
  //
  // The deterministic way to fake this without depending on
  // Chromium's smooth-scroll timing: grow the scroll range by pushing
  // a burst, then set scrollTop to a value that is higher than the
  // pre-burst max but still > 100 px from the new bottom, and fire
  // a scroll event.
  await page.evaluate(() => {
    const f = (window as unknown as StickFixtureWindow)
      .__BUZZ_E2E_STICK_FIXTURE__;
    // Add enough items to push the bottom well past the current
    // scrollTop.
    for (let i = 0; i < 30; i += 1) {
      f.push(`burst-${i}`);
    }
  });
  // Read the new layout BEFORE the rAF scroll-to-bottom completes.
  // (The mutation observer schedules a smooth-scroll, but we
  // intercept by firing a synthetic intermediate scroll event
  // ourselves.)
  const beforeIntermediate = await page.evaluate(() => {
    const f = (window as unknown as StickFixtureWindow)
      .__BUZZ_E2E_STICK_FIXTURE__;
    return f.state();
  });
  // Pick a scrollTop that is climbing toward the new bottom but still
  // > 100 px away from it.
  const intermediateScrollTop =
    beforeIntermediate.scrollHeight - beforeIntermediate.clientHeight - 250;
  await page.evaluate((top) => {
    (
      window as unknown as StickFixtureWindow
    ).__BUZZ_E2E_STICK_FIXTURE__.simulateScroll(top);
  }, intermediateScrollTop);

  const midAnimation = await readState(page);
  expect(midAnimation.distanceFromBottom).toBeGreaterThan(100);
  // The actual regression assertion: an intermediate scroll event
  // *toward* the bottom does not unstick the feed. Pre-fix this is
  // `false`; post-fix it stays `true`.
  expect(midAnimation.isNearBottom).toBe(true);

  // Confirm the end-to-end contract: the next content append still
  // schedules a scroll-to-bottom because isNearBottom is sticky.
  await page.evaluate(() => {
    const f = (window as unknown as StickFixtureWindow)
      .__BUZZ_E2E_STICK_FIXTURE__;
    f.push("after-mid-anim-1");
    f.push("after-mid-anim-2");
  });
  await page.waitForTimeout(800);
  const settled = await readState(page);
  expect(settled.itemCount).toBe(60 + 30 + 2);
  expect(settled.distanceFromBottom).toBeLessThan(5);
  expect(settled.isNearBottom).toBe(true);
});

test("user scrolling up still unsticks the activity feed", async ({ page }) => {
  // Direction-awareness guard: the fix must not over-correct and pin
  // the container even when the user genuinely pulls the view up.
  await mountFixture(page, 60);

  await page.evaluate(() => {
    (
      window as unknown as StickFixtureWindow
    ).__BUZZ_E2E_STICK_FIXTURE__.scrollToTop();
  });

  const afterScrollUp = await readState(page);
  expect(afterScrollUp.isNearBottom).toBe(false);
  expect(afterScrollUp.distanceFromBottom).toBeGreaterThan(100);

  // Pushing more items should NOT yank scrollTop back to the bottom
  // — sticky is off because the user moved away.
  await page.evaluate(() => {
    const f = (window as unknown as StickFixtureWindow)
      .__BUZZ_E2E_STICK_FIXTURE__;
    for (let i = 0; i < 5; i += 1) f.push();
  });
  await page.waitForTimeout(400);

  const afterPushes = await readState(page);
  expect(afterPushes.isNearBottom).toBe(false);
  expect(afterPushes.distanceFromBottom).toBeGreaterThan(100);
});
