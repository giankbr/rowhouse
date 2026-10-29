import { AlertTriangle } from "lucide-react";
import { useEffect, useState } from "react";
import type { ConnectionConfig, Destructive } from "../lib/api";
import { Button, Input, Modal } from "./ui";

const LABEL: Record<Destructive["kind"], string> = {
  drop: "Drops",
  truncate: "Empties",
  "delete-all": "Deletes every row of",
  "update-all": "Updates every row of",
  "drop-column": "Drops columns from",
};

/** The phrase the user must type: the object's name if there is one, else CONFIRM. */
export function confirmPhrase(items: Destructive[]): string {
  const objects = [...new Set(items.map((i) => i.object.split(".").pop() ?? i.object))];
  return objects.length === 1 && objects[0] !== "?" ? objects[0] : "CONFIRM";
}

export function ConfirmDestructive({
  items,
  conn,
  onCancel,
  onConfirm,
}: {
  items: Destructive[] | null;
  conn: ConnectionConfig | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [typed, setTyped] = useState("");
  useEffect(() => setTyped(""), [items]);
  if (!items) return null;
  const phrase = confirmPhrase(items);
  const prod = conn?.env === "production";
  const ok = typed.trim() === phrase;

  return (
    <Modal
      open
      onClose={onCancel}
      width={480}
      title={
        <span className="flex items-center gap-2 text-danger">
          <AlertTriangle size={15} /> Destructive query{prod && " on PRODUCTION"}
        </span>
      }
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">This can't be undone.</span>
          <Button onClick={onCancel}>Cancel</Button>
          <Button variant="primary" className="bg-danger" disabled={!ok} onClick={onConfirm}>
            Run anyway
          </Button>
        </>
      }
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => (e.preventDefault(), ok && onConfirm())}>
        <ul className="flex flex-col gap-1.5">
          {items.map((d, i) => (
            <li key={i} className="rounded-md border border-danger/25 bg-danger/5 px-2.5 py-1.5 text-xs">
              <span className="text-danger">{LABEL[d.kind]}</span> <span className="font-mono font-semibold">{d.object}</span>
              <pre className="selectable mt-1 truncate font-mono text-2xs text-muted">{d.statement}</pre>
            </li>
          ))}
        </ul>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted">
            Type <span className="selectable rounded bg-panel px-1 font-mono font-semibold text-fg">{phrase}</span> to confirm
            {conn && (
              <>
                {" "}on <span className="font-medium text-fg">{conn.name}</span>
              </>
            )}
          </span>
          <Input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} className="font-mono" />
        </label>
      </form>
    </Modal>
  );
}
