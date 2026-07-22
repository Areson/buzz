import * as React from "react";

export function useThreadOpenScrollTarget(
  threadHeadId: string | null,
  firstUnreadReplyId: string | null,
  isFetchingReplies: boolean,
) {
  const [scrollTargetId, setScrollTargetId] = React.useState<string | null>(
    null,
  );
  const latchedForHeadRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!threadHeadId) {
      latchedForHeadRef.current = null;
      return;
    }
    if (latchedForHeadRef.current === threadHeadId) {
      return;
    }

    // Explicit route/deep-link targets take precedence over the unread anchor.
    if (scrollTargetId !== null) {
      latchedForHeadRef.current = threadHeadId;
      return;
    }

    // A cached thread has data while its fresh subtree is still loading. Wait
    // for that refresh so a stale all-read snapshot cannot latch too early.
    if (isFetchingReplies) {
      return;
    }

    latchedForHeadRef.current = threadHeadId;
    if (firstUnreadReplyId) {
      setScrollTargetId(firstUnreadReplyId);
    }
  }, [firstUnreadReplyId, isFetchingReplies, scrollTargetId, threadHeadId]);

  const clearScrollTarget = React.useCallback(() => {
    setScrollTargetId(null);
  }, []);

  return [scrollTargetId, setScrollTargetId, clearScrollTarget] as const;
}
