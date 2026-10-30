import { Command, History, Network, PanelLeftOpen, SquarePen } from "lucide-react";
import type { ConnectionConfig } from "../lib/api";
import { mod } from "../lib/types";
import { KindBadge } from "./KindBadge";
import { cx } from "./ui";

/** Collapsed sidebar: a slim icon rail that keeps the essentials one click away. */
export function SidebarRail({
  conn,
  onExpand,
  onSwitch,
  onNewQuery,
  onErd,
  onHistory,
  onPalette,
}: {
  conn: ConnectionConfig | null;
  onExpand: () => void;
  onSwitch: () => void;
  onNewQuery: () => void;
  onErd: () => void;
  onHistory: () => void;
  onPalette: () => void;
}) {
  const btn = "flex size-8 items-center justify-center rounded-md text-muted hover:bg-hover hover:text-fg";
  return (
    <div className="flex w-11 flex-col items-center gap-1 bg-panel py-1.5">
      <button className={cx(btn, "mb-1")} title={`Switch connection (${mod}+O)`} onClick={onSwitch}>
        {conn ? <KindBadge kind={conn.kind} size={20} /> : <Command size={14} />}
      </button>
      <button className={btn} title={`Show sidebar (${mod}+B)`} onClick={onExpand}>
        <PanelLeftOpen size={15} />
      </button>
      <button className={btn} title={`New query (${mod}+T)`} onClick={onNewQuery}>
        <SquarePen size={14} />
      </button>
      <button className={btn} title="Diagram" onClick={onErd}>
        <Network size={14} />
      </button>
      <button className={btn} title={`History (${mod}+Y)`} onClick={onHistory}>
        <History size={14} />
      </button>
      <span className="flex-1" />
      <button className={btn} title={`Command palette (${mod}+K)`} onClick={onPalette}>
        <Command size={14} />
      </button>
    </div>
  );
}
