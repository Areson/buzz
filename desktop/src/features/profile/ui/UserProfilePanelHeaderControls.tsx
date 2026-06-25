import { ArrowLeft, X } from "lucide-react";

import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import type { ProfilePanelView } from "@/features/profile/ui/UserProfilePanelUtils";
import {
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { Button } from "@/shared/ui/button";

export function UserProfilePanelHeaderLeft({
  title,
  view,
  onBack,
}: {
  title: string;
  view: ProfilePanelView;
  onBack: () => void;
}) {
  return (
    <AuxiliaryPanelHeaderGroup>
      {view !== "summary" ? (
        <Button
          aria-label="Back to profile"
          className="shrink-0"
          data-testid="user-profile-panel-back"
          onClick={onBack}
          size="icon"
          type="button"
          variant="outline"
        >
          <ArrowLeft />
        </Button>
      ) : null}
      <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>
    </AuxiliaryPanelHeaderGroup>
  );
}

export function UserProfilePanelHeaderActions({
  effectivePubkey,
  view,
  viewerIsOwner,
  onClose,
}: {
  effectivePubkey: string | null;
  view: ProfilePanelView;
  viewerIsOwner: boolean;
  onClose: () => void;
}) {
  return (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      {view === "memories" && viewerIsOwner && effectivePubkey ? (
        <MemoryRefreshButton
          agentPubkey={effectivePubkey}
          variant="outline"
          viewerIsOwner={viewerIsOwner}
        />
      ) : null}
      <Button
        aria-label="Close profile"
        data-testid="user-profile-panel-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );
}
