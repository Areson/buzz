import * as React from "react";
import { AnimatePresence, motion } from "motion/react";
import { ChevronRight } from "lucide-react";

import type { TimelineMessage } from "@/features/messages/types";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import {
  type UserProfileLookup,
  resolveUserLabel,
} from "@/features/profile/lib/identity";
import {
  parseSystemMessagePayload,
  type SystemMessagePayload,
} from "@/features/messages/lib/describeSystemEvent";

import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { SystemMessageRow } from "./SystemMessageRow";

/** Max avatars to show in the stacked ingress before showing +N. */
const MAX_STACKED_AVATARS = 4;

// ---------------------------------------------------------------------------
// Summary builder
// ---------------------------------------------------------------------------

/** Resolve an actor pubkey to a display name. */
function resolveActorName(
  pubkey: string | undefined,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string {
  if (!pubkey) return "Someone";
  return resolveUserLabel({ pubkey, currentPubkey, profiles });
}

/** Describe a single action type + count as a fragment (no actor prefix). */
function describeAction(type: string, count: number): string | null {
  switch (type) {
    case "member_joined_self":
      return count === 1
        ? "joined the channel"
        : `joined the channel (×${count})`;
    case "member_joined":
      return `added ${count} member${count === 1 ? "" : "s"}`;
    case "member_left":
      return count === 1 ? "left the channel" : `left the channel (×${count})`;
    case "member_removed":
      return `removed ${count} member${count === 1 ? "" : "s"}`;
    case "topic_changed":
      return `changed the topic`;
    case "purpose_changed":
      return `changed the purpose`;
    case "channel_created":
      return "created this channel";
    default:
      return null;
  }
}

/**
 * Build a summary grouped by actor.
 *
 * Single actor, one action:  "tho added 5 members"
 * Single actor, mixed:       "tho added 3 members, removed 2 members"
 * Multi actor (semicolons):  "tho added 5 members; wes added 2 members"
 * Self-join:                 "tho joined the channel"
 */
function buildSummary(
  payloads: SystemMessagePayload[],
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  _personaLookup?: Map<string, string>,
): string {
  // Group counts by actor → type.
  const actorTypes = new Map<string, Map<string, number>>();
  // Preserve insertion order of actors.
  const actorOrder: string[] = [];

  for (const p of payloads) {
    const actorKey = p.actor ?? "__unknown__";
    let typeMap = actorTypes.get(actorKey);
    if (!typeMap) {
      typeMap = new Map();
      actorTypes.set(actorKey, typeMap);
      actorOrder.push(actorKey);
    }
    // Distinguish self-joins ("joined") from adds ("added N members").
    const type =
      p.type === "member_joined" && p.actor === p.target
        ? "member_joined_self"
        : p.type;
    typeMap.set(type, (typeMap.get(type) ?? 0) + 1);
  }

  const clauses: string[] = [];

  for (const actorKey of actorOrder) {
    const name = resolveActorName(
      actorKey === "__unknown__" ? undefined : actorKey,
      currentPubkey,
      profiles,
    );
    const typeMap = actorTypes.get(actorKey);
    if (!typeMap) continue;
    const actions: string[] = [];

    for (const [type, count] of typeMap) {
      const desc = describeAction(type, count);
      if (desc) actions.push(desc);
    }

    if (actions.length === 0) continue;

    // First action gets the actor name; subsequent actions for the same actor
    // omit the name to read naturally: "tho added 3 members, removed 2 members"
    clauses.push(`${name} ${actions.join(", ")}`);
  }

  return clauses.length > 0
    ? clauses.join("; ")
    : `${payloads.length} system event${payloads.length === 1 ? "" : "s"}`;
}

// ---------------------------------------------------------------------------
// Avatar helpers
// ---------------------------------------------------------------------------

/**
 * Extract unique pubkeys to display as stacked avatars.
 * For add/remove: show targets. For topic/purpose/channel: show actors.
 */
function extractAvatarPubkeys(payloads: SystemMessagePayload[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];

  for (const p of payloads) {
    const key =
      p.type === "member_joined" ||
      p.type === "member_removed" ||
      p.type === "member_left"
        ? // For member_left the actor IS the target (they left themselves)
          p.type === "member_left"
          ? p.actor
          : p.target
        : p.actor;

    if (key && !seen.has(key)) {
      seen.add(key);
      result.push(key);
    }
  }

  return result;
}

function resolveAvatarUrl(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string | null {
  if (!pubkey || !profiles) return null;
  return profiles[pubkey.toLowerCase()]?.avatarUrl ?? null;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SystemEventGroupRow({
  entries,
  currentPubkey,
  onToggleReaction,
  personaLookup,
  profiles,
}: {
  entries: MainTimelineEntry[];
  currentPubkey?: string;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
}) {
  const [expanded, setExpanded] = React.useState(false);

  const payloads = React.useMemo(
    () =>
      entries
        .map((e) => parseSystemMessagePayload(e.message.body))
        .filter((p): p is SystemMessagePayload => p !== null),
    [entries],
  );

  const summary = React.useMemo(
    () => buildSummary(payloads, currentPubkey, profiles, personaLookup),
    [payloads, currentPubkey, profiles, personaLookup],
  );

  const avatarPubkeys = React.useMemo(
    () => extractAvatarPubkeys(payloads),
    [payloads],
  );

  const visibleAvatars = avatarPubkeys.slice(0, MAX_STACKED_AVATARS);
  const overflowCount = avatarPubkeys.length - visibleAvatars.length;

  const groupId = React.useId();
  const panelId = `${groupId}-panel`;

  return (
    <div data-testid="system-event-group">
      {/* Collapsed summary row — centered pill */}
      <div className="flex justify-center py-1 px-4">
        <button
          aria-controls={panelId}
          aria-expanded={expanded}
          className="flex items-center gap-1.5 rounded-full border border-border/50 pl-1 pr-2.5 py-1 transition-colors hover:bg-muted/30"
          data-testid="system-event-group-toggle"
          onClick={() => setExpanded((prev) => !prev)}
          type="button"
        >
          {/* Stacked avatars */}
          <div className="flex shrink-0 items-center">
            {visibleAvatars.map((pubkey, index) => (
              <div
                key={pubkey}
                className={index > 0 ? "-ml-1.5" : ""}
                style={{ zIndex: 10 - index }}
              >
                <UserAvatar
                  avatarUrl={resolveAvatarUrl(pubkey, profiles)}
                  className="!h-5 !w-5 rounded-full border border-background text-[7px]"
                  displayName={resolveActorName(
                    pubkey,
                    currentPubkey,
                    profiles,
                  )}
                  size="xs"
                />
              </div>
            ))}
            {overflowCount > 0 ? (
              <div
                className="-ml-1.5 flex h-4 w-4 items-center justify-center rounded-full border border-background bg-muted text-[7px] font-medium text-muted-foreground"
                style={{ zIndex: 10 - visibleAvatars.length }}
              >
                +{overflowCount}
              </div>
            ) : null}
          </div>
          <span className="text-xs text-muted-foreground/70">{summary}</span>
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform duration-150",
              expanded && "rotate-90",
            )}
          />
        </button>
      </div>

      {/* Expanded children — inline flex-wrapped chips with staggered animation */}
      <AnimatePresence>
        {expanded ? (
          <motion.section
            className="flex flex-wrap justify-center gap-1 px-4 pb-1"
            data-testid="system-event-group-children"
            id={panelId}
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.15, ease: "easeOut" }}
          >
            {entries.map((entry, index) => (
              <motion.div
                key={entry.message.id}
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{
                  duration: 0.12,
                  delay: index * 0.03,
                  ease: "easeOut",
                }}
              >
                <SystemMessageRow
                  message={entry.message}
                  currentPubkey={currentPubkey}
                  onToggleReaction={onToggleReaction}
                  personaLookup={personaLookup}
                  profiles={profiles}
                  compact
                />
              </motion.div>
            ))}
          </motion.section>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
