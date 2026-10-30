import { Braces } from "lucide-react";
import { useEffect, useState } from "react";
import { substitute, type VarRef } from "../lib/vars";
import { Button, Kbd, Modal } from "./ui";

const KEY = "rh-vars-v1";
export function recentVars(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}
function remember(v: Record<string, string>) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...recentVars(), ...v }));
  } catch {
    /* storage unavailable */
  }
}

/** Asks for `{{name}}` / `:name` values before a query runs. */
export function VariablesDialog({
  request,
  dialect,
  onRun,
  onClose,
}: {
  request: { sql: string; vars: VarRef[]; initial: Record<string, string> } | null;
  dialect: string;
  onRun: (values: Record<string, string>, sql: string) => void;
  onClose: () => void;
}) {
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!request) return;
    const recent = recentVars();
    setValues(Object.fromEntries(request.vars.map((v) => [v.name, request.initial[v.name] ?? recent[v.name] ?? ""])));
  }, [request]);
  if (!request) return null;
  const preview = substitute(request.sql, values, dialect);
  const submit = () => {
    remember(values);
    onRun(values, preview);
  };
  return (
    <Modal
      open
      onClose={onClose}
      width={560}
      title={
        <span className="flex items-center gap-2">
          <Braces size={14} className="text-accent" /> Query variables
        </span>
      }
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">
            <code className="font-mono">:name</code> becomes a quoted value · <code className="font-mono">{"{{name}}"}</code> is pasted as written
          </span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit}>
            Run <Kbd>↵</Kbd>
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        {request.vars.map((v, i) => (
          <label key={v.name} className="grid grid-cols-[140px_1fr] items-center gap-3">
            <span className="truncate text-right font-mono text-xs text-muted">{v.raw ? `{{${v.name}}}` : `:${v.name}`}</span>
            <input
              autoFocus={i === 0}
              value={values[v.name] ?? ""}
              onChange={(e) => setValues((x) => ({ ...x, [v.name]: e.target.value }))}
              spellCheck={false}
              placeholder={v.raw ? "raw SQL text" : "value"}
              className="h-7 w-full rounded-md border border-line bg-elevated px-2 font-mono text-xs outline-none focus:border-accent focus:ring-2 focus:ring-accent-soft"
            />
          </label>
        ))}
        <pre className="selectable mt-2 max-h-40 overflow-auto rounded-lg border border-line bg-panel p-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">{preview}</pre>
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}
