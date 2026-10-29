// Typed bridge to the Rust side. When running in a plain browser (`pnpm dev`
// without Tauri) a small in-memory mock is used so the UI can be designed fast.

export type DbKind = "postgres" | "mysql" | "sqlite";

export interface SshConfig {
  host: string;
  port?: number | null;
  user: string;
  auth: "key" | "password";
  keyPath?: string | null;
  /** pinned SHA256 fingerprint (trust on first use) */
  hostKey?: string | null;
}

export interface ConnectionConfig {
  id: string;
  name: string;
  kind: DbKind;
  host?: string | null;
  port?: number | null;
  user?: string | null;
  database?: string | null;
  file?: string | null;
  readOnly: boolean;
  mcpEnabled: boolean;
  color?: string | null;
  ssh?: SshConfig | null;
}

export interface TableInfo {
  schema: string;
  name: string;
  kind: "table" | "view";
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
  primary_key: boolean;
}

export type Cell = string | number | boolean | null | object;

export interface QueryResult {
  columns: { name: string; data_type: string }[];
  rows: Cell[][];
  rows_affected: number;
  truncated: boolean;
}

export interface QueryOutput {
  results: QueryResult[];
  elapsed_ms: number;
}

export interface ChangeSet {
  schema: string;
  table: string;
  updates: { key: Record<string, Cell>; set: Record<string, Cell> }[];
  inserts: Record<string, Cell>[];
  deletes: Record<string, Cell>[];
}

export interface McpInfo {
  command: string;
  configPath: string;
  snippet: unknown;
}

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  return mock(cmd, args ?? {}) as Promise<T>;
}

export const api = {
  listConnections: () => call<ConnectionConfig[]>("list_connections"),
  saveConnection: (config: ConnectionConfig, password?: string, sshSecret?: string) =>
    call<ConnectionConfig>("save_connection", { config, password, sshSecret }),
  deleteConnection: (id: string) => call<void>("delete_connection", { id }),
  /** resolves to the SSH host-key fingerprint when tunnelled */
  testConnection: (config: ConnectionConfig, password?: string, sshSecret?: string) =>
    call<string | null>("test_connection", { config, password, sshSecret }),
  connect: (id: string) => call<void>("connect", { id }),
  disconnect: (id: string) => call<void>("disconnect", { id }),
  listTables: (id: string) => call<TableInfo[]>("list_tables", { id }),
  describeTable: (id: string, schema: string, table: string) =>
    call<ColumnInfo[]>("describe_table", { id, schema, table }),
  tableSql: (id: string, schema: string, table: string, limit = 300) =>
    call<string>("table_sql", { id, schema, table, limit }),
  runQuery: (id: string, sql: string, limit?: number) =>
    call<QueryOutput>("run_query", { id, sql, limit }),
  previewChanges: (id: string, changes: ChangeSet) => call<string[]>("preview_changes", { id, changes }),
  applyChanges: (id: string, changes: ChangeSet) => call<number>("apply_changes", { id, changes }),
  exportQuery: (id: string, sql: string, format: ExportFormat, path: string, schema?: string, table?: string) =>
    call<number>("export_query", { id, sql, format, path, schema, table }),
  tableSqlAll: (id: string, schema: string, table: string) => call<string>("table_sql_all", { id, schema, table }),
  importCsv: (id: string, schema: string, table: string, path: string) =>
    call<number>("import_csv", { id, schema, table, path }),
  mcpInfo: () => call<McpInfo>("mcp_info"),
};

export type ExportFormat = "csv" | "json" | "sql";

/** Native save / open dialogs (mocked in the browser). */
export const dialogs = {
  async save(defaultName: string, ext: string): Promise<string | null> {
    if (!isTauri) return `/mock/${defaultName}`;
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: defaultName, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  },
  async openCsv(): Promise<string | null> {
    if (!isTauri) return "/mock/import.csv";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: false, filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }] });
    return typeof r === "string" ? r : null;
  },
};

export function newConnection(kind: DbKind = "postgres"): ConnectionConfig {
  return {
    id: crypto.randomUUID(),
    name: "",
    kind,
    host: kind === "sqlite" ? null : "localhost",
    port: kind === "postgres" ? 5432 : kind === "mysql" ? 3306 : null,
    user: kind === "postgres" ? "postgres" : kind === "mysql" ? "root" : null,
    database: null,
    file: null,
    readOnly: false,
    mcpEnabled: true,
    color: null,
    ssh: null,
  };
}

// ---------------------------------------------------------------------------
// Browser mock
// ---------------------------------------------------------------------------

const mockConns: ConnectionConfig[] = [
  { ...newConnection("postgres"), id: "local-pg", name: "Local Postgres", database: "app_dev", color: "green" },
  { ...newConnection("mysql"), id: "prod-my", name: "Production MySQL", host: "db.internal", database: "shop", readOnly: true, color: "red" },
  { ...newConnection("sqlite"), id: "lite", name: "analytics.db", file: "~/data/analytics.db" },
];

const mockTables: TableInfo[] = [
  ...["users", "orders", "order_items", "products", "invoices", "sessions", "audit_log"].map((name) => ({
    schema: "public",
    name,
    kind: "table" as const,
  })),
  { schema: "public", name: "v_monthly_revenue", kind: "view" },
  { schema: "billing", name: "subscriptions", kind: "table" },
  { schema: "billing", name: "plans", kind: "table" },
];

