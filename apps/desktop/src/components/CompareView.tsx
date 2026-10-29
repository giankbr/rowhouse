import { ArrowRight, Check, Copy, GitCompareArrows, Loader2, Minus, PencilLine, Plus, SquareTerminal } from "lucide-react";
import { useEffect, useState } from "react";
import { api, kindInfo, type ConnectionConfig, type SchemaChange, type SchemaDiff } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, cx } from "./ui";

function useSchemas(connId: string) {
  const [schemas, setSchemas] = useState<string[]>([]);
  useEffect(() => {
    if (!connId) return;
    api
      .listTables(connId)
      .then((t) => setSchemas([...new Set(t.map((x) => x.schema))]))
      .catch(() => setSchemas([]));
  }, [connId]);
  return schemas;
}

function Side({
  label,
  connections,
  connId,
  schema,
  onConn,
  onSchema,
}: {
  label: string;
  connections: ConnectionConfig[];
  connId: string;
  schema: string;
  onConn: (id: string) => void;
  onSchema: (s: string) => void;
}) {
  const schemas = useSchemas(connId);
  useEffect(() => {
    if (schemas.length && !schemas.includes(schema)) onSchema(schemas.includes("public") ? "public" : schemas[0]);
  }, [schemas]); // eslint-disable-line react-hooks/exhaustive-deps
  const conn = connections.find((c) => c.id === connId);
  const sel = "h-7 rounded-md border border-line bg-elevated px-1.5 text-xs outline-none focus:border-accent";
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1">
      <span className="text-[10.5px] font-medium tracking-wide text-subtle uppercase">{label}</span>
      <div className="flex items-center gap-1.5">
        {conn && <KindBadge kind={conn.kind} size={18} />}
        <select value={connId} onChange={(e) => onConn(e.target.value)} className={cx(sel, "min-w-0 flex-1")}>
          {connections.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
        <select value={schema} onChange={(e) => onSchema(e.target.value)} className={cx(sel, "w-32")}>
          {schemas.map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
      </div>
    </div>
  );
}

const META: Record<SchemaChange["kind"], { icon: typeof Plus; cls: string; text: string }> = {
  tableAdded: { icon: Plus, cls: "text-success", text: "create table" },
  columnAdded: { icon: Plus, cls: "text-success", text: "add column" },
  columnChanged: { icon: PencilLine, cls: "text-warning", text: "change" },
  columnRemoved: { icon: Minus, cls: "text-danger", text: "only in target" },
  tableRemoved: { icon: Minus, cls: "text-danger", text: "only in target" },
};

function describe(c: SchemaChange) {
  switch (c.kind) {
    case "tableAdded":
      return `${c.columns.length} columns`;
    case "columnAdded":
      return `${c.column.name} ${c.column.data_type}${c.column.nullable ? "" : " NOT NULL"}`;
    case "columnChanged":
      return c.aspects
        .map((a) =>
          a === "type"
            ? `${c.column}: ${c.from.data_type} → ${c.to.data_type}`
            : a === "nullable"
              ? `${c.column}: ${c.from.nullable ? "NULL" : "NOT NULL"} → ${c.to.nullable ? "NULL" : "NOT NULL"}`
              : `${c.column}: default ${c.from.default ?? "none"} → ${c.to.default ?? "none"}`,
        )
        .join(" · ");
    case "columnRemoved":
      return c.column;
    case "tableRemoved":
      return "table";
  }
}

export function CompareView({
  connections,
  currentId,
  onOpenSql,
  onToast,
}: {
  connections: ConnectionConfig[];
  currentId: string;
  onOpenSql: (connId: string, sql: string) => void;
  onToast: (msg: string, kind?: "ok" | "err") => void;
}) {
  // default source: another connection of the same engine (e.g. staging vs production)
  const current = connections.find((c) => c.id === currentId);
  const dialect = (c?: ConnectionConfig) => (c ? kindInfo(c.kind).dialect : undefined);
  const others = connections.filter((c) => c.id !== currentId);
  const preferred = others.find((c) => dialect(c) === dialect(current)) ?? others[0];
  const [source, setSource] = useState({ id: preferred?.id ?? currentId, schema: "public" });
  const [target, setTarget] = useState({ id: currentId, schema: "public" });
  const [diff, setDiff] = useState<SchemaDiff | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  const run = async () => {
    setBusy(true);
    try {
      setDiff(await api.compareSchema(source.id, source.schema, target.id, target.schema));
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  const byTable = new Map<string, SchemaChange[]>();
  diff?.changes.forEach((c) => byTable.set(c.table, [...(byTable.get(c.table) ?? []), c]));
  const count = (k: SchemaChange["kind"][]) => diff?.changes.filter((c) => k.includes(c.kind)).length ?? 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-end gap-3 border-b border-line px-3 py-2.5">
        <Side label="Source (desired)" connections={connections} connId={source.id} schema={source.schema} onConn={(id) => setSource({ id, schema: "" })} onSchema={(schema) => setSource((s) => ({ ...s, schema }))} />
        <ArrowRight size={14} className="mb-2 shrink-0 text-subtle" />
        <Side label="Target (to update)" connections={connections} connId={target.id} schema={target.schema} onConn={(id) => setTarget({ id, schema: "" })} onSchema={(schema) => setTarget((s) => ({ ...s, schema }))} />
        <Button variant="primary" onClick={run} disabled={busy || !source.schema || !target.schema}>
          {busy ? <Loader2 size={12} className="animate-spin" /> : <GitCompareArrows size={12} />} Compare
        </Button>
      </div>

      {!diff ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-xs text-subtle">
          <GitCompareArrows size={20} className="text-accent" />
          Pick two schemas — Rowhouse lists what differs and writes the migration for the target.
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="flex w-[46%] min-w-72 flex-col border-r border-line">
            <div className="flex h-8 shrink-0 items-center gap-3 border-b border-line bg-panel/60 px-3 text-2xs">
              <span className="text-success">+{count(["tableAdded", "columnAdded"])} add</span>
              <span className="text-warning">~{count(["columnChanged"])} change</span>
              <span className="text-danger">−{count(["tableRemoved", "columnRemoved"])} only in target</span>
              <span className="ml-auto text-subtle">{diff.identical.length} identical</span>
            </div>
            {diff.crossDialect && (
              <div className="border-b border-line bg-warning/10 px-3 py-1.5 text-2xs text-warning">
                Different engines — type names may not line up; review the script carefully.
              </div>
            )}
            <div className="flex-1 overflow-y-auto">
              {diff.changes.length === 0 && (
                <div className="flex h-full items-center justify-center gap-1.5 text-xs text-success">
                  <Check size={14} /> Schemas match
                </div>
              )}
              {[...byTable.entries()].map(([table, items]) => (
                <div key={table} className="border-b border-line/60 px-3 py-2">
                  <div className="mb-1 font-mono text-xs font-semibold">{table}</div>
                  {items.map((c, i) => {
                    const m = META[c.kind];
                    const Icon = m.icon;
                    return (
                      <div key={i} className="flex items-start gap-1.5 py-0.5 text-xs">
                        <Icon size={12} className={cx("mt-0.5 shrink-0", m.cls)} />
                        <span className={cx("shrink-0 text-2xs", m.cls)}>{m.text}</span>
                        <span className="selectable min-w-0 font-mono text-[11px] break-all text-muted">{describe(c)}</span>
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line bg-panel/60 px-3 text-2xs text-muted">
              <span className="font-medium text-fg">Migration for target</span>
              <span className="flex-1" />
              <Button
                variant="ghost"
                size="xs"
                onClick={() => {
                  navigator.clipboard.writeText(diff.sql);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                }}
              >
                {copied ? <Check size={11} /> : <Copy size={11} />} {copied ? "Copied" : "Copy"}
              </Button>
              <Button size="xs" onClick={() => onOpenSql(target.id, diff.sql)}>
                <SquareTerminal size={11} /> Open in query tab
              </Button>
            </div>
            <pre className="selectable flex-1 overflow-auto p-3 font-mono text-[11.5px] leading-relaxed">
              {diff.sql.split("\n").map((l, i) => (
                <div key={i} className={cx(l.startsWith("--") ? "text-subtle" : l.startsWith("CREATE") || l.includes(" ADD ") ? "text-success" : "")}>
                  {l || " "}
                </div>
              ))}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
