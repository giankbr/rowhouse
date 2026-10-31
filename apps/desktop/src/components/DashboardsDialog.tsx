import { LayoutGrid, Pin, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type ConnectionConfig, type Dashboard, type DashTile } from "../lib/api";
import { uid } from "../lib/types";
import { Button, Input, Modal, cx } from "./ui";

/** All dashboards (this connection first); create a new one. */
export function DashboardsDialog({
  open,
  conn,
  connections,
  onClose,
  onOpen,
  onToast,
}: {
  open: boolean;
  conn: ConnectionConfig | null;
  connections: ConnectionConfig[];
  onClose: () => void;
  onOpen: (d: Dashboard) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [list, setList] = useState<Dashboard[] | null>(null);
  const [name, setName] = useState("");
  useEffect(() => {
    if (!open) return;
    setName("");
    api.dashboards.list().then(setList).catch((e) => onToast(String(e)));
  }, [open, onToast]);
  const create = async () => {
    if (!conn || !name.trim()) return;
    try {
      const d = await api.dashboards.save({ id: uid(), name: name.trim(), connectionId: conn.id, tiles: [], refreshSecs: 0, updatedAt: 0 });
      onOpen(d);
      onClose();
    } catch (e) {
      onToast(String(e));
    }
  };
  const sorted = [...(list ?? [])].sort((a, b) => Number(b.connectionId === conn?.id) - Number(a.connectionId === conn?.id) || b.updatedAt - a.updatedAt);
  return (
    <Modal
      open={open}
      onClose={onClose}
      width={520}
      title={
        <span className="flex items-center gap-2">
          <LayoutGrid size={14} className="text-accent" /> Dashboards
        </span>
      }
      footer={<Button onClick={onClose}>Close</Button>}
    >
      <div className="flex flex-col gap-3">
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            create();
          }}
        >
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={conn ? `New dashboard on ${conn.name}` : "Open a connection first"} disabled={!conn} />
          <Button type="submit" variant="primary" disabled={!conn || !name.trim()}>
            <Plus size={12} /> Create
          </Button>
        </form>
        {list && list.length === 0 && <p className="text-xs text-muted">No dashboards yet. Create one here, or pin a chart from a query tab.</p>}
        <div className="-mx-1 flex flex-col">
          {sorted.map((d) => {
            const c = connections.find((x) => x.id === d.connectionId);
            return (
              <button
                key={d.id}
                onClick={() => (onOpen(d), onClose())}
                className={cx("flex items-center gap-3 rounded-lg px-2.5 py-2 text-left hover:bg-hover", d.connectionId !== conn?.id && "opacity-75")}
              >
                <LayoutGrid size={14} className="shrink-0 text-subtle" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs font-medium">{d.name}</span>
                  <span className="block truncate text-2xs text-subtle">
                    {c?.name ?? "missing connection"} · {d.tiles.length} tile{d.tiles.length === 1 ? "" : "s"}
                    {d.refreshSecs ? ` · refreshes every ${d.refreshSecs >= 3600 ? "hour" : `${d.refreshSecs / 60} min`}` : ""}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </div>
    </Modal>
  );
}

/** Pin the current query (and its chart) to a dashboard. */
export function PinDialog({
  open,
  conn,
  tile,
  onClose,
  onPinned,
  onToast,
}: {
  open: boolean;
  conn: ConnectionConfig | null;
  tile: Omit<DashTile, "id" | "w" | "h"> | null;
  onClose: () => void;
  onPinned: (d: Dashboard) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [list, setList] = useState<Dashboard[]>([]);
  const [target, setTarget] = useState<string>("new");
  const [newName, setNewName] = useState("");
  const [title, setTitle] = useState("");
  useEffect(() => {
    if (!open || !conn) return;
    setTitle(tile?.title ?? "");
    setNewName("");
    api.dashboards
      .list()
      .then((l) => {
        const mine = l.filter((d) => d.connectionId === conn.id).sort((a, b) => b.updatedAt - a.updatedAt);
        setList(mine);
        setTarget(mine[0]?.id ?? "new");
      })
      .catch(() => {});
  }, [open, conn, tile]);
  if (!open || !conn || !tile) return null;
  const pin = async () => {
    const t: DashTile = { ...tile, id: uid(), title: title.trim() || tile.title || "Untitled", w: tile.view === "number" ? 3 : 6, h: 1 };
    try {
      const base = target === "new" ? { id: uid(), name: newName.trim() || "My dashboard", connectionId: conn.id, tiles: [], refreshSecs: 0, updatedAt: 0 } : list.find((d) => d.id === target)!;
      const d = await api.dashboards.save({ ...base, tiles: [...base.tiles, t] });
      onPinned(d);
      onToast(`Pinned to ${d.name}`, "ok");
      onClose();
    } catch (e) {
      onToast(String(e));
    }
  };
  return (
    <Modal
      open
      onClose={onClose}
      width={440}
      title={
        <span className="flex items-center gap-2">
          <Pin size={14} className="text-accent" /> Pin to dashboard
        </span>
      }
      footer={
        <>
          <span className="flex-1" />
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={pin}>
            Pin
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-2xs text-muted">
          Tile title
          <Input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Revenue this week" />
        </label>
        <label className="flex flex-col gap-1 text-2xs text-muted">
          Dashboard
          <select value={target} onChange={(e) => setTarget(e.target.value)} className="h-7 rounded-md border border-line bg-elevated px-1.5 text-xs text-fg outline-none">
            {list.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
            <option value="new">+ New dashboard…</option>
          </select>
        </label>
        {target === "new" && <Input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder={`Dashboard name (e.g. ${conn.name} overview)`} />}
        <p className="text-2xs text-subtle">
          Shown as a {tile.view}
          {tile.snippetId != null ? "; it follows the saved query, so later edits show up on the dashboard" : ""}. Tiles always run read-only.
        </p>
      </div>
    </Modal>
  );
}
