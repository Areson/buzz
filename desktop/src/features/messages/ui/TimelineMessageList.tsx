import * as React from "react";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import type { TimelineVirtualItem } from "@/features/messages/lib/timelineSnapshot";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
} from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import type { ChatVirtualizer } from "./useChatScrollVirtualizer";
import { DayDivider } from "./DayDivider";
import type { ExpandedDiff } from "./DiffMessageExpanded";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";
import { UnreadDivider } from "./UnreadDivider";

type TimelineMessageListProps = {
  agentPubkeys?: ReadonlySet<string>;
  /**
   * Key of the day group whose heading is currently pinned in the overlay
   * (option B sticky-overlay). The matching inline day divider is hidden so the
   * label never doubles — the overlay is its stand-in while it would be pinned.
   */
  activeDayKey?: string | null;
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  currentPubkey?: string;
  /** Event id of the oldest unread top-level message; renders a "New" divider above it. */
  firstUnreadMessageId?: string | null;
  followThreadById?: (rootId: string) => void;
  highlightedMessageId?: string | null;
  isFollowingThreadById?: (rootId: string) => boolean;
  messageFooters?: Record<string, React.ReactNode>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  /**
   * Open the expanded diff modal. Forwarded to each `MessageRow`; the surface
   * owns the modal so it survives a row scrolling out of the virtual window.
   */
  onExpandDiff?: (diff: ExpandedDiff) => void;
  isSendingVideoReviewComment?: boolean;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
  /** Per-thread unread counts keyed by thread root id. */
  threadUnreadCounts?: ReadonlyMap<string, number>;
  /**
   * The virtualizer that owns this surface's scroll. Built by
   * `useChatScrollVirtualizer` at the timeline level so the single scroll owner
   * also owns measurement; this component is a pure renderer of its rows.
   */
  virtualizer: ChatVirtualizer;
  /** The flat virtual-item list the virtualizer's `count` mirrors. */
  items: TimelineVirtualItem[];
  /**
   * Top padding (px) that bottom-aligns a channel shorter than the viewport.
   * Always `0` once content fills the viewport. Applied to the row spacer so it
   * pushes the absolute rows down without entering the measured total.
   */
  topPad: number;
  /** Ref the hook reads to measure chrome around the spacer for `topPad`. */
  spacerRef: React.RefObject<HTMLElement | null>;
};

