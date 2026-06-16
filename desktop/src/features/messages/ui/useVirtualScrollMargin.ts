import * as React from "react";

/**
 * Measure the virtualized list's offset from the top of the scroll container's
 * scrollable content, to feed `useVirtualizer({ scrollMargin })`.
 *
 * The main timeline's scroll container holds content ABOVE the virtualized list
 * inside the SAME scrollable element: the pagination sentinel, the
 * "load older" spinner, and the channel/DM intro banner. `@tanstack/react-virtual`
 * positions items at `paddingStart + scrollMargin`, so without this the
 * virtualizer assumes row 0 sits at scrollTop 0 — but it's actually painted
 * `scrollMargin` px lower. That mismatch is what makes freshly-loaded rows
 * sandwich into the header/list seam and the viewport drift while rows fill.
 *
 * We re-measure whenever the above-content can change height (intro mount/
 * unmount, spinner toggle) AND via a ResizeObserver on the scroll container, so
 * the margin stays correct as content streams in.
 */
export function useVirtualScrollMargin(
  scrollContainerRef: React.RefObject<HTMLDivElement | null>,
  listOuterRef: React.RefObject<HTMLDivElement | null>,
  // Re-measure triggers — values whose change can shift the list's offset.
  deps: ReadonlyArray<unknown>,
): number {
  const [scrollMargin, setScrollMargin] = React.useState(0);

  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    const list = listOuterRef.current;
    if (!container || !list) {
      return;
    }

    const measure = () => {
      const c = scrollContainerRef.current;
      const l = listOuterRef.current;
      if (!c || !l) {
        return;
      }
      // Offset of the list within the scroll container's scrollable content:
      // distance from the container's content top to the list's top.
      const next = Math.round(
        l.getBoundingClientRect().top -
          c.getBoundingClientRect().top +
          c.scrollTop,
      );
      setScrollMargin((current) => (current === next ? current : next));
    };

    measure();

    if (typeof ResizeObserver === "undefined") {
      return;
    }
    // The above-content lives inside the container; observe the container so a
    // height change in the sentinel/spinner/intro re-measures the margin.
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
    // deps drive intentional re-measures (intro/spinner/list visibility).
  }, [scrollContainerRef, listOuterRef, ...deps]);

  return scrollMargin;
}
