import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Scroll-anchoring guards for the virtualized main timeline (PR: virtualize
// timeline). The channel-jump bug was the timeline yanking the viewport when
// the message set changed. These assert geometry via getBoundingClientRect,
// not scrollTop — the bug surfaced as the anchored row visibly jumping even
// while scrollTop looked plausible, so position-on-screen is the real contract.

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

// A short channel (#general seeds four backdated messages, far below a full
// viewport) must bottom-align: the rows sit against the bottom of the scroll
// area with a top pad above them, exactly like the legacy spacer. If topPad
// were dropped, the rows would float at the top with empty space below.
test("short channel bottom-aligns its messages against the viewport floor", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  const rows = page.getByTestId("message-row");
  await expect(rows.first()).toBeVisible();

  // Far fewer rows than fill the viewport — the precondition for bottom-align.
  const metrics = await timeline.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(metrics.scrollHeight).toBeLessThanOrEqual(metrics.clientHeight + 1);

  // The content bottom-aligns against the scroll container's content-box floor
  // (the inner edge above the composer-reserved bottom padding), and the first
  // row is pushed well below the top by the pad above it. "Last row" is the
  // lowest of any rendered row kind — #general's final seed is a system join
  // row, which sits below the last message-row, so anchoring on message-row
  // alone would misread the floor.
  const geometry = await timeline.evaluate((element) => {
    const style = getComputedStyle(element);
    const paddingBottom = Number.parseFloat(style.paddingBottom);
    const timelineRect = element.getBoundingClientRect();
    const contentRows = Array.from(
      element.querySelectorAll(
        '[data-testid="message-row"], [data-testid="system-message-row"]',
      ),
    ).map((row) => row.getBoundingClientRect());
    const firstTop = contentRows[0]?.top ?? Number.NaN;
    const lastBottom = Math.max(...contentRows.map((rect) => rect.bottom));
    return {
      timelineTop: timelineRect.top,
      contentFloor: timelineRect.bottom - paddingBottom,
      firstTop,
      lastBottom,
    };
  });

  // Bottom-aligned: the last row ends within a small margin of the content
  // floor (one row gap of slack).
  expect(geometry.contentFloor - geometry.lastBottom).toBeLessThanOrEqual(24);
  // Padded from the top: the first row is pushed down, not floating at the top.
  expect(geometry.firstTop - geometry.timelineTop).toBeGreaterThan(96);
});

// The channel-jump bug: loading older messages prepended rows above the
// viewport while an end-follow re-pin loop fought the user's scroll, freezing
// the rendered window on the newest messages — the user could not scroll back
// through history at all, and any anchored row was yanked off-screen. #load-older
// seeds 260 messages, more than the 200 initial-history limit, so scrolling up
// pages in a real older batch (a genuine prepend below index 60). The contract
// is geometric: a row the user is reading must hold its on-screen position as
// the window scrolls and the older page lands. We assert its
// getBoundingClientRect().top, not scrollTop — the bug moved the row even when
// scrollTop looked plausible.
test("loading older messages holds the anchored row's screen position", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-load-older").click();
  await expect(page.getByTestId("chat-title")).toHaveText("load-older");

  const timeline = page.getByTestId("message-timeline");
  const rows = page.getByTestId("message-row");
  await expect(rows.first()).toBeVisible();

  // Anchor on index 100: it is inside the newest-200 initial load (oldest is
  // index 60), so it is loaded from channel-open, and it is far enough from the
  // bottom that reaching it requires real scrollback through the window. Under
  // the freeze the rendered window never left the newest ~11 rows, so index 100
  // never mounted — the anchor is absent and the test fails at the first probe.
  const ANCHOR = "mock-load-older-100";

  // Screen position (top relative to the scroll container) of the anchor row,
  // or null when it is not mounted. getBoundingClientRect is the user-visible
  // geometry the bug disturbed; scrollTop is not.
  const anchorTop = () =>
    timeline.evaluate((element, id) => {
      const row = element.querySelector(`[data-message-id="${id}"]`);
      if (!row) {
        return null;
      }
      return (
        row.getBoundingClientRect().top - element.getBoundingClientRect().top
      );
    }, ANCHOR);

  // A real wheel (not a synthetic scrollTop assignment) is required: it drives
  // both the virtualizer and the top-sentinel IntersectionObserver that arms
  // load-older. Scroll up in bounded steps until the anchor row settles into a
  // stable on-screen position near the viewport top. Each step also pages in
  // the older batch as the sentinel enters its 200px margin, so by the time the
  // anchor is parked the prepend has already landed beneath it.
  await timeline.hover();
  let before: number | null = null;
  for (let i = 0; i < 120; i++) {
    const top = await anchorTop();
    // Park once the anchor is mounted and sitting in the upper region of the
    // viewport — the position a reader would hold while paging older history.
    if (top !== null && top >= 0 && top <= 200) {
      before = top;
      break;
    }
    await page.mouse.wheel(0, -120);
    await page.waitForTimeout(40);
  }
  // Under the freeze the window stays pinned to the newest rows, so index 100
  // never mounts and `before` stays null. Reaching a real on-screen position is
  // itself proof the window tracked the scrollback.
  expect(before).not.toBeNull();

  // Let any pending prepend settle, then confirm the anchor held its place. The
  // freeze regression snapped the viewport, moving the row hundreds of pixels;
  // a correct end-anchor reconcile keeps it within a hair of where it was.
  await page.waitForTimeout(200);
  const after = await anchorTop();
  expect(after).not.toBeNull();
  expect(Math.abs((after as number) - (before as number))).toBeLessThanOrEqual(
    8,
  );
});
