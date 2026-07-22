import * as React from "react";

import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";

type RenderChannelAuxiliaryPaneOptions = Pick<
  React.ComponentProps<typeof RightAuxiliaryPane>,
  "canResetWidth" | "onResetWidth" | "onResizeStart" | "widthPx"
> & {
  paneKey?: string;
  panel: React.ReactNode;
  testId: string;
  useSplitAuxiliaryPane: boolean;
};

export function renderChannelAuxiliaryPane({
  canResetWidth,
  onResetWidth,
  onResizeStart,
  paneKey,
  panel,
  testId,
  useSplitAuxiliaryPane,
  widthPx,
}: RenderChannelAuxiliaryPaneOptions) {
  const key = paneKey ?? testId;
  return useSplitAuxiliaryPane ? (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      key={key}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId={testId}
      widthPx={widthPx}
    >
      {panel}
    </RightAuxiliaryPane>
  ) : (
    <React.Fragment key={key}>{panel}</React.Fragment>
  );
}
