import * as React from "react";
import { Search } from "lucide-react";

import {
  useUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { useSearchMessagesQuery } from "@/features/search/hooks";
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
import type { Channel, SearchHit } from "@/shared/api/types";
import { Input } from "@/shared/ui/input";

const MIN_QUERY_LENGTH = 2;

type SearchResultSection = {
  count: number;
  results: SearchResult[];
  title: string;
};

type TopbarSearchProps = {
  channels: Channel[];
  currentPubkey?: string;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
  onOpenUser: (pubkey: string) => void;
};

export function TopbarSearch({
  channels,
  currentPubkey,
  onOpenChannel,
  onOpenResult,
  onOpenUser,
}: TopbarSearchProps) {
  const [query, setQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const [isFocused, setIsFocused] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const trimmedQuery = query.trim();
  const shouldSearch = isFocused && trimmedQuery.length >= MIN_QUERY_LENGTH;
  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  const searchQuery = useSearchMessagesQuery(trimmedQuery, {
    enabled: shouldSearch,
    limit: 8,
  });
  const usersQuery = useUserSearchQuery(trimmedQuery, {
    enabled: shouldSearch,
    limit: 4,
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
    if (!shouldSearch) {
      return [];
    }

    const normalizedQuery = trimmedQuery.toLowerCase();

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
      .slice(0, 4);
  }, [channels, shouldSearch, trimmedQuery]);

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
      enabled: shouldSearch && messageResults.length > 0,
    },
  );
  const resultProfiles = resultProfilesQuery.data?.profiles;

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (results.length === 0) {
        return 0;
      }

      return Math.min(current, results.length - 1);
    });
  }, [results]);

  const reset = React.useCallback(() => {
    setQuery("");
    setSelectedIndex(0);
    setIsFocused(false);
    inputRef.current?.blur();
  }, []);

  const openResult = React.useCallback(
    (result: SearchResult) => {
      reset();

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
    [onOpenChannel, onOpenResult, onOpenUser, reset],
  );

  const selectedResult = results[selectedIndex];
  const isSearching = searchQuery.isLoading || usersQuery.isLoading;
  const showResults = shouldSearch;

  return (
    <div className="relative">
      <div className="flex h-7 items-center gap-2 rounded-lg border border-border/70 bg-background/75 px-2.5 shadow-sm backdrop-blur">
        <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <Input
          aria-label="Search Sprout"
          className="h-auto border-0 bg-transparent px-0 py-0 text-xs shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-0"
          data-testid="global-search"
          onBlur={() => {
            window.setTimeout(() => setIsFocused(false), 120);
          }}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
            setIsFocused(true);
          }}
          onFocus={() => setIsFocused(true)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              reset();
              return;
            }

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
      </div>

      {showResults ? (
        <div
          className="absolute left-1/2 top-full mt-1 max-h-[60vh] w-[min(520px,64vw)] -translate-x-1/2 overflow-y-auto rounded-2xl border border-border/80 bg-popover p-2 text-popover-foreground shadow-2xl"
          data-testid="topbar-search-results"
        >
          {results.length === 0 ? (
            <p className="px-2 py-2 text-xs text-muted-foreground">
              {isSearching ? "Searching..." : "No matches found."}
            </p>
          ) : (
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
                            onMouseEnter={() => setSelectedIndex(absoluteIndex)}
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
          )}
        </div>
      ) : null}
    </div>
  );
}
