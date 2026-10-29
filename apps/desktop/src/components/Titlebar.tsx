import { Bot, Command, Moon, PanelLeft, Settings, Sun } from "lucide-react";
import { kindInfo, type ConnectionConfig } from "../lib/api";
import { isMac, mod } from "../lib/types";
import { Button, Dot, Kbd, cx } from "./ui";

export function Titlebar({
  conn,
  dark,
  onToggleTheme,
  onToggleSidebar,
  onPalette,
  onMcp,
  onSettings,
}: {
  conn: ConnectionConfig | null;
  dark: boolean;
  onToggleTheme: () => void;
  onToggleSidebar: () => void;
  onPalette: () => void;
  onMcp: () => void;
  onSettings: () => void;
}) {
  return (
    <header
      data-tauri-drag-region
      className={cx("flex h-10 shrink-0 items-center gap-2 border-b border-line bg-panel pr-2", isMac ? "pl-[78px]" : "pl-2")}
    >
      <Button variant="ghost" size="icon" onClick={onToggleSidebar} title="Toggle sidebar">
        <PanelLeft size={14} />
      </Button>

      <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center gap-2 text-xs">
        <span data-tauri-drag-region className="font-semibold tracking-tight">Rowhouse</span>
        {conn && (
          <>
            <span className="text-subtle">/</span>
            <Dot color={conn.color} />
            <span className="truncate font-medium">{conn.name}</span>
            <span className="truncate text-muted">
              {kindInfo(conn.kind).file ? conn.file : `${conn.user ?? ""}@${conn.host ?? ""}${conn.database ? "/" + conn.database : ""}`}
            </span>
            {conn.readOnly && (
              <span className="rounded bg-warning/15 px-1.5 py-px text-[10px] font-semibold text-warning">READ-ONLY</span>
            )}
          </>
        )}
      </div>

      <button
        onClick={onPalette}
        className="flex h-6 w-60 items-center gap-2 rounded-md border border-line bg-elevated px-2 text-2xs text-subtle hover:border-line-strong"
      >
        <Command size={11} />
        <span className="flex-1 truncate text-left whitespace-nowrap">Jump to table or command…</span>
        <Kbd>{mod}</Kbd>
        <Kbd>K</Kbd>
      </button>
      <Button variant="ghost" size="icon" onClick={onMcp} title="Connect an AI assistant (MCP)">
        <Bot size={14} />
      </Button>
      <Button variant="ghost" size="icon" onClick={onSettings} title="Settings (AI)">
        <Settings size={14} />
      </Button>
      <Button variant="ghost" size="icon" onClick={onToggleTheme} title="Toggle theme">
        {dark ? <Sun size={14} /> : <Moon size={14} />}
      </Button>
    </header>
  );
}
