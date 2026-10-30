import { AlertTriangle, Bot, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, isTauri, type ConnectionConfig, type Destructive } from "../lib/api";
import { confirmPhrase } from "./ConfirmDestructive";
import { Button, Input, Modal, cx } from "./ui";

export interface ApprovalRequest {
  id: string;
  connectionId: string;
  connectionName: string;
  sql: string;
  client: string;
  createdAt: number;
}

/**
 * Polls for write requests from rowhouse-mcp and asks the user, one at a time.
 * Destructive statements additionally need the table name typed.
 */
export function ApprovalCenter({ connections }: { connections: ConnectionConfig[] }) {
  const [queue, setQueue] = useState<ApprovalRequest[]>([]);
  const [destructive, setDestructive] = useState<Destructive[]>([]);
  const [typed, setTyped] = useState("");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const seen = useRef(new Set<string>());
  const current = queue[0];

  const poll = useCallback(async () => {
    try {
      const pending = await api.approvals.pending();
      setQueue(pending);
      const fresh = pending.filter((p) => !seen.current.has(p.id));
      fresh.forEach((p) => seen.current.add(p.id));
      if (fresh.length && isTauri) {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const w = getCurrentWindow();
        await w.unminimize().catch(() => {});
        await w.setFocus().catch(() => {});
        await w.requestUserAttention(2).catch(() => {});
      }
    } catch {
      /* not fatal */
    }
  }, []);

  useEffect(() => {
    poll();
    const t = setInterval(poll, 1000);
    return () => clearInterval(t);
  }, [poll]);

  useEffect(() => {
    setTyped("");
    setReason("");
    if (current) api.checkDestructive(current.sql).then(setDestructive).catch(() => setDestructive([]));
  }, [current?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!current) return null;
  const conn = connections.find((c) => c.id === current.connectionId);
  const prod = conn?.env === "production";
  const phrase = destructive.length ? confirmPhrase(destructive) : null;
  const canApprove = !phrase || typed.trim() === phrase;

  const answer = async (approved: boolean) => {
    setBusy(true);
    try {
      await api.approvals.answer(current.id, approved, approved ? undefined : reason.trim() || undefined);
    } finally {
      setBusy(false);
      setQueue((q) => q.slice(1));
    }
  };

  return (
    <Modal
      open
      onClose={() => {}}
      width={600}
      title={
        <span className="flex items-center gap-2">
          <Bot size={15} className="text-accent" /> An AI assistant wants to change data
          {queue.length > 1 && <span className="rounded bg-hover px-1.5 text-2xs font-normal text-muted">+{queue.length - 1} more</span>}
        </span>
      }
      footer={
        <>
          <Input
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            placeholder="Reason for declining (optional, sent back to the AI)"
            className="flex-1"
          />
          <Button onClick={() => answer(false)} disabled={busy}>
            Decline
          </Button>
          <Button variant="primary" className={cx(prod && "bg-danger")} onClick={() => answer(true)} disabled={busy || !canApprove}>
            Approve & run
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        <div className="flex items-center gap-2 text-muted">
          <span>
            {current.client} → <span className="font-medium text-fg">{current.connectionName}</span>
          </span>
          {conn?.env && conn.env !== "none" && (
            <span className={cx("rounded px-1.5 py-px text-[10px] font-semibold uppercase", prod ? "bg-danger text-white" : "bg-hover")}>
              {conn.env}
            </span>
          )}
          <span className="ml-auto text-subtle">{new Date(current.createdAt * 1000).toLocaleTimeString()}</span>
        </div>
        <pre className="selectable max-h-64 overflow-auto rounded-lg border border-line bg-panel p-3 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap">
          {current.sql}
        </pre>
        {prod && (
          <div className="flex items-center gap-1.5 text-danger">
            <ShieldAlert size={13} /> This is a production connection.
          </div>
        )}
        {phrase && (
          <div className="flex flex-col gap-1.5 rounded-lg border border-danger/30 bg-danger/5 p-2.5">
            <span className="flex items-center gap-1.5 text-danger">
              <AlertTriangle size={13} /> Destructive: {destructive.map((d) => `${d.kind} ${d.object}`).join(", ")}
            </span>
            <span className="text-muted">
              Type <span className="rounded bg-panel px-1 font-mono font-semibold text-fg">{phrase}</span> to allow it
            </span>
            <Input value={typed} onChange={(e) => setTyped(e.target.value)} className="font-mono" />
          </div>
        )}
        <p className="text-2xs text-subtle">The statement runs exactly as shown, once. Nothing else is allowed by approving it.</p>
      </div>
    </Modal>
  );
}
