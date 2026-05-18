import {
  Bot,
  MessageCircle,
  Share2,
  SmilePlus,
  UserMinus,
  UserPlus,
} from "lucide-react";

import type { UserNote } from "@/shared/api/socialTypes";
import type { UserProfileSummary } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type NoteCardProps = {
  note: UserNote;
  profile?: UserProfileSummary | null;
  isAgent?: boolean;
  isOwnNote: boolean;
  isFollowing: boolean;
  onFollow?: (pubkey: string) => void;
  onReact?: (note: UserNote) => void;
  onReply?: (note: UserNote) => void;
  onShare?: (note: UserNote) => void;
  onUnfollow?: (pubkey: string) => void;
};

function formatRelativeTime(unixSeconds: number): string {
  const now = Date.now() / 1_000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3_600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86_400) return `${Math.floor(diff / 3_600)}h`;
  if (diff < 604_800) return `${Math.floor(diff / 86_400)}d`;

  return new Date(unixSeconds * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function NoteCard({
  note,
  profile,
  isAgent,
  isOwnNote,
  isFollowing,
  onFollow,
  onReact,
  onReply,
  onShare,
  onUnfollow,
}: NoteCardProps) {
  const displayName = profile?.displayName ?? `${note.pubkey.slice(0, 8)}...`;
  const avatarUrl = profile?.avatarUrl ?? null;

  return (
    <article className="group flex items-start gap-2.5 rounded-2xl px-1 pb-1 pt-4 transition-colors hover:bg-muted/20 sm:px-2">
      <div className="relative shrink-0">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="!h-9 !w-9 shrink-0"
          displayName={displayName}
        />
        {isAgent ? (
          <Bot className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full bg-background p-0.5 text-muted-foreground" />
        ) : null}
      </div>

      <div className="min-w-0 flex-1 space-y-0">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
          <span className="truncate text-sm font-semibold leading-none tracking-tight">
            {displayName}
          </span>
          {isAgent ? (
            <span className="inline-flex h-4 items-center rounded bg-muted px-1 text-[10px] font-medium text-muted-foreground">
              bot
            </span>
          ) : null}
          {profile?.nip05Handle ? (
            <span className="truncate text-xs text-muted-foreground">
              {profile.nip05Handle}
            </span>
          ) : null}
          <span className="shrink-0 text-xs text-muted-foreground/70">
            {formatRelativeTime(note.createdAt)}
          </span>
          {!isOwnNote ? (
            <div className="ml-auto hidden shrink-0 group-hover:block">
              {isFollowing ? (
                <Button
                  className="h-6 gap-1 px-2 text-xs text-muted-foreground"
                  onClick={() => onUnfollow?.(note.pubkey)}
                  size="sm"
                  variant="ghost"
                >
                  <UserMinus className="h-3 w-3" />
                  Unfollow
                </Button>
              ) : (
                <Button
                  className="h-6 gap-1 px-2 text-xs text-muted-foreground"
                  onClick={() => onFollow?.(note.pubkey)}
                  size="sm"
                  variant="ghost"
                >
                  <UserPlus className="h-3 w-3" />
                  Follow
                </Button>
              )}
            </div>
          ) : null}
        </div>

        <div className="mt-0.5 pb-3 text-sm text-foreground">
          <Markdown content={note.content} tight />
        </div>

        <div className="flex flex-wrap items-center gap-3 text-[11px] font-medium text-muted-foreground">
          <button
            className="inline-flex items-center gap-1 hover:text-foreground"
            onClick={() => onReact?.(note)}
            type="button"
          >
            <SmilePlus className="h-3.5 w-3.5" />
            React
          </button>
          <button
            className="inline-flex items-center gap-1 hover:text-foreground"
            onClick={() => onReply?.(note)}
            type="button"
          >
            <MessageCircle className="h-3.5 w-3.5" />
            Reply
          </button>
          <button
            className="inline-flex items-center gap-1 hover:text-foreground"
            onClick={() => onShare?.(note)}
            type="button"
          >
            <Share2 className="h-3.5 w-3.5" />
            Share
          </button>
          {!isOwnNote ? (
            isFollowing ? (
              <button
                className="hover:text-foreground"
                onClick={() => onUnfollow?.(note.pubkey)}
                type="button"
              >
                Unfollow
              </button>
            ) : (
              <button
                className="hover:text-foreground"
                onClick={() => onFollow?.(note.pubkey)}
                type="button"
              >
                Follow
              </button>
            )
          ) : null}
        </div>
      </div>
    </article>
  );
}
