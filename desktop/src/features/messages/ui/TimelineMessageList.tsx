import * as React from "react";

import {
  formatDayHeading,
  isSameDay,
} from "@/features/messages/lib/dateFormatters";
import {
  groupTimelineEntries,
  type AnnotatedTimelineEntry,
} from "@/features/messages/lib/groupTimelineEntries";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { CompactMessageRow } from "./CompactMessageRow";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemEventGroupRow } from "./SystemEventGroupRow";
import { SystemMessageRow } from "./SystemMessageRow";

type TimelineMessageListProps = {
  activeReplyTargetId?: string | null;
  channelId?: string | null;
  currentPubkey?: string;
  highlightedMessageId?: string | null;
  messageFooters?: Record<string, React.ReactNode>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
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
};

/** Return the first message's createdAt for a given annotated entry. */
function getEntryLeadTimestamp(entry: AnnotatedTimelineEntry): number {
  if (entry.entryType === "system-event-group") {
    return entry.entries[0].message.createdAt;
  }
  return entry.message.createdAt;
}

export const TimelineMessageList = React.memo(function TimelineMessageList({
  activeReplyTargetId = null,
  channelId,
  currentPubkey,
  highlightedMessageId = null,
  messageFooters,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  onToggleReaction,
  personaLookup,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
}: TimelineMessageListProps) {
  const annotated = React.useMemo(() => {
    const raw = buildMainTimelineEntries(messages);
    return groupTimelineEntries(raw);
  }, [messages]);

  const dayGroups: Array<{
    key: string;
    label: string;
    elements: React.ReactNode[];
  }> = [];
  let currentDayGroup: (typeof dayGroups)[number] | null = null;

  for (let i = 0; i < annotated.length; i++) {
    const entry = annotated[i];
    const leadTimestamp = getEntryLeadTimestamp(entry);

    // Day divider — start a new day group when the day changes
    if (
      !currentDayGroup ||
      (i > 0 &&
        !isSameDay(getEntryLeadTimestamp(annotated[i - 1]), leadTimestamp))
    ) {
      currentDayGroup = {
        key: `day-${leadTimestamp}`,
        label: formatDayHeading(leadTimestamp),
        elements: [],
      };
      dayGroups.push(currentDayGroup);
    }

    // --- System event group (accordion) ---
    if (entry.entryType === "system-event-group") {
      const groupKey = entry.entries.map((e) => e.message.id).join(",");
      currentDayGroup.elements.push(
        <div key={`sys-group-${groupKey}`} className="my-1">
          <SystemEventGroupRow
            entries={entry.entries}
            currentPubkey={currentPubkey}
            onToggleReaction={onToggleReaction}
            personaLookup={personaLookup}
            profiles={profiles}
          />
        </div>,
      );
      continue;
    }

    // --- Single entries ---
    const { message, summary } = entry;

    // --- Single system message (not grouped) ---
    if (message.kind === KIND_SYSTEM_MESSAGE) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup.elements.push(
        <div key={message.id} className="flex flex-col gap-1">
          <SystemMessageRow
            message={message}
            currentPubkey={currentPubkey}
            onToggleReaction={onToggleReaction}
            personaLookup={personaLookup}
            profiles={profiles}
          />
          {footer}
        </div>,
      );
      continue;
    }

    // --- Search highlight state ---
    const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
    const isSearchActive = message.id === searchActiveMessageId;

    // --- Message with thread summary ---
    if (summary && onReply) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup.elements.push(
        <div key={message.id} className="flex flex-col gap-0">
          <MessageRow
            activeReplyTargetId={activeReplyTargetId}
            channelId={channelId}
            highlighted={message.id === highlightedMessageId}
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
            profiles={profiles}
          />
          <MessageThreadSummaryRow
            depth={message.depth}
            message={message}
            onOpenThread={onReply}
            summary={summary}
          />
          {footer}
        </div>,
      );
      continue;
    }

    // --- Compact message (continuation from same author) ---
    if (entry.isGroupContinuation) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup.elements.push(
        <div key={message.id} className="flex flex-col gap-1">
          <CompactMessageRow
            activeReplyTargetId={activeReplyTargetId}
            highlighted={message.id === highlightedMessageId || isSearchActive}
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
            onToggleReaction={onToggleReaction}
            onReply={onReply}
            profiles={profiles}
            searchQuery={isSearchMatch ? searchQuery : undefined}
          />
          {footer}
        </div>,
      );
      continue;
    }

    // --- Full message row ---
    const footer = messageFooters?.[message.id] ?? null;
    currentDayGroup.elements.push(
      <div key={message.id} className="flex flex-col gap-1">
        <MessageRow
          activeReplyTargetId={activeReplyTargetId}
          channelId={channelId}
          highlighted={message.id === highlightedMessageId || isSearchActive}
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
          profiles={profiles}
          searchQuery={isSearchMatch ? searchQuery : undefined}
        />
        {footer}
      </div>,
    );
  }

  return dayGroups.map((group) => (
    <section className="flex flex-col gap-2.5" key={group.key}>
      <DayDivider label={group.label} />
      {group.elements}
    </section>
  ));
});
