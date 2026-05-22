import * as React from "react";
import { Trash2 } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "motion/react";

import { useTerminal } from "./TerminalContext";

type SessionStatus = "idle" | "active" | "exited" | "none";

type TerminalStatusPillProps = {
  channelId: string;
};

/**
 * Tiny proof-of-life indicator for a terminal session.
 * Shows when the terminal panel is closed but a PTY session exists.
 */
export function TerminalStatusPill({ channelId }: TerminalStatusPillProps) {
  const { isOpen, toggle } = useTerminal();
  const [status, setStatus] = React.useState<SessionStatus>("none");
  const [lastLine, setLastLine] = React.useState("");
  const [isHovered, setIsHovered] = React.useState(false);
  const activityTimeoutRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  // Check session existence on mount and channel change.
  React.useEffect(() => {
    let cancelled = false;

    async function check() {
      try {
        const exists = await invoke<boolean>("terminal_has_session", {
          channelId,
        });
        if (cancelled) return;
        if (!exists) {
          setStatus("none");
          setLastLine("");
          return;
        }
        setStatus("idle");
        const line = await invoke<string | null>("terminal_get_last_line", {
          channelId,
        });
        if (!cancelled && line) {
          setLastLine(line);
        }
      } catch {
        // Ignore — session may not exist.
      }
    }

    check();
    return () => {
      cancelled = true;
    };
  }, [channelId]);

  // Listen for terminal:data and terminal:exit to update status.
  React.useEffect(() => {
    let unlistenData: UnlistenFn | null = null;
    let unlistenExit: UnlistenFn | null = null;

    listen<{ sessionId: string; data: string }>("terminal:data", (event) => {
      // We don't have the sessionId→channelId mapping on the frontend,
      // so just mark as active on any data event and refresh last line.
      setStatus("active");
      if (activityTimeoutRef.current) {
        clearTimeout(activityTimeoutRef.current);
      }
      activityTimeoutRef.current = setTimeout(() => {
        setStatus((prev) => (prev === "active" ? "idle" : prev));
      }, 2000);

      // Extract last line from the data chunk.
      const lines = event.payload.data.split("\n");
      // biome-ignore lint/suspicious/noControlCharactersInRegex: ANSI escape stripping requires \x1b
      const ansiRegex = /\x1b\[[^a-zA-Z~]*[a-zA-Z~]/g;
      const last = lines.reverse().find((l: string) => l.trim().length > 0);
      if (last) {
        setLastLine(last.replace(ansiRegex, "").trim());
      }
    }).then((fn) => {
      unlistenData = fn;
    });

    listen<{ sessionId: string; exitCode: number }>(
      "terminal:exit",
      (_event) => {
        setStatus("exited");
      },
    ).then((fn) => {
      unlistenExit = fn;
    });

    return () => {
      unlistenData?.();
      unlistenExit?.();
      if (activityTimeoutRef.current) {
        clearTimeout(activityTimeoutRef.current);
      }
    };
  }, []);

  const handleKill = React.useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        // Get session ID for this channel then close it.
        const exists = await invoke<boolean>("terminal_has_session", {
          channelId,
        });
        if (exists) {
          // We need the session ID — open_session returns it without creating a new one.
          // For now, close all for this channel by re-opening to get the ID.
          // Actually, let's add a close-by-channel approach.
          // Simpler: just close the session via the existing command by getting the ID from open.
          const result = await invoke<{ sessionId: string }>(
            "terminal_open_session",
            { input: { channelId, cols: 80, rows: 24 } },
          );
          await invoke("terminal_close_session", {
            input: { sessionId: result.sessionId },
          });
        }
        setStatus("none");
        setLastLine("");
      } catch {
        // Ignore.
      }
    },
    [channelId],
  );

  // Don't show if terminal is expanded or no session exists.
  if (isOpen || status === "none") return null;

  return (
    <AnimatePresence>
      <motion.button
        type="button"
        initial={{ opacity: 0, scale: 0.9 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.9 }}
        transition={{ duration: 0.15 }}
        className="flex h-7 w-16 items-center gap-1.5 rounded-full border border-border/50 bg-terminal px-2 text-terminal-foreground shadow-sm transition-colors hover:border-border"
        onClick={toggle}
        onMouseEnter={() => setIsHovered(true)}
        onMouseLeave={() => setIsHovered(false)}
        aria-label="Open terminal"
      >
        {/* Status dot */}
        <span
          className={`size-2 shrink-0 rounded-full ${
            status === "active"
              ? "animate-pulse bg-white"
              : status === "exited"
                ? "bg-red-500"
                : "bg-green-500"
          }`}
        />

        {/* Last line preview */}
        <span className="min-w-0 flex-1 truncate font-mono text-[9px] leading-none opacity-70">
          {lastLine || "…"}
        </span>

        {/* Kill button on hover */}
        {isHovered ? (
          <button
            type="button"
            className="shrink-0 rounded p-0.5 text-terminal-foreground/50 hover:text-red-400 transition-colors"
            onClick={handleKill}
            aria-label="Kill terminal session"
          >
            <Trash2 className="size-2.5" />
          </button>
        ) : null}
      </motion.button>
    </AnimatePresence>
  );
}