const first = ["Ayu", "Budi", "Citra", "Dimas", "Eka", "Fajar", "Gita", "Hana", "Indra", "Joko", "Kirana", "Lukas"];
const cities = ["Jakarta", "Bandung", "Surabaya", "Yogyakarta", "Denpasar", "Medan", "Makassar"];

function mockRows(n: number): Cell[][] {
  return Array.from({ length: n }, (_, i) => {
    const name = first[i % first.length];
    return [
      i + 1,
      `${name} ${String.fromCharCode(65 + (i % 26))}.`,
      `${name.toLowerCase()}${i}@example.com`,
      cities[i % cities.length],
      i % 7 === 0 ? null : (Math.round(Math.sin(i) * 50000 + 60000) / 100).toFixed(2),
      i % 3 !== 0,
      i % 5 === 0 ? { plan: "pro", seats: (i % 9) + 1 } : null,
      new Date(Date.UTC(2026, i % 12, (i % 27) + 1, i % 24, i % 60)).toISOString().replace("T", " ").slice(0, 19),
    ];
  });
}

const mockCols: ColumnInfo[] = [
  { name: "id", data_type: "bigint", nullable: false, default: "nextval('users_id_seq')", primary_key: true },
  { name: "full_name", data_type: "text", nullable: false, default: null, primary_key: false },
  { name: "email", data_type: "citext", nullable: false, default: null, primary_key: false },
  { name: "city", data_type: "varchar(64)", nullable: true, default: null, primary_key: false },
  { name: "balance", data_type: "numeric(12,2)", nullable: true, default: "0", primary_key: false },
  { name: "active", data_type: "boolean", nullable: false, default: "true", primary_key: false },
  { name: "meta", data_type: "jsonb", nullable: true, default: null, primary_key: false },
  { name: "created_at", data_type: "timestamptz", nullable: false, default: "now()", primary_key: false },
];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function mock(cmd: string, a: Record<string, unknown>): Promise<unknown> {
  await sleep(cmd === "run_query" ? 180 : 60);
  switch (cmd) {
    case "list_connections":
      return mockConns;
    case "save_connection": {
      const c = a.config as ConnectionConfig;
      const i = mockConns.findIndex((x) => x.id === c.id);
      if (i >= 0) mockConns[i] = c;
      else mockConns.push(c);
      return c;
    }
    case "delete_connection": {
      const i = mockConns.findIndex((x) => x.id === a.id);
      if (i >= 0) mockConns.splice(i, 1);
      return null;
    }
    case "list_tables":
      return mockTables;
    case "describe_table":
      return mockCols;
    case "table_sql":
      return `SELECT * FROM "${a.schema}"."${a.table}" LIMIT ${a.limit}`;
    case "run_query": {
      const sql = String(a.sql).trim().toLowerCase();
      if (sql.startsWith("select") || sql.startsWith("with")) {
        return {
          elapsed_ms: 12 + Math.round(Math.random() * 30),
          results: [
            {
              columns: mockCols.map((c) => ({ name: c.name, data_type: c.data_type.toUpperCase() })),
              rows: mockRows(300),
              rows_affected: 300,
              truncated: false,
            },
          ],
        } satisfies QueryOutput;
      }
      if (sql.startsWith("err")) throw 'database error: syntax error at or near "err"';
      return { elapsed_ms: 4, results: [{ columns: [], rows: [], rows_affected: 3, truncated: false }] };
    }
    case "preview_changes": {
      const c = a.changes as ChangeSet;
      const t = `"${c.schema}"."${c.table}"`;
      const lit = (v: Cell) => (v === null ? "NULL" : typeof v === "string" ? `'${v.replace(/'/g, "''")}'` : JSON.stringify(v));
      const where = (k: Record<string, Cell>) => Object.entries(k).map(([n, v]) => `"${n}" = ${lit(v)}`).join(" AND ");
      return [
        ...c.updates.map((u) => `UPDATE ${t} SET ${Object.entries(u.set).map(([n, v]) => `"${n}" = ${lit(v)}`).join(", ")} WHERE ${where(u.key)}`),
        ...c.inserts.map((r) => `INSERT INTO ${t} (${Object.keys(r).map((n) => `"${n}"`).join(", ")}) VALUES (${Object.values(r).map(lit).join(", ")})`),
        ...c.deletes.map((k) => `DELETE FROM ${t} WHERE ${where(k)}`),
      ];
    }
    case "apply_changes": {
      const c = a.changes as ChangeSet;
      return c.updates.length + c.inserts.length + c.deletes.length;
    }
    case "test_connection":
      return (a.config as ConnectionConfig).ssh ? "SHA256:GFCDfRT4H3B+37G+mjBEKq0ZZhRZCUZ7H1027f1NUUU" : null;
    case "export_query":
      return 300;
    case "table_sql_all":
      return `SELECT * FROM "${a.schema}"."${a.table}"`;
    case "import_csv":
      return 1200;
    case "mcp_info":
      return {
        command: "/Applications/Rowhouse.app/Contents/MacOS/rowhouse-mcp",
        configPath: "~/Library/Application Support/id.morfotech.rowhouse/connections.json",
        snippet: { mcpServers: { rowhouse: { command: "/Applications/Rowhouse.app/Contents/MacOS/rowhouse-mcp" } } },
      };
    default:
      return null;
  }
}