export const TimelineMessageList = React.memo(function TimelineMessageList({
  agentPubkeys,
  activeDayKey = null,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  followThreadById,
  highlightedMessageId = null,
  isFollowingThreadById,
  messageFooters,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  onExpandDiff,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  threadUnreadCounts,
  unfollowThreadById,
  virtualizer,
  items,
  topPad,
  spacerRef,
}: TimelineMessageListProps) {
  const reviewCommentsByRootId = React.useMemo(
    () => buildVideoReviewCommentsByRootId(messages),
    [messages],
  );
  // Contexts are memoized per message id so MessageRow/Markdown memo
  // comparisons hold across unrelated timeline re-renders (typing
  // indicators, presence updates) — a fresh context object per render would
  // defeat the memo and re-render every video message on every pass.
  const videoReviewContextById = React.useMemo(() => {
    const contexts = new Map<
      string,
      NonNullable<ReturnType<typeof buildVideoReviewContextForMessage>>
    >();
    for (const message of messages) {
      const comments = reviewCommentsByRootId.get(message.id) ?? [];
      const context = buildVideoReviewContextForMessage({
        channelId,
        channelName,
        channelType,
        comments,
        isSendingVideoReviewComment,
        message,
        onSendVideoReviewComment,
        onToggleReaction,
        profiles,
      });
      if (context) {
        contexts.set(message.id, context);
      }
    }
    return contexts;
  }, [
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messages,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
    reviewCommentsByRootId,
  ]);

  const renderItem = (item: TimelineVirtualItem): React.ReactNode => {
    switch (item.kind) {
      case "day":
        // The day separator scrolls inline with the rows (it is a normal
        // virtual item, not sticky — sticky cannot work on an absolute row).
        // The thin connector line that the legacy day-`<section>` drew behind
        // the centered pill is re-homed onto this row so the divider keeps its
        // rule-behind-label look without the group wrapper.
        //
        // When this is the active (pinned) day, the overlay header above is its
        // stand-in, so we hide the inline label with `invisible` — the row keeps
        // its measured height (no scroll jump) but never shows a second copy.
        return (
          <div
            className={cn(
              "relative before:absolute before:inset-x-0 before:top-[15px] before:h-px before:bg-border/35 before:content-['']",
              item.key === activeDayKey && "invisible",
            )}
          >
            <DayDivider label={formatDayHeading(item.headingTimestamp)} />
          </div>
        );
      case "unread":
        return <UnreadDivider />;
      case "system": {
        const footer = messageFooters?.[item.message.id] ?? null;
        return (
          <div className="flex flex-col gap-1">
            <SystemMessageRow
              message={item.message}
              currentPubkey={currentPubkey}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
            />
            {footer}
          </div>
        );
      }
      case "message": {
        const { message, summary } = item;
        const footer = messageFooters?.[message.id] ?? null;

        if (summary && onReply) {
          const isHighlighted = message.id === highlightedMessageId;
          return (
            <div
              className={cn(
                "group/message relative mx-1 flex flex-col gap-0 rounded-2xl px-0 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
                isHighlighted &&
                  "-mx-4 px-4 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-6",
              )}
            >
              <MessageRow
                agentPubkeys={agentPubkeys}
                channelId={channelId}
                highlighted={false}
                hoverBackground={false}
                isFollowingThread={
                  isFollowingThreadById
                    ? isFollowingThreadById(message.id)
                    : undefined
                }
                message={message}
                onDelete={
                  onDelete && currentPubkey && message.pubkey === currentPubkey
                    ? onDelete
                    : undefined
                }
                onEdit={
                  onEdit && currentPubkey && message.pubkey === currentPubkey
                    ? onEdit
                    : undefined
                }
                onFollowThread={
                  followThreadById
                    ? () => followThreadById(message.id)
                    : undefined
                }
                onMarkUnread={onMarkUnread}
                onToggleReaction={onToggleReaction}
                onReply={onReply}
                onExpandDiff={onExpandDiff}
                onUnfollowThread={
                  unfollowThreadById
                    ? () => unfollowThreadById(message.id)
                    : undefined
                }
                profiles={profiles}
                showDepthGuides={false}
                videoReviewContext={videoReviewContextById.get(message.id)}
              />
              <MessageThreadSummaryRow
                depth={message.depth}
                message={message}
                onOpenThread={onReply}
                showDepthGuides={false}
                summary={summary}
                unreadCount={threadUnreadCounts?.get(message.id)}
              />
              {footer}
            </div>
          );
        }

        const isSearchMatch =
          searchMatchingMessageIds?.has(message.id) ?? false;
        const isSearchActive = message.id === searchActiveMessageId;
        return (
          <div className="flex flex-col gap-1">
            <MessageRow
              agentPubkeys={agentPubkeys}
              channelId={channelId}
              highlighted={
                message.id === highlightedMessageId || isSearchActive
              }
              message={message}
              onDelete={
                onDelete && currentPubkey && message.pubkey === currentPubkey
                  ? onDelete
                  : undefined
              }
              onEdit={
                onEdit && currentPubkey && message.pubkey === currentPubkey
                  ? onEdit
                  : undefined
              }
              onMarkUnread={onMarkUnread}
              onToggleReaction={onToggleReaction}
              onReply={onReply}
              onExpandDiff={onExpandDiff}
              profiles={profiles}
              searchQuery={isSearchMatch ? searchQuery : undefined}
              showDepthGuides={false}
              videoReviewContext={videoReviewContextById.get(message.id)}
            />
            {footer}
          </div>
        );
      }
    }
  };

  // Bottom-align a short channel by shifting the absolute rows down by
  // `topPad`. The pad CANNOT be a `paddingTop` on the spacer: an
  // absolutely-positioned child resolves `top:0` against the padding box's
  // inner edge, so padding inflates the box without moving the rows — they
  // would pin to the top with dead space below. Folding `topPad` into each
  // row's `translateY` (and into the spacer height for the matching scroll
  // range) is what actually pushes the rows to the floor.
  return (
    <div
      className="relative w-full"
      ref={spacerRef as React.Ref<HTMLDivElement>}
      style={{ height: `${virtualizer.getTotalSize() + topPad}px` }}
    >
      {virtualizer.getVirtualItems().map((virtualRow) => (
        <div
          data-index={virtualRow.index}
          key={virtualRow.key}
          ref={virtualizer.measureElement}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            width: "100%",
            transform: `translateY(${virtualRow.start + topPad}px)`,
          }}
        >
          {renderItem(items[virtualRow.index])}
        </div>
      ))}
    </div>
  );
});
