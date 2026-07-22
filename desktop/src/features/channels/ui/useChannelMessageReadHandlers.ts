import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";

type MarkMessageReadState = (messageId: string) => void;

export function useChannelMessageReadHandlers(
  markMessageRead: MarkMessageReadState,
  markMessageUnread: MarkMessageReadState,
) {
  const handleMessageMarkRead = React.useCallback(
    (message: TimelineMessage) => markMessageRead(message.id),
    [markMessageRead],
  );
  const handleMessageMarkUnread = React.useCallback(
    (message: TimelineMessage) => markMessageUnread(message.id),
    [markMessageUnread],
  );

  return { handleMessageMarkRead, handleMessageMarkUnread };
}
