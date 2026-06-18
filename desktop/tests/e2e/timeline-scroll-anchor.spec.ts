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
