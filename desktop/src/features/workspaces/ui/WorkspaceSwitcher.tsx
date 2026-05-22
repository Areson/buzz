import {
  Check,
  ChevronDown,
  MoreHorizontal,
  Plus,
  WifiOff,
} from "lucide-react";
import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import type { Workspace } from "@/features/workspaces/types";
import { getUserProfile } from "@/shared/api/tauri";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

import { EditWorkspaceDialog } from "./EditWorkspaceDialog";

const CONNECTION_STATE_LABEL: Record<ConnectionState, string> = {
  idle: "Not connected",
  connecting: "Connecting…",
  connected: "Connected",
  reconnecting: "Reconnecting to relay…",
  stalled: "Connection lost — relay is not responding",
  disconnected: "Disconnected from relay",
};

function relayIconUrl(relayUrl: string | undefined) {
  if (!relayUrl) {
    return null;
  }

  try {
    const url = new URL(relayUrl);
    url.protocol = url.protocol === "ws:" ? "http:" : "https:";
    url.pathname = "/favicon.ico";
    url.search = "";
    url.hash = "";
    return url.toString();
  } catch {
    return null;
  }
}

function relayInfoUrl(relayUrl: string | undefined) {
  if (!relayUrl) {
    return null;
  }

  try {
    const url = new URL(relayUrl);
    url.protocol = url.protocol === "ws:" ? "http:" : "https:";
    url.pathname = "/";
    url.search = "";
    url.hash = "";
    return url.toString();
  } catch {
    return null;
  }
}

async function fetchRelayProfileIcon(relayUrl: string) {
  const infoUrl = relayInfoUrl(relayUrl);
  if (!infoUrl) {
    return null;
  }

  const response = await fetch(infoUrl, {
    headers: {
      Accept: "application/nostr+json",
    },
  });

  if (!response.ok) {
    return null;
  }

  const info = (await response.json()) as { self?: unknown };
  if (typeof info.self !== "string" || info.self.length === 0) {
    return null;
  }

  const profile = await getUserProfile(info.self);
  return profile.avatarUrl ? rewriteRelayUrl(profile.avatarUrl) : null;
}

function RelayIcon({
  associatedIconUrl,
  className,
  workspace,
}: {
  associatedIconUrl?: string | null;
  className: string;
  workspace: Workspace | null | undefined;
}) {
  const iconUrl = relayIconUrl(workspace?.relayUrl);
  const fallbackIconUrl = "/sprout.svg";
  const [src, setSrc] = React.useState<string | null>(
    associatedIconUrl ?? iconUrl ?? fallbackIconUrl,
  );

  React.useEffect(() => {
    setSrc(associatedIconUrl ?? iconUrl ?? fallbackIconUrl);
  }, [associatedIconUrl, iconUrl]);

  if (!src) {
    return null;
  }

  if (src === fallbackIconUrl) {
    return (
      <span
        aria-hidden="true"
        className={`${className} flex items-center justify-center leading-none`}
      >
        🌱
      </span>
    );
  }

  return (
    <img
      alt=""
      className={className}
      onError={() => {
        setSrc((current) =>
          current === fallbackIconUrl ? null : fallbackIconUrl,
        );
      }}
      referrerPolicy="no-referrer"
      src={src}
    />
  );
}

