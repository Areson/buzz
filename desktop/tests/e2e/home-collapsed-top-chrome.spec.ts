import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test.describe("home inbox header collapsed-sidebar chrome clearance", () => {
  test("inbox uses one continuous header backdrop across list and detail panes", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();
    await expect(page.getByTestId("home-inbox-detail")).toBeVisible();

    const inboxBackdrop = await page
      .getByTestId("home-inbox")
      .evaluate((el) => {
        const style = window.getComputedStyle(el, "::before");
        return {
          backdropFilter: style.backdropFilter || style.webkitBackdropFilter,
          backgroundColor: style.backgroundColor,
          height: style.height,
        };
      });
    expect(inboxBackdrop.backdropFilter).toContain("blur");
    expect(inboxBackdrop.backgroundColor).not.toBe("rgba(0, 0, 0, 0)");
    expect(Number.parseFloat(inboxBackdrop.height)).toBeGreaterThan(0);

    const paneHeaderStyles = await page
      .getByTestId("home-inbox")
      .locator("[data-home-inbox-header]")
      .evaluateAll((headers) =>
        headers.map((header) => {
          const style = window.getComputedStyle(header);
          return {
            backdropFilter: style.backdropFilter || style.webkitBackdropFilter,
            backgroundColor: style.backgroundColor,
          };
        }),
      );
    expect(paneHeaderStyles).toHaveLength(2);
    for (const style of paneHeaderStyles) {
      expect(style.backdropFilter).toBe("none");
      expect(style.backgroundColor).toBe("rgba(0, 0, 0, 0)");
    }
  });

  test.use({ viewport: { width: 1280, height: 720 } });

  test("inbox options clear the macOS traffic-light region when sidebar is collapsed", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();

    await page.locator('[data-sidebar="trigger"]').click();

    const inboxOptions = page.getByTestId("inbox-options-trigger");
    await expect(inboxOptions).toBeVisible();
    await expect
      .poll(async () =>
        inboxOptions.evaluate((element) =>
          Math.round(element.getBoundingClientRect().left),
        ),
      )
      .toBeGreaterThanOrEqual(168);

    await waitForAnimations(page);
  });
});
