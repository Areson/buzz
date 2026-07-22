import type * as React from "react";

import type { ScrollTargetAlignment } from "@/features/messages/ui/anchoredScrollTarget";

export type UseAnchoredScrollOptions = {
  /** Scroll container. Owned by the parent so external refs still compose. */
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  /** Inner content element — must wrap every renderable row, including the
   *  sentinel and bottom anchor. Used to schedule layout work on resize. */
  contentRef: React.RefObject<HTMLDivElement | null>;
  /** Resets when changed; lets us drop anchor + scroll state across channels. */
  channelId?: string | null;
  /** Suppresses initial scroll-to-bottom while a skeleton is showing. */
  isLoading: boolean;
  /** Source of truth for the rendered list. Used to detect new-at-bottom
   *  arrivals and to seed/refresh the anchor pre-render. */
  messages: Array<{ id: string }>;
  splitPanelOpen?: boolean;

  /** When set, scroll to this message on mount and on change. */
  targetMessageId?: string | null;
  /** Thread-open anchors use top/bottom; explicit targets default to center. */
  targetAlignment?: ScrollTargetAlignment;
  /** Whether a targeted message should pulse after scrolling to it. */
  highlightTargetMessage?: boolean;
  /** Keeps a targeted message centered until the user deliberately scrolls. */
  pinTargetCentered?: boolean;
  onTargetReached?: (messageId: string) => void;
  virtualCancelBottomIntent?: () => void;
  virtualScrollToMessage?: (
    messageId: string,
    options?: { behavior?: ScrollBehavior },
  ) => boolean;
  /** Imperative virtualizer-owned bottom jump, used only when virtualizer mode is active. */
  virtualScrollToBottom?: (behavior?: ScrollBehavior) => void;
  virtualSettleAtBottom?: () => void;
  /** When active, the virtualizer owns prepend compensation and bottom-state synchronization. */
  virtualizerOwnsPrependAnchoring?: boolean;
  /** Bumps when a virtualized range changes, so pending target/search retries can re-check newly mounted DOM. */
  virtualizerRenderVersion?: number;
};

export type UseAnchoredScrollResult = {
  /** Pass through to the scroll container's `onScroll`. */
  onScroll: (
    event?: Pick<React.UIEvent<HTMLDivElement>, "currentTarget">,
  ) => void;
  /** True when the user is within the bottom threshold. */
  isAtBottom: boolean;
  /** Number of new messages that arrived while the user was not at the bottom. */
  newMessageCount: number;
  /** Message id that should pulse a highlight (target/active-search). */
  highlightedMessageId: string | null;
  /** Imperative: scroll to bottom. */
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  /** Arm a one-shot scroll-to-bottom for the next appended message. */
  scrollToBottomOnNextUpdate: () => void;
  /** Imperative: scroll a specific message into view. */
  scrollToMessage: (
    messageId: string,
    options?: { highlight?: boolean; behavior?: ScrollBehavior },
  ) => boolean;
  /** Sync bottom affordances from a virtualizer-owned scroller. */
  onVirtualizerAtBottomStateChange: (atBottom: boolean) => void;
};