type WorkspaceSwitcherProps = {
  activeWorkspace: Workspace | null;
  placement?: "header" | "footer";
  workspaces: Workspace[];
  onSwitchWorkspace: (id: string) => void;
  onAddWorkspace: () => void;
  onUpdateWorkspace: (
    id: string,
    updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveWorkspace: (id: string) => void;
};

export function WorkspaceSwitcher({
  activeWorkspace,
  placement = "header",
  workspaces,
  onSwitchWorkspace,
  onAddWorkspace,
  onUpdateWorkspace,
  onRemoveWorkspace,
}: WorkspaceSwitcherProps) {
  const [editingWorkspace, setEditingWorkspace] =
    React.useState<Workspace | null>(null);
  const [dropdownOpen, setDropdownOpen] = React.useState(false);
  const connectionState = useRelayConnection();
  const degraded = isRelayConnectionDegraded(connectionState);
  const connectionLabel = CONNECTION_STATE_LABEL[connectionState];
  const isFooterPlacement = placement === "footer";
  const activeRelayIconQuery = useQuery({
    enabled: Boolean(activeWorkspace?.relayUrl),
    queryKey: ["relay-profile-icon", activeWorkspace?.relayUrl ?? ""],
    queryFn: () => fetchRelayProfileIcon(activeWorkspace?.relayUrl ?? ""),
    staleTime: 5 * 60 * 1_000,
    gcTime: 30 * 60 * 1_000,
    retry: false,
  });
  const activeRelayIconUrl = activeRelayIconQuery.data ?? null;
  const triggerLabel = degraded
    ? `${activeWorkspace?.name ?? "Workspace"} — ${connectionLabel}`
    : (activeWorkspace?.name ?? "No workspace");
  const triggerContent = (
    <>
      {degraded ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <span
              aria-hidden="false"
              className={
                isFooterPlacement
                  ? "flex h-3.5 w-3.5 shrink-0 animate-pulse items-center justify-center text-destructive"
                  : "flex h-5 w-5 shrink-0 animate-pulse items-center justify-center text-destructive"
              }
              data-testid="relay-connection-warning"
              role="img"
            >
              <WifiOff className={isFooterPlacement ? "h-3 w-3" : "h-4 w-4"} />
            </span>
          </TooltipTrigger>
          <TooltipContent side={isFooterPlacement ? "top" : "bottom"}>
            {connectionLabel}
          </TooltipContent>
        </Tooltip>
      ) : isFooterPlacement ? null : (
        <RelayIcon
          associatedIconUrl={activeRelayIconUrl}
          className="h-5 w-5 shrink-0 rounded-md object-cover"
          workspace={activeWorkspace}
        />
      )}
      {!degraded && isFooterPlacement ? (
        <RelayIcon
          associatedIconUrl={activeRelayIconUrl}
          className="h-3 w-3 shrink-0 rounded-sm object-cover"
          workspace={activeWorkspace}
        />
      ) : null}
      <span
        className={
          degraded
            ? "min-w-0 flex-1 truncate text-destructive animate-pulse"
            : "min-w-0 flex-1 truncate"
        }
      >
        {activeWorkspace?.name ?? "No workspace"}
      </span>
      <ChevronDown
        className={
          isFooterPlacement
            ? "h-3 w-3 shrink-0 text-sidebar-foreground/40"
            : "h-3.5 w-3.5 shrink-0 text-sidebar-foreground/50"
        }
      />
    </>
  );
  const dropdown = (
    <DropdownMenu open={dropdownOpen} onOpenChange={setDropdownOpen}>
      <DropdownMenuTrigger asChild>
        {isFooterPlacement ? (
          <button
            aria-label={triggerLabel}
            className="mt-0.5 flex min-w-0 items-center gap-1 rounded-md text-xs leading-4 text-sidebar-foreground/50 transition-colors hover:text-sidebar-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sidebar-ring data-[state=open]:text-sidebar-foreground"
            data-testid="workspace-switcher"
            onClick={(event) => {
              event.stopPropagation();
            }}
            type="button"
          >
            {triggerContent}
          </button>
        ) : (
          <SidebarMenuButton
            aria-label={degraded ? triggerLabel : undefined}
            className="h-auto gap-2 rounded-xl px-2.5 py-2 data-[state=open]:bg-sidebar-accent"
            data-testid="workspace-switcher"
            type="button"
          >
            {triggerContent}
          </SidebarMenuButton>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className={
          isFooterPlacement
            ? "min-w-[220px]"
            : "w-[--radix-dropdown-menu-trigger-width] min-w-[220px]"
        }
        onCloseAutoFocus={(e) => e.preventDefault()}
        side={isFooterPlacement ? "top" : "bottom"}
        sideOffset={4}
      >
        {workspaces.map((workspace) => (
          <DropdownMenuItem
            key={workspace.id}
            className="group flex items-center gap-2 pr-1"
            onSelect={() => {
              onSwitchWorkspace(workspace.id);
            }}
          >
            <span className="flex h-4 w-4 shrink-0 items-center justify-center">
              {activeWorkspace?.id === workspace.id ? (
                <Check className="h-3.5 w-3.5 text-primary" />
              ) : null}
            </span>
            <RelayIcon
              associatedIconUrl={
                activeWorkspace?.id === workspace.id ? activeRelayIconUrl : null
              }
              className="h-4 w-4 shrink-0 rounded-sm object-cover"
              workspace={workspace}
            />
            <span className="min-w-0 flex-1 truncate">{workspace.name}</span>
            <button
              aria-label={`Edit ${workspace.name}`}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded opacity-0 hover:bg-accent group-hover:opacity-100 group-focus:opacity-100"
              onClick={(e) => {
                e.stopPropagation();
                e.preventDefault();
                setDropdownOpen(false);
                setEditingWorkspace(workspace);
              }}
              type="button"
            >
              <MoreHorizontal className="h-3.5 w-3.5" />
            </button>
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onAddWorkspace}>
          <Plus className="h-4 w-4" />
          <span>Add Workspace</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  return (
    <>
      {isFooterPlacement ? (
        dropdown
      ) : (
        <SidebarMenu>
          <SidebarMenuItem>{dropdown}</SidebarMenuItem>
        </SidebarMenu>
      )}

      <EditWorkspaceDialog
        canRemove={workspaces.length > 1}
        onOpenChange={(open) => {
          if (!open) setEditingWorkspace(null);
        }}
        onRemove={onRemoveWorkspace}
        onSave={onUpdateWorkspace}
        open={editingWorkspace !== null}
        workspace={editingWorkspace}
      />
    </>
  );
}
