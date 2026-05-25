import { ChevronLeft, ChevronRight } from "lucide-react";

import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { Button } from "@/shared/ui/button";
import type { Channel, SearchHit } from "@/shared/api/types";
import { SidebarTrigger } from "@/shared/ui/sidebar";

type AppHeaderControlsProps = {
  canGoBack: boolean;
  canGoForward: boolean;
  channels: Channel[];
  currentPubkey?: string;
  onGoBack: () => void;
  onGoForward: () => void;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
  onOpenUser: (pubkey: string) => void;
};

export function AppHeaderControls({
  canGoBack,
  canGoForward,
  channels,
  currentPubkey,
  onGoBack,
  onGoForward,
  onOpenChannel,
  onOpenResult,
  onOpenUser,
}: AppHeaderControlsProps) {
  return (
    <>
      <div className="pointer-events-none fixed inset-x-0 top-0 z-20 h-20 bg-background/35 backdrop-blur-2xl backdrop-saturate-150 [mask-image:linear-gradient(to_bottom,black_0%,black_45%,transparent_100%)] supports-[backdrop-filter]:bg-background/25 [-webkit-mask-image:linear-gradient(to_bottom,black_0%,black_45%,transparent_100%)]" />

      <div className="pointer-events-none fixed inset-x-0 top-0 z-[70] h-10">
        <div className="pointer-events-auto absolute left-[80px] top-[9px] flex items-center gap-0.5">
          <SidebarTrigger className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground" />
          <Button
            aria-label="Go back"
            className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
            data-testid="global-back"
            disabled={!canGoBack}
            onClick={onGoBack}
            size="icon"
            variant="ghost"
          >
            <ChevronLeft className="h-3 w-3" />
          </Button>
          <Button
            aria-label="Go forward"
            className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
            data-testid="global-forward"
            disabled={!canGoForward}
            onClick={onGoForward}
            size="icon"
            variant="ghost"
          >
            <ChevronRight className="h-3 w-3" />
          </Button>
        </div>

        <div className="pointer-events-auto absolute left-1/2 top-[7px] w-[min(420px,42vw)] -translate-x-1/2">
          <TopbarSearch
            channels={channels}
            currentPubkey={currentPubkey}
            onOpenChannel={onOpenChannel}
            onOpenResult={onOpenResult}
            onOpenUser={onOpenUser}
          />
        </div>
      </div>
    </>
  );
}
