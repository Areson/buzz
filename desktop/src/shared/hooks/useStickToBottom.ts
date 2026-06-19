import { useCallback, useEffect, useRef } from "react";

/**
 * Keeps a scroll container pinned to the bottom as new content arrives,
 * unless the user has scrolled up. Mirrors the "sticky scroll" pattern
 * from goose's MessageTimeline.
 *
 * Attach `ref` to the scrollable container and `onScroll` as its scroll
 * handler. The hook observes DOM mutations inside the container and
 * auto-scrolls when the user is near the bottom (within 100 px).
 *
 * Scroll calls are batched via `requestAnimationFrame` so rapid streaming
 * updates (e.g. token-by-token SSE) don't cause layout thrashing.
 *
 * `onScroll` is direction-aware: it only unsticks when the user moves
 * the viewport *upward*. Intermediate scroll events emitted by the
 * browser during the smooth scroll-to-bottom animation see scrollTop
 * climbing toward the new bottom, so they leave the sticky bit alone.
 * Without that guard, a smooth-scroll mid-animation would flip
 * `isNearBottom` to false (the visible scrollTop is still well above
 * scrollHeight - clientHeight - 100), and the next content update
 * would refuse to follow — the user-visible symptom is the feed
 * appearing to "jump back a couple lines" as new content pushes the
 * old bottom out of view.
 */
export function useStickToBottom<T extends HTMLElement = HTMLDivElement>() {
  const ref = useRef<T>(null);
  const isNearBottomRef = useRef(true);
  const lastScrollTopRef = useRef(0);

  const onScroll = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const distance = scrollHeight - scrollTop - clientHeight;
    if (distance < 100) {
      // At (or very near) the bottom — always sticky, regardless of
      // direction. Resticks the container once a smooth-scroll
      // animation settles, and handles the initial state.
      isNearBottomRef.current = true;
    } else if (scrollTop < lastScrollTopRef.current) {
      // User pulled the viewport upward. Detach.
      isNearBottomRef.current = false;
    }
    // Otherwise: scrollTop is climbing toward the bottom (smooth-scroll
    // in flight) or unchanged. Leave the sticky bit as-is.
    lastScrollTopRef.current = scrollTop;
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // Start at the bottom; the observer below only reacts to later changes.
    el.scrollTop = el.scrollHeight;
    lastScrollTopRef.current = el.scrollTop;

    let rafId: number | null = null;

    const scrollIfSticky = () => {
      // Coalesce to one scroll per animation frame.
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        if (isNearBottomRef.current && ref.current) {
          ref.current.scrollTo({
            top: ref.current.scrollHeight,
            behavior: "smooth",
          });
        }
      });
    };

    const observer = new MutationObserver(scrollIfSticky);
    observer.observe(el, {
      childList: true,
      subtree: true,
      characterData: true,
    });

    return () => {
      observer.disconnect();
      if (rafId !== null) cancelAnimationFrame(rafId);
    };
  }, []);

  return { ref, onScroll, isNearBottomRef };
}
