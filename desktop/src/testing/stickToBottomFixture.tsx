import * as React from "react";
import { createRoot, type Root } from "react-dom/client";

import { useStickToBottom } from "@/shared/hooks/useStickToBottom";

/**
 * E2E test fixture: mounts a scroll container that uses the real
 * `useStickToBottom` hook so a Playwright spec can drive it through the
 * exact code path the activity feed uses. Lives under `src/testing/` so
 * the production bundle ignores it unless the E2E bridge boots.
 */

const HOST_ID = "buzz-e2e-stick-fixture";

type FixtureHandle = {
  push: (text?: string) => number;
  scrollToBottom: () => void;
  scrollToTop: () => void;
  /**
   * Manually set scrollTop and dispatch a `scroll` event so the
   * React-attached `onScroll` handler observes it. Lets specs
   * deterministically simulate a mid-animation intermediate scroll
   * event without depending on Chromium's `behavior: "smooth"`
   * timing in a tiny test container.
   */
  simulateScroll: (scrollTop: number) => void;
  state: () => {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
    isNearBottom: boolean;
    distanceFromBottom: number;
    itemCount: number;
  };
  unmount: () => void;
};

declare global {
  interface Window {
    __BUZZ_E2E_MOUNT_STICK_FIXTURE__?: (options?: {
      seedItems?: number;
    }) => FixtureHandle;
  }
}

function StickToBottomFixture({
  initialItems,
  registerHandle,
}: {
  initialItems: number;
  registerHandle: (handle: Omit<FixtureHandle, "unmount">) => void;
}) {
  const [items, setItems] = React.useState<string[]>(() =>
    Array.from({ length: initialItems }, (_, i) => `seed-${i}`),
  );
  const { ref, onScroll, isNearBottomRef } = useStickToBottom<HTMLDivElement>();
  const seqRef = React.useRef(initialItems);

  React.useEffect(() => {
    registerHandle({
      push: (text?: string) => {
        seqRef.current += 1;
        const label = text ?? `item-${seqRef.current}`;
        setItems((prev) => [...prev, label]);
        return seqRef.current;
      },
      scrollToBottom: () => {
        const el = ref.current;
        if (!el) return;
        el.scrollTop = el.scrollHeight;
        // Manually fire onScroll so the hook updates its internal state
        // — synthetic scrollTop changes don't dispatch a scroll event.
        onScroll();
      },
      scrollToTop: () => {
        const el = ref.current;
        if (!el) return;
        el.scrollTop = 0;
        onScroll();
      },
      simulateScroll: (scrollTop: number) => {
        const el = ref.current;
        if (!el) return;
        el.scrollTop = scrollTop;
        // Dispatch a real scroll event so React's onScroll fires.
        el.dispatchEvent(new Event("scroll", { bubbles: true }));
      },
      state: () => {
        const el = ref.current;
        if (!el) {
          return {
            scrollTop: 0,
            scrollHeight: 0,
            clientHeight: 0,
            isNearBottom: isNearBottomRef.current,
            distanceFromBottom: 0,
            itemCount: 0,
          };
        }
        return {
          scrollTop: el.scrollTop,
          scrollHeight: el.scrollHeight,
          clientHeight: el.clientHeight,
          isNearBottom: isNearBottomRef.current,
          distanceFromBottom: el.scrollHeight - el.scrollTop - el.clientHeight,
          itemCount: el.querySelectorAll("[data-stick-item]").length,
        };
      },
    });
  }, [isNearBottomRef, onScroll, ref, registerHandle]);

  return (
    <div
      data-testid="stick-to-bottom-fixture"
      onScroll={onScroll}
      ref={ref}
      style={{
        height: "240px",
        width: "320px",
        overflowY: "auto",
        border: "1px solid #444",
        fontFamily: "monospace",
        fontSize: "14px",
        lineHeight: "20px",
      }}
    >
      {items.map((item) => (
        <div data-stick-item key={item} style={{ padding: "0 8px" }}>
          {item}
        </div>
      ))}
    </div>
  );
}

let activeRoot: Root | null = null;
let activeHost: HTMLDivElement | null = null;

export function installStickFixtureBridge() {
  if (window.__BUZZ_E2E_MOUNT_STICK_FIXTURE__) return;

  window.__BUZZ_E2E_MOUNT_STICK_FIXTURE__ = (options) => {
    if (activeRoot) {
      activeRoot.unmount();
      activeRoot = null;
    }
    if (activeHost) {
      activeHost.remove();
      activeHost = null;
    }

    const host = document.createElement("div");
    host.id = HOST_ID;
    host.style.position = "fixed";
    host.style.top = "0";
    host.style.left = "0";
    host.style.zIndex = "2147483647";
    host.style.background = "white";
    host.style.color = "black";
    document.body.appendChild(host);

    const root = createRoot(host);
    activeRoot = root;
    activeHost = host;

    let resolveHandle: (handle: Omit<FixtureHandle, "unmount">) => void;
    const handlePromise = new Promise<Omit<FixtureHandle, "unmount">>(
      (resolve) => {
        resolveHandle = resolve;
      },
    );

    root.render(
      <StickToBottomFixture
        initialItems={options?.seedItems ?? 60}
        registerHandle={(handle) => resolveHandle(handle)}
      />,
    );

    // The bridge contract is synchronous — but we need React to have
    // rendered and registered the handle before the caller proceeds.
    // Spin the microtask queue once; if the handle isn't ready, fall
    // back to a stub that retries each call. In practice React 19's
    // createRoot + initial render registers within a microtask, so the
    // promise is usually already resolved by the time the user reads
    // the returned methods via await page.evaluateHandle.
    let registered: Omit<FixtureHandle, "unmount"> | null = null;
    void handlePromise.then((h) => {
      registered = h;
    });

    const ensure = () => {
      if (!registered) {
        throw new Error(
          "stick-to-bottom fixture not yet mounted — await an animation frame first",
        );
      }
      return registered;
    };

    return {
      push: (text) => ensure().push(text),
      scrollToBottom: () => ensure().scrollToBottom(),
      scrollToTop: () => ensure().scrollToTop(),
      simulateScroll: (top) => ensure().simulateScroll(top),
      state: () => ensure().state(),
      unmount: () => {
        root.unmount();
        host.remove();
        activeRoot = null;
        activeHost = null;
        delete (window as Window).__BUZZ_E2E_MOUNT_STICK_FIXTURE__;
      },
    };
  };
}
