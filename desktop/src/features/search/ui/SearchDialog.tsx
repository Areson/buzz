import * as React from "react";
import {
  LoaderCircle,
  MessagesSquare,
  Search,
  X,
  type LucideIcon,
} from "lucide-react";

import {
  useUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { Channel, SearchHit } from "@/shared/api/types";
import {
  ChannelResultBody,
  MessageResultBody,
  resultIcon,
  resultKey,
  resultTestId,
  SearchResultShell,
  type SearchResult,
  UserResultAvatar,
  UserResultBody,
} from "@/features/search/ui/SearchResultItem";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";

const MIN_QUERY_LENGTH = 2;

function SearchState({
  icon: Icon,
  title,
  description,
}: {
  icon: LucideIcon;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
        <Icon className="h-5 w-5" />
      </div>
      <p className="mt-4 text-base font-semibold tracking-tight">{title}</p>
      <p className="mt-2 max-w-md text-sm text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

function SearchLoadingState() {
  return (
    <div className="space-y-3 px-3 py-3" data-testid="search-loading">
      {["first", "second", "third"].map((row) => (
        <div
          className="rounded-2xl border border-border/80 bg-card/60 p-4"
          key={row}
        >
          <Skeleton className="h-4 w-32" />
          <Skeleton className="mt-3 h-4 w-full" />
          <Skeleton className="mt-2 h-4 w-3/4" />
        </div>
      ))}
    </div>
  );
}

type SearchDialogProps = {
  channels: Channel[];
  currentPubkey?: string;
  initialQuery?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
  onOpenUser: (pubkey: string) => void;
};

type SearchResultSection = {
  count: number;
  results: SearchResult[];
  title: string;
};

export function SearchDialog({
  channels,
  currentPubkey,
  initialQuery = "",
  open,
  onOpenChange,
  onOpenChannel,
  onOpenResult,
  onOpenUser,
}: SearchDialogProps) {
  const [query, setQuery] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  const searchQuery = useSearchMessagesQuery(debouncedQuery, {
    enabled: open,
    limit: 12,
  });
  const usersQuery = useUserSearchQuery(debouncedQuery, {
    enabled: open && debouncedQuery.length >= MIN_QUERY_LENGTH,
    limit: 6,
  });

  const messageResults = searchQuery.data?.hits ?? [];
  const userResults = React.useMemo(
    () =>
      (usersQuery.data ?? []).filter(
        (user) => user.pubkey.toLowerCase() !== currentPubkey?.toLowerCase(),
      ),
    [currentPubkey, usersQuery.data],
  );
  const channelResults = React.useMemo(() => {
    if (debouncedQuery.length < MIN_QUERY_LENGTH) {
      return [];
    }

    const normalizedQuery = debouncedQuery.toLowerCase();

    return channels
      .filter(
        (channel) =>
          channel.channelType !== "dm" &&
          (channel.archivedAt
            ? channel.isMember
            : channel.visibility === "open" || channel.isMember) &&
          (channel.name.toLowerCase().includes(normalizedQuery) ||
            channel.description.toLowerCase().includes(normalizedQuery)),
      )
      .sort((a, b) => {
        const aNameMatches = a.name.toLowerCase().includes(normalizedQuery);
        const bNameMatches = b.name.toLowerCase().includes(normalizedQuery);

        if (aNameMatches !== bNameMatches) {
          return aNameMatches ? -1 : 1;
        }

        return a.name.localeCompare(b.name);
      })
      .slice(0, 5);
  }, [channels, debouncedQuery]);
  const sections = React.useMemo<SearchResultSection[]>(
    () =>
      [
        {
          count: userResults.length,
          results: userResults.map((user) => ({
            kind: "user" as const,
            user,
          })),
          title: "People",
        },
        {
          count: channelResults.length,
          results: channelResults.map((channel) => ({
            kind: "channel" as const,
            channel,
          })),
          title: "Channels",
        },
        {
          count: searchQuery.data?.found ?? messageResults.length,
          results: messageResults.map((hit) => ({
            kind: "message" as const,
            hit,
          })),
          title: "Messages",
        },
      ].filter((section) => section.results.length > 0),
    [channelResults, messageResults, searchQuery.data?.found, userResults],
  );
  const results = React.useMemo<SearchResult[]>(
    () => sections.flatMap((section) => section.results),
    [sections],
  );
  const resultProfilesQuery = useUsersBatchQuery(
    messageResults.map((hit) => hit.pubkey),
    {
      enabled: open && messageResults.length > 0,
    },
  );
  const resultProfiles = resultProfilesQuery.data?.profiles;

  const openResult = React.useCallback(
    (result: SearchResult) => {
      onOpenChange(false);

      if (result.kind === "channel") {
        onOpenChannel(result.channel.id);
        return;
      }

      if (result.kind === "user") {
        onOpenUser(result.user.pubkey);
        return;
      }

      onOpenResult(result.hit);
    },
    [onOpenChange, onOpenChannel, onOpenResult, onOpenUser],
  );

  React.useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < MIN_QUERY_LENGTH) {
      setDebouncedQuery("");
      return;
    }

    const timeout = window.setTimeout(() => {
      setDebouncedQuery(trimmed);
    }, 300);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [query]);

  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
    }
  }, [open]);

  React.useEffect(() => {
    if (!open) {
      return;
    }

    setQuery(initialQuery);
    setSelectedIndex(0);
  }, [initialQuery, open]);

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (results.length === 0) {
        return 0;
      }

      return Math.min(current, results.length - 1);
    });
  }, [results]);

  const selectedResult = results[selectedIndex];

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-w-3xl translate-y-[-8vh] gap-0 overflow-hidden p-0 [&>button]:hidden"
        data-testid="search-dialog"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          inputRef.current?.focus();
        }}
      >
        <DialogHeader className="border-b border-border/80 px-4 py-3">
          <DialogTitle className="sr-only">Search Sprout</DialogTitle>
          <DialogDescription className="sr-only">
            Search people, channels, and message history across Sprout.
          </DialogDescription>
          <div className="flex items-center gap-2 rounded-xl border border-input bg-card px-3 py-2.5 shadow-sm">
            <Search className="h-4 w-4 text-muted-foreground" />
            <Input
              autoFocus
              className="h-auto border-0 bg-transparent px-0 py-0 text-base shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-0"
              data-testid="search-input"
              onChange={(event) => {
                setQuery(event.target.value);
                setSelectedIndex(0);
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown" && results.length > 0) {
                  event.preventDefault();
                  setSelectedIndex((current) =>
                    Math.min(current + 1, results.length - 1),
                  );
                  return;
                }

                if (event.key === "ArrowUp" && results.length > 0) {
                  event.preventDefault();
                  setSelectedIndex((current) => Math.max(current - 1, 0));
                  return;
                }

                if (
                  event.key === "Enter" &&
                  !event.nativeEvent.isComposing &&
                  selectedResult
                ) {
                  event.preventDefault();
                  openResult(selectedResult);
                }
              }}
              placeholder="Search Sprout"
              ref={inputRef}
              value={query}
            />
            <button
              aria-label="Close search"
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              onClick={() => onOpenChange(false)}
              type="button"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </DialogHeader>

        <div className="max-h-[64vh] overflow-y-auto">
          {debouncedQuery.length < MIN_QUERY_LENGTH ? (
            <SearchState
              description="Type at least two characters to search people, channels, streams, forums, DMs, approvals, and agent updates."
              icon={MessagesSquare}
              title="Search across Sprout"
            />
          ) : (searchQuery.isLoading || usersQuery.isLoading) &&
            results.length === 0 ? (
            <SearchLoadingState />
          ) : (searchQuery.error instanceof Error ||
              usersQuery.error instanceof Error) &&
            results.length === 0 ? (
            <SearchState
              description={
                searchQuery.error instanceof Error
                  ? searchQuery.error.message
                  : usersQuery.error instanceof Error
                    ? usersQuery.error.message
                    : "Search failed."
              }
              icon={LoaderCircle}
              title="Search unavailable"
            />
          ) : results.length === 0 ? (
            <SearchState
              description="Try a different keyword, username, channel name, or phrase from the message body."
              icon={Search}
              title="No matches found"
            />
          ) : (
            <div className="p-2.5" data-testid="search-results">
              <div className="space-y-3">
                {sections.map((section) => {
                  let sectionStartIndex = 0;
                  for (const previousSection of sections) {
                    if (previousSection === section) {
                      break;
                    }
                    sectionStartIndex += previousSection.results.length;
                  }

                  return (
                    <section key={section.title}>
                      <div className="mb-1.5 flex items-center justify-between px-1.5 text-[11px] font-medium text-muted-foreground">
                        <span>{section.title}</span>
                        <span>{section.count}</span>
                      </div>
                      <div className="space-y-1">
                        {section.results.map((result, index) => {
                          const absoluteIndex = sectionStartIndex + index;

                          return (
                            <SearchResultShell
                              icon={resultIcon(result, channelLookup)}
                              isSelected={absoluteIndex === selectedIndex}
                              key={resultKey(result)}
                              leading={
                                result.kind === "user" ? (
                                  <UserResultAvatar user={result.user} />
                                ) : undefined
                              }
                              onClick={() => openResult(result)}
                              onMouseEnter={() =>
                                setSelectedIndex(absoluteIndex)
                              }
                              testId={resultTestId(result)}
                            >
                              {result.kind === "channel" ? (
                                <ChannelResultBody channel={result.channel} />
                              ) : result.kind === "user" ? (
                                <UserResultBody user={result.user} />
                              ) : (
                                <MessageResultBody
                                  currentPubkey={currentPubkey}
                                  hit={result.hit}
                                  resultProfiles={resultProfiles}
                                />
                              )}
                            </SearchResultShell>
                          );
                        })}
                      </div>
                    </section>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
