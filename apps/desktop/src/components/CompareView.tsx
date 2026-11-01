import { ArrowRight, Check, Copy, GitCompareArrows, Loader2, Minus, PencilLine, Plus, SquareTerminal } from "lucide-react";
import { useEffect, useState } from "react";
import { api, kindInfo, type Cell, type ConnectionConfig, type DataDiff, type RowDiff, type SchemaChange, type SchemaDiff } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Segmented, cx } from "./ui";

function useSchemas(connId: string) {
  const [tables, setTables] = useState<{ schema: string; name: string; kind: string }[]>([]);
  useEffect(() => {
    if (!connId) return;
    api
      .listTables(connId)
      .then(setTables)
      .catch(() => setTables([]));
  }, [connId]);
  return { schemas: [...new Set(tables.map((x) => x.schema))], tables };
}

function Side({
  label,
  connections,
  connId,
  schema,
  onConn,
  onSchema,
  table,
  onTable,
}: {
  label: string;
  connections: ConnectionConfig[];
  connId: string;
  schema: string;
  onConn: (id: string) => void;
  onSchema: (s: string) => void;
  /** data mode: pick a table too */
  table?: string;
  onTable?: (t: string) => void;
}) {
  const { schemas, tables } = useSchemas(connId);
  const tableNames = tables.filter((t) => t.schema === schema && t.kind === "table").map((t) => t.name);
  useEffect(() => {
    if (onTable && tableNames.length && !tableNames.includes(table ?? "")) onTable(tableNames[0]);
  }, [tableNames.join(), onTable]); // eslint-disable-line react-hooks/exhaustive-deps
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
        <select value={schema} onChange={(e) => onSchema(e.target.value)} className={cx(sel, "w-28")}>
          {schemas.map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        {onTable && (
          <select value={table} onChange={(e) => onTable(e.target.value)} className={cx(sel, "w-36 font-mono")}>
            {tableNames.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        )}
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

export function describeChange(c: SchemaChange) {
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
  const [mode, setMode] = useState<"schema" | "data">("schema");
  const [tables, setTables] = useState({ source: "", target: "" });
  const [data, setData] = useState<DataDiff | null>(null);
  const [diff, setDiff] = useState<SchemaDiff | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  const run = async () => {
    setBusy(true);
    try {
      if (mode === "data") setData(await api.compareData(source.id, source.schema, tables.source, target.id, target.schema, tables.target));
      else setDiff(await api.compareSchema(source.id, source.schema, target.id, target.schema));
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
        <div className="flex flex-col gap-1">
          <span className="text-[10.5px] font-medium tracking-wide text-subtle uppercase">Compare</span>
          <Segmented
            value={mode}
            onChange={setMode}
            options={[
              { value: "schema", label: "Schema" },
              { value: "data", label: "Data" },
            ]}
          />
        </div>
        <Side
          label="Source (desired)"
          connections={connections}
          connId={source.id}
          schema={source.schema}
          onConn={(id) => setSource({ id, schema: "" })}
          onSchema={(schema) => setSource((s) => ({ ...s, schema }))}
          table={mode === "data" ? tables.source : undefined}
          onTable={mode === "data" ? (t) => setTables((x) => ({ ...x, source: t, target: x.target || t })) : undefined}
        />
        <ArrowRight size={14} className="mb-2 shrink-0 text-subtle" />
        <Side
          label="Target (to update)"
          connections={connections}
          connId={target.id}
          schema={target.schema}
          onConn={(id) => setTarget({ id, schema: "" })}
          onSchema={(schema) => setTarget((s) => ({ ...s, schema }))}
          table={mode === "data" ? tables.target : undefined}
          onTable={mode === "data" ? (t) => setTables((x) => ({ ...x, target: t })) : undefined}
        />
        <Button variant="primary" onClick={run} disabled={busy || !source.schema || !target.schema || (mode === "data" && (!tables.source || !tables.target))}>
          {busy ? <Loader2 size={12} className="animate-spin" /> : <GitCompareArrows size={12} />} Compare
        </Button>
      </div>

      {mode === "data" ? (
        data ? (
          <DataResult data={data} onOpenSql={() => onOpenSql(target.id, data.sql)} />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 text-xs text-subtle">
            <GitCompareArrows size={20} className="text-accent" />
            Pick two tables — rows are matched by the target's primary key (up to 200,000 per side).
          </div>
        )
      ) : !diff ? (
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
                        <span className="selectable min-w-0 font-mono text-[11px] break-all text-muted">{describeChange(c)}</span>
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

const show = (v: Cell) => (v === null ? "NULL" : typeof v === "object" ? JSON.stringify(v) : String(v));
const keyText = (r: RowDiff) => Object.entries(r.key).map(([k, v]) => `${k}=${show(v)}`).join(", ");

function DataResult({ data, onOpenSql }: { data: DataDiff; onOpenSql: () => void }) {
  const [copied, setCopied] = useState(false);
  const stat = (n: number, label: string, cls: string) => (
    <div className="flex flex-col rounded-lg border border-line px-3 py-2">
      <span className={cx("text-base font-semibold tabular-nums", cls)}>{n.toLocaleString()}</span>
      <span className="text-[10.5px] text-subtle">{label}</span>
    </div>
  );
  const section = (title: string, rows: RowDiff[], total: number, cls: string, render?: (r: RowDiff) => React.ReactNode) =>
    total > 0 && (
      <div className="border-b border-line/60 px-3 py-2">
        <div className={cx("mb-1 text-xs font-medium", cls)}>
          {title} <span className="text-subtle">({total.toLocaleString()}{rows.length < total ? `, first ${rows.length} shown` : ""})</span>
        </div>
        {rows.map((r, i) => (
          <div key={i} className="py-0.5 font-mono text-[11px]">
            <span className="text-muted">{keyText(r)}</span>
            {render?.(r)}
          </div>
        ))}
      </div>
    );
  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex w-[46%] min-w-72 flex-col border-r border-line">
        <div className="grid shrink-0 grid-cols-4 gap-2 border-b border-line p-3">
          {stat(data.onlyInSource, "to insert", "text-success")}
          {stat(data.changed, "to update", "text-warning")}
          {stat(data.onlyInTarget, "only in target", "text-danger")}
          {stat(data.identical, "identical", "text-muted")}
        </div>
        <div className="border-b border-line px-3 py-1.5 text-2xs text-subtle">
          key: <span className="font-mono">{data.keyColumns.join(", ")}</span> · {data.sourceRows.toLocaleString()} source rows ·{" "}
          {data.targetRows.toLocaleString()} target rows
          {data.truncated && <span className="text-warning"> · capped at 200,000 rows per side</span>}
        </div>
        <div className="flex-1 overflow-y-auto">
          {data.onlyInSource + data.changed + data.onlyInTarget === 0 && (
            <div className="flex h-full items-center justify-center gap-1.5 text-xs text-success">
              <Check size={14} /> Tables match
            </div>
          )}
          {section("Changed", data.samplesChanged, data.changed, "text-warning", (r) => (
            <div className="pl-3">
              {Object.entries(r.changes).map(([c, [from, to]]) => (
                <div key={c}>
                  {c}: <span className="text-danger line-through">{show(from)}</span> → <span className="text-success">{show(to)}</span>
                </div>
              ))}
            </div>
          ))}
          {section("Only in source → insert", data.samplesOnlyInSource, data.onlyInSource, "text-success")}
          {section("Only in target", data.samplesOnlyInTarget, data.onlyInTarget, "text-danger")}
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line bg-panel/60 px-3 text-2xs text-muted">
          <span className="font-medium text-fg">Sync script for target</span>
          <span className="flex-1" />
          <Button
            variant="ghost"
            size="xs"
            onClick={() => {
              navigator.clipboard.writeText(data.sql);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
          >
            {copied ? <Check size={11} /> : <Copy size={11} />} {copied ? "Copied" : "Copy"}
          </Button>
          <Button size="xs" onClick={onOpenSql}>
            <SquareTerminal size={11} /> Open in query tab
          </Button>
        </div>
        <pre className="selectable flex-1 overflow-auto p-3 font-mono text-[11.5px] leading-relaxed">
          {data.sql.split("\n").map((l, i) => (
            <div key={i} className={cx(l.startsWith("--") ? "text-subtle" : l.startsWith("INSERT") ? "text-success" : l.startsWith("UPDATE") ? "text-warning" : "")}>
              {l || " "}
            </div>
          ))}
        </pre>
      </div>
    </div>
  );
}
