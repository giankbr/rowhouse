import { Check, Copy, KeyRound, Plus, SquareTerminal, Trash2 } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { api, kindInfo, type ColumnInfo, type ConnectionConfig, type TableAction, type TableInfo } from "../lib/api";
import { Button, Input, Modal, cx } from "./ui";

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

export type MenuItem =
  | { label: ReactNode; hint?: string; danger?: boolean; disabled?: boolean; onSelect: () => void }
  | "separator";

/** Right-click menu at viewport coordinates; flips to stay on screen. */
export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - r.width - 6)),
      y: Math.max(4, y + r.height > window.innerHeight - 6 ? y - r.height : y),
    });
  }, [x, y]);
  useEffect(() => {
    const close = (e: Event) => !ref.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", onClose);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("keydown", key);
    };
  }, [onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      className="animate-pop fixed z-50 min-w-52 rounded-lg border border-line bg-elevated p-1 text-xs shadow-pop"
      style={{ left: pos.x, top: pos.y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((it, i) =>
        it === "separator" ? (
          <div key={i} className="mx-1 my-1 h-px bg-line" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={it.disabled}
            onClick={() => (onClose(), it.onSelect())}
            className={cx(
              "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover disabled:pointer-events-none disabled:opacity-40",
              it.danger && "text-danger hover:bg-danger/10",
            )}
          >
            {it.label}
            {it.hint && <span className="ml-auto pl-4 text-2xs text-subtle">{it.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Rename / duplicate / truncate / drop
// ---------------------------------------------------------------------------

export type TableActionKind = TableAction["op"];

const TITLES: Record<TableActionKind, string> = {
  rename: "Rename",
  duplicate: "Duplicate",
  truncate: "Truncate",
  drop: "Drop",
};

export function TableActionDialog({
  conn,
  table,
  kind,
  onClose,
  onDone,
  onToast,
}: {
  conn: ConnectionConfig;
  table: TableInfo | null;
  kind: TableActionKind | null;
  onClose: () => void;
  onDone: (msg: string, renamedTo?: string) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [name, setName] = useState("");
  const [withData, setWithData] = useState(true);
  const [confirm, setConfirm] = useState("");
  const [sql, setSql] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const open = !!table && !!kind;
  const isView = table?.kind === "view";
  const destructive = kind === "drop" || kind === "truncate";

  useEffect(() => {
    if (!table || !kind) return;
    setName(kind === "duplicate" ? `${table.name}_copy` : kind === "rename" ? table.name : "");
    setWithData(true);
    setConfirm("");
    setSql(null);
    setError(null);
  }, [table, kind]);

  const action = (): TableAction | null => {
    if (!kind) return null;
    if (kind === "rename") return { op: "rename", newName: name.trim() };
    if (kind === "duplicate") return { op: "duplicate", newName: name.trim(), withData };
    return { op: kind };
  };

  // live preview of the statements
  useEffect(() => {
    const a = action();
    if (!open || !a || !table) return;
    const t = setTimeout(() => {
      api
        .tableAction(conn.id, table.schema, table.name, isView, a, true)
        .then((s) => (setSql(s), setError(null)))
        .catch((e) => (setSql(null), setError(String(e))));
    }, 150);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, name, withData, kind, table, conn.id]);

  const ready = !!sql && !error && (!destructive || confirm === table?.name) && !conn.readOnly;

  const run = async () => {
    const a = action();
    if (!a || !table || !ready) return;
    setBusy(true);
    try {
      await api.tableAction(conn.id, table.schema, table.name, isView, a, false);
      const what = isView ? "View" : "Table";
      onDone(
        kind === "drop"
          ? `${what} ${table.name} dropped`
          : kind === "truncate"
            ? `${table.name} emptied`
            : kind === "rename"
              ? `Renamed to ${name.trim()}`
              : `Created ${name.trim()}`,
        kind === "rename" ? name.trim() : undefined,
      );
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) return null;
  return (
    <Modal
      open
      onClose={onClose}
      width={520}
      title={`${TITLES[kind!]} ${isView ? "view" : "table"} · ${table!.name}`}
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">{conn.readOnly ? "This connection is read-only" : destructive ? "This can't be undone" : ""}</span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant={destructive ? "default" : "primary"} className={cx(destructive && "border-danger/40 text-danger hover:bg-danger/10")} disabled={!ready || busy} onClick={run}>
            {busy ? "Working…" : TITLES[kind!]}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          run();
        }}
      >
        {(kind === "rename" || kind === "duplicate") && (
          <label className="flex flex-col gap-1 text-2xs text-muted">
            New name
            <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} onFocus={(e) => e.target.select()} />
          </label>
        )}
        {kind === "duplicate" && (
          <label className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={withData} onChange={(e) => setWithData(e.target.checked)} className="accent-[var(--accent)]" />
            Copy the rows too
          </label>
        )}
        {destructive && (
          <>
            <p className="text-xs text-muted">
              {kind === "drop"
                ? `Deletes the ${isView ? "view" : "table and all its rows"} permanently.`
                : "Deletes every row; the table and its structure stay."}{" "}
              Type <span className="selectable font-mono font-medium text-fg">{table!.name}</span> to confirm.
            </p>
            <Input autoFocus value={confirm} onChange={(e) => setConfirm(e.target.value)} placeholder={table!.name} />
          </>
        )}
        {error ? (
          <div className="rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">{error}</div>
        ) : (
          <pre className="selectable max-h-40 overflow-auto rounded-lg border border-line bg-panel p-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">
            {sql ? sql.map((s) => s.replace(/;?\s*$/, ";")).join("\n") : " "}
          </pre>
        )}
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// DDL viewer
// ---------------------------------------------------------------------------

export function DdlDialog({
  conn,
  table,
  onClose,
  onOpenInQuery,
}: {
  conn: ConnectionConfig;
  table: TableInfo | null;
  onClose: () => void;
  onOpenInQuery: (sql: string) => void;
}) {
  const [ddl, setDdl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!table) return;
    setDdl(null);
    setError(null);
    setCopied(false);
    api
      .tableDdl(conn.id, table.schema, table.name)
      .then(setDdl)
      .catch((e) => setError(String(e)));
  }, [conn.id, table]);
  if (!table) return null;
  return (
    <Modal
      open
      onClose={onClose}
      width={720}
      title={`DDL · ${table.name}`}
      footer={
        <>
          <span className="flex-1" />
          <Button
            variant="ghost"
            disabled={!ddl}
            onClick={() => {
              navigator.clipboard.writeText(ddl!);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
          >
            {copied ? <Check size={12} /> : <Copy size={12} />} {copied ? "Copied" : "Copy"}
          </Button>
          <Button variant="ghost" disabled={!ddl} onClick={() => (onOpenInQuery(ddl!), onClose())}>
            <SquareTerminal size={12} /> Open in query tab
          </Button>
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      {error ? (
        <div className="selectable rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-2xs text-danger">{error}</div>
      ) : (
        <pre className="selectable -mx-4 -my-3 min-h-32 overflow-auto px-4 py-3 font-mono text-[11.5px] leading-relaxed whitespace-pre text-fg/90">
          {ddl ?? <span className="text-subtle">Loading…</span>}
        </pre>
      )}
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// New table
// ---------------------------------------------------------------------------

const STARTER: Record<string, { id: string; text: string; ts: string; tsDefault: string }> = {
  postgres: { id: "bigint GENERATED BY DEFAULT AS IDENTITY", text: "text", ts: "timestamptz", tsDefault: "now()" },
  mysql: { id: "bigint AUTO_INCREMENT", text: "varchar(255)", ts: "datetime", tsDefault: "CURRENT_TIMESTAMP" },
  sqlite: { id: "INTEGER", text: "TEXT", ts: "TEXT", tsDefault: "CURRENT_TIMESTAMP" },
  duckdb: { id: "INTEGER", text: "VARCHAR", ts: "TIMESTAMP", tsDefault: "current_timestamp" },
  mssql: { id: "bigint IDENTITY(1,1)", text: "nvarchar(255)", ts: "datetime2", tsDefault: "SYSUTCDATETIME()" },
  clickhouse: { id: "UInt64", text: "String", ts: "DateTime", tsDefault: "now()" },
  cql: { id: "uuid", text: "text", ts: "timestamp", tsDefault: "" },
  oracle: { id: "NUMBER GENERATED BY DEFAULT AS IDENTITY", text: "VARCHAR2(255)", ts: "TIMESTAMP", tsDefault: "SYSTIMESTAMP" },
};

const TYPES: Record<string, string[]> = {
  postgres: ["text", "varchar(255)", "integer", "bigint", "numeric(12,2)", "boolean", "timestamptz", "date", "jsonb", "uuid"],
  mysql: ["varchar(255)", "text", "int", "bigint", "decimal(12,2)", "tinyint(1)", "datetime", "date", "json"],
  sqlite: ["TEXT", "INTEGER", "REAL", "BLOB", "NUMERIC"],
  duckdb: ["VARCHAR", "INTEGER", "BIGINT", "DOUBLE", "DECIMAL(12,2)", "BOOLEAN", "TIMESTAMP", "DATE", "JSON"],
  mssql: ["nvarchar(255)", "nvarchar(max)", "int", "bigint", "decimal(12,2)", "bit", "datetime2", "date", "uniqueidentifier"],
  clickhouse: ["String", "Int32", "Int64", "UInt64", "Float64", "Decimal(12,2)", "Bool", "DateTime", "Date", "Nullable(String)"],
  cql: ["text", "int", "bigint", "uuid", "timeuuid", "timestamp", "boolean", "double", "decimal", "map<text, text>"],
  oracle: ["VARCHAR2(255)", "NUMBER(10)", "NUMBER(12,2)", "DATE", "TIMESTAMP", "CLOB", "BLOB", "NVARCHAR2(255)", "CHAR(1)"],
};

function starter(dialect: string): ColumnInfo[] {
  const s = STARTER[dialect] ?? STARTER.postgres;
  const cols: ColumnInfo[] = [
    { name: "id", data_type: s.id, nullable: false, default: null, primary_key: true },
    { name: "name", data_type: s.text, nullable: true, default: null, primary_key: false },
  ];
  if (s.tsDefault) cols.push({ name: "created_at", data_type: s.ts, nullable: false, default: s.tsDefault, primary_key: false });
  return cols;
}

export function NewTableDialog({
  conn,
  open,
  schemas,
  defaultSchema,
  onClose,
  onCreated,
  onToast,
}: {
  conn: ConnectionConfig;
  open: boolean;
  schemas: string[];
  defaultSchema: string;
  onClose: () => void;
  onCreated: (t: TableInfo) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const dialect = kindInfo(conn.kind).dialect;
  const [schema, setSchema] = useState(defaultSchema);
  const [name, setName] = useState("");
  const [cols, setCols] = useState<ColumnInfo[]>(() => starter(dialect));
  const [sql, setSql] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setSchema(defaultSchema);
    setName("");
    setCols(starter(dialect));
    setError(null);
  }, [open, defaultSchema, dialect]);

  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => {
      api
        .createTable(conn.id, schema, name.trim() || "new_table", cols, true)
        .then((s) => (setSql(s), setError(null)))
        .catch((e) => (setSql(""), setError(String(e))));
    }, 150);
    return () => clearTimeout(t);
  }, [open, conn.id, schema, name, cols]);

  const patch = (i: number, p: Partial<ColumnInfo>) => setCols((cs) => cs.map((c, j) => (j === i ? { ...c, ...p } : c)));

  const create = async () => {
    if (!name.trim() || error) return;
    setBusy(true);
    try {
      await api.createTable(conn.id, schema, name.trim(), cols, false);
      onCreated({ schema, name: name.trim(), kind: "table" });
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) return null;
  const cell = "h-7 rounded-md border border-transparent bg-transparent px-1.5 text-xs outline-none hover:border-line focus:border-accent focus:bg-elevated";
  return (
    <Modal
      open
      onClose={onClose}
      width={760}
      title="New table"
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">{conn.readOnly ? "This connection is read-only" : ""}</span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!name.trim() || !!error || busy || conn.readOnly} onClick={create}>
            {busy ? "Creating…" : "Create table"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex gap-2">
          {schemas.length > 1 && (
            <label className="flex w-44 flex-col gap-1 text-2xs text-muted">
              Schema
              <select value={schema} onChange={(e) => setSchema(e.target.value)} className="h-7 rounded-md border border-line bg-elevated px-1.5 text-xs text-fg outline-none">
                {schemas.map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </select>
            </label>
          )}
          <label className="flex flex-1 flex-col gap-1 text-2xs text-muted">
            Table name
            <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="customers" />
          </label>
        </div>

        <div className="rounded-lg border border-line">
          <div className="grid grid-cols-[1.1fr_1.4fr_56px_1.1fr_40px_28px] items-center gap-1 border-b border-line bg-panel px-2 py-1 text-2xs text-subtle">
            <span>Column</span>
            <span>Type</span>
            <span className="text-center">Null</span>
            <span>Default</span>
            <span className="text-center">PK</span>
            <span />
          </div>
          {cols.map((c, i) => (
            <div key={i} className="grid grid-cols-[1.1fr_1.4fr_56px_1.1fr_40px_28px] items-center gap-1 border-b border-line/60 px-1 py-0.5 last:border-0">
              <input className={cx(cell, "font-medium")} value={c.name} spellCheck={false} onChange={(e) => patch(i, { name: e.target.value })} placeholder="column" />
              <input className={cx(cell, "font-mono text-[11.5px]")} value={c.data_type} spellCheck={false} list={`rh-nt-types-${dialect}`} onChange={(e) => patch(i, { data_type: e.target.value })} />
              <span className="flex justify-center">
                <input type="checkbox" checked={c.nullable} disabled={c.primary_key} onChange={(e) => patch(i, { nullable: e.target.checked })} className="accent-[var(--accent)]" />
              </span>
              <input className={cx(cell, "font-mono text-[11.5px]")} value={c.default ?? ""} spellCheck={false} placeholder="—" onChange={(e) => patch(i, { default: e.target.value || null })} />
              <span className="flex justify-center">
                <button
                  title="Primary key"
                  onClick={() => patch(i, { primary_key: !c.primary_key, nullable: c.primary_key ? c.nullable : false })}
                  className={cx("rounded p-1", c.primary_key ? "text-warning" : "text-subtle/50 hover:text-muted")}
                >
                  <KeyRound size={12} />
                </button>
              </span>
              <Button size="icon" variant="ghost" title="Remove column" onClick={() => setCols((cs) => cs.filter((_, j) => j !== i))}>
                <Trash2 size={12} />
              </Button>
            </div>
          ))}
          <datalist id={`rh-nt-types-${dialect}`}>
            {(TYPES[dialect] ?? TYPES.postgres).map((t) => (
              <option key={t} value={t} />
            ))}
          </datalist>
          <button
            onClick={() => setCols((cs) => [...cs, { name: "", data_type: (TYPES[dialect] ?? TYPES.postgres)[0], nullable: true, default: null, primary_key: false }])}
            className="flex h-7 w-full items-center gap-1.5 px-2.5 text-2xs text-muted hover:bg-hover hover:text-fg"
          >
            <Plus size={12} /> Add column
          </button>
        </div>

        {error ? (
          <div className="rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">{error}</div>
        ) : (
          <pre className="selectable max-h-40 overflow-auto rounded-lg border border-line bg-panel p-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre text-muted">{sql || " "}</pre>
        )}
      </div>
    </Modal>
  );
}
