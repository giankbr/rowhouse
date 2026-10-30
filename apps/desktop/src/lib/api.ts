// Typed bridge to the Rust side. When running in a plain browser (`pnpm dev`
// without Tauri) a small in-memory mock is used so the UI can be designed fast.

export type DbKind =
  | "postgres"
  | "neon"
  | "supabase"
  | "cockroach"
  | "redshift"
  | "greenplum"
  | "mysql"
  | "mariadb"
  | "tidb"
  | "singlestore"
  | "sqlite"
  | "turso"
  | "d1"
  | "duckdb"
  | "mssql"
  | "clickhouse"
  | "redis"
  | "upstash"
  | "mongodb"
  | "cassandra"
  | "scylla";

export type Dialect = "postgres" | "mysql" | "sqlite" | "duckdb" | "mssql" | "clickhouse" | "redis" | "mongodb" | "cql";

export interface KindInfo {
  kind: DbKind;
  label: string;
  abbr: string;
  color: string;
  dialect: Dialect;
  port: number | null;
  user: string | null;
  /** embedded file database */
  file?: boolean;
  /** shown with a "beta" tag until tested against a real server in CI */
  beta?: boolean;
  /** hosted HTTP API: no host/port, TLS or SSH settings */
  api?: boolean;
  /** TLS required by default (hosted services) */
  tls?: boolean;
}

export const KINDS: KindInfo[] = [
  { kind: "postgres", label: "PostgreSQL", abbr: "Pg", color: "#336791", dialect: "postgres", port: 5432, user: "postgres" },
  { kind: "neon", label: "Neon", abbr: "Ne", color: "#00b386", dialect: "postgres", port: 5432, user: null, tls: true },
  { kind: "supabase", label: "Supabase", abbr: "Sb", color: "#249361", dialect: "postgres", port: 5432, user: "postgres", tls: true },
  { kind: "mysql", label: "MySQL", abbr: "My", color: "#e48e00", dialect: "mysql", port: 3306, user: "root" },
  { kind: "mariadb", label: "MariaDB", abbr: "Ma", color: "#4b8b9b", dialect: "mysql", port: 3306, user: "root" },
  { kind: "sqlite", label: "SQLite", abbr: "Sl", color: "#7c3aed", dialect: "sqlite", port: null, user: null, file: true },
  { kind: "turso", label: "Turso", abbr: "Tu", color: "#13a88a", dialect: "sqlite", port: null, user: null, api: true },
  { kind: "d1", label: "Cloudflare D1", abbr: "D1", color: "#f38020", dialect: "sqlite", port: null, user: null, api: true },
  { kind: "mssql", label: "SQL Server", abbr: "Ss", color: "#a91d22", dialect: "mssql", port: 1433, user: "sa", beta: true },
  { kind: "duckdb", label: "DuckDB", abbr: "Du", color: "#1f1f1f", dialect: "duckdb", port: null, user: null, file: true },
  { kind: "clickhouse", label: "ClickHouse", abbr: "Ch", color: "#d4b106", dialect: "clickhouse", port: 8123, user: "default" },
  { kind: "redis", label: "Redis", abbr: "Re", color: "#b3261e", dialect: "redis", port: 6379, user: null },
  { kind: "upstash", label: "Upstash", abbr: "Up", color: "#00c98d", dialect: "redis", port: 6379, user: "default", tls: true },
  { kind: "mongodb", label: "MongoDB", abbr: "Mo", color: "#00684a", dialect: "mongodb", port: 27017, user: null, beta: true },
  { kind: "cassandra", label: "Cassandra", abbr: "Ca", color: "#1287b1", dialect: "cql", port: 9042, user: null, beta: true },
  { kind: "scylla", label: "ScyllaDB", abbr: "Sc", color: "#6c2bd9", dialect: "cql", port: 9042, user: null, beta: true },
  { kind: "cockroach", label: "CockroachDB", abbr: "Cr", color: "#6933ff", dialect: "postgres", port: 26257, user: "root", beta: true },
  { kind: "redshift", label: "Redshift", abbr: "Rs", color: "#205b97", dialect: "postgres", port: 5439, user: "awsuser", beta: true },
  { kind: "greenplum", label: "Greenplum", abbr: "Gp", color: "#3f8f3f", dialect: "postgres", port: 5432, user: "gpadmin", beta: true },
  { kind: "tidb", label: "TiDB", abbr: "Ti", color: "#dc150b", dialect: "mysql", port: 4000, user: "root", beta: true },
  { kind: "singlestore", label: "SingleStore", abbr: "S2", color: "#aa00ff", dialect: "mysql", port: 3306, user: "root", beta: true },
];

export const kindInfo = (k: DbKind): KindInfo => KINDS.find((x) => x.kind === k) ?? KINDS[0];

export interface SshConfig {
  host: string;
  port?: number | null;
  user: string;
  auth: "key" | "password" | "agent";
  keyPath?: string | null;
  /** custom agent socket / pipe (e.g. 1Password); empty = system default */
  agentSocket?: string | null;
  /** pinned SHA256 fingerprint (trust on first use) */
  hostKey?: string | null;
}

export type TlsMode = "disable" | "prefer" | "require" | "verify-ca" | "verify-full";
export interface TlsConfig {
  mode: TlsMode;
  caCert?: string | null;
  clientCert?: string | null;
  clientKey?: string | null;
}

export type Environment = "none" | "local" | "development" | "staging" | "production";

export const ENVIRONMENTS: { value: Environment; label: string; color: string | null }[] = [
  { value: "none", label: "None", color: null },
  { value: "local", label: "Local", color: "gray" },
  { value: "development", label: "Dev", color: "blue" },
  { value: "staging", label: "Staging", color: "orange" },
  { value: "production", label: "Production", color: "red" },
];

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
  /** MCP clients may request writes (each needs approval) */
  mcpWrites?: boolean;
  color?: string | null;
  ssh?: SshConfig | null;
  env?: Environment;
  tls?: TlsConfig | null;
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

export interface HistoryEntry {
  id: number;
  connectionId: string;
  sql: string;
  source: "app" | "mcp";
  ok: boolean;
  error: string | null;
  rows: number;
  elapsedMs: number;
  at: number;
}

export interface AiSettings {
  enabled: boolean;
  provider: "openai" | "anthropic";
  baseUrl: string;
  model: string;
  inlineCompletions?: boolean;
}

export interface AiAnswer {
  sql: string;
  explanation: string;
}

export interface Destructive {
  kind: "drop" | "truncate" | "delete-all" | "update-all" | "drop-column";
  object: string;
  statement: string;
}

export interface ErdTable {
  schema: string;
  name: string;
  kind: string;
  columns: ColumnInfo[];
}
export interface ForeignKey {
  name: string;
  schema: string;
  table: string;
  columns: string[];
  ref_schema: string;
  ref_table: string;
  ref_columns: string[];
}
export interface Erd {
  tables: ErdTable[];
  foreign_keys: ForeignKey[];
  truncated: boolean;
}

export type ImportSource = "dbeaver" | "tableplus" | "navicat";
export interface ImportCandidate {
  config: ConnectionConfig;
  skippedReason: string | null;
}

export type FilterOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains" | "starts-with" | "is-null" | "not-null" | "in";
export interface Filter {
  column: string;
  op: FilterOp;
  value: string;
  numeric: boolean;
}
export interface PageRequest {
  schema: string;
  table: string;
  filters: Filter[];
  sort: { column: string; desc: boolean } | null;
  limit: number;
  offset: number;
  count: boolean;
}
export interface Page {
  result: QueryResult;
  total: number | null;
  sql: string;
  elapsed_ms: number;
}

export interface RowDiff {
  key: Record<string, Cell>;
  changes: Record<string, [Cell, Cell]>;
}
export interface DataDiff {
  keyColumns: string[];
  comparedColumns: string[];
  sourceRows: number;
  targetRows: number;
  onlyInSource: number;
  onlyInTarget: number;
  changed: number;
  identical: number;
  samplesOnlyInSource: RowDiff[];
  samplesOnlyInTarget: RowDiff[];
  samplesChanged: RowDiff[];
  truncated: boolean;
  sql: string;
}

export type ColumnOp =
  | { op: "add"; column: ColumnInfo }
  | { op: "drop"; name: string }
  | { op: "rename"; from: string; to: string }
  | { op: "alter"; from: ColumnInfo; to: ColumnInfo };

export interface Snippet {
  id: number;
  name: string;
  sql: string;
  connectionId: string | null;
  updatedAt: number;
}

export interface DetectedImport {
  source: ImportSource;
  path: string;
  /** importable connections in the file */
  count: number;
}

export type Every =
  | { kind: "minutes"; n: number }
  | { kind: "daily"; at: string }
  | { kind: "weekly"; weekday: number; at: string };
export type Notify = "never" | "on-failure" | "on-change" | "always";

export interface ScheduleRun {
  at: number;
  ok: boolean;
  rows: number;
  elapsedMs: number;
  error?: string | null;
  changed: boolean;
  file?: string | null;
}

export interface Schedule {
  id: string;
  name: string;
  connectionId: string;
  sql: string;
  every: Every;
  enabled: boolean;
  export: { dir: string; format: ExportFormat } | null;
  notify: Notify;
  createdAt: number;
  runs: ScheduleRun[];
}

/** Mirrors `Schedule::next_run` in rowhouse-core (unix seconds). */
export function nextRun(s: Schedule): number | null {
  if (!s.enabled) return null;
  const since = Math.max(s.runs[0]?.at ?? 0, s.createdAt);
  const e = s.every;
  if (e.kind === "minutes") return since + Math.max(1, e.n) * 60;
  const [h, m] = e.at.split(":").map(Number);
  const base = new Date(since * 1000);
  const c = new Date(base);
  c.setHours(h, m, 0, 0);
  if (e.kind === "weekly") {
    const today = (base.getDay() + 6) % 7;
    c.setDate(c.getDate() + ((e.weekday - today + 7) % 7));
    if (c <= base) c.setDate(c.getDate() + 7);
  } else if (c <= base) c.setDate(c.getDate() + 1);
  return Math.floor(c.getTime() / 1000);
}

export function describeEvery(e: Every): string {
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  if (e.kind === "minutes") return e.n === 1 ? "every minute" : e.n % 60 === 0 ? `every ${e.n / 60} h` : `every ${e.n} min`;
  if (e.kind === "daily") return `daily at ${e.at}`;
  return `${days[e.weekday]} at ${e.at}`;
}

export interface PlanNode {
  label: string;
  detail: string | null;
  cost: number | null;
  rows: number | null;
  actual_ms: number | null;
  actual_rows: number | null;
  loops: number | null;
  warnings: string[];
  children: PlanNode[];
}
export interface Plan {
  nodes: PlanNode[];
  raw: string;
  analyzed: boolean;
  execution_ms: number | null;
  planning_ms: number | null;
}

export type SchemaChange =
  | { kind: "tableAdded"; table: string; columns: ColumnInfo[] }
  | { kind: "tableRemoved"; table: string }
  | { kind: "columnAdded"; table: string; column: ColumnInfo }
  | { kind: "columnRemoved"; table: string; column: string }
  | { kind: "columnChanged"; table: string; column: string; from: ColumnInfo; to: ColumnInfo; aspects: string[] };
export interface SchemaDiff {
  changes: SchemaChange[];
  identical: string[];
  sql: string;
  crossDialect: boolean;
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

/** A Tauri IPC channel that forwards streamed text to `fn` (a plain callback in the browser mock). */
async function channel(fn?: (s: string) => void): Promise<unknown> {
  if (!isTauri) return fn;
  const { Channel } = await import("@tauri-apps/api/core");
  const ch = new Channel<string>();
  ch.onmessage = (s) => fn?.(s);
  return ch;
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
  runQuery: (id: string, sql: string, limit?: number, runId?: string, timeoutMs?: number) =>
    call<QueryOutput>("run_query", { id, sql, limit, runId, timeoutMs }),
  cancelQuery: (id: string, runId: string) => call<void>("cancel_query", { id, runId }),
  previewChanges: (id: string, changes: ChangeSet) => call<string[]>("preview_changes", { id, changes }),
  applyChanges: (id: string, changes: ChangeSet) => call<number>("apply_changes", { id, changes }),
  exportQuery: (id: string, sql: string, format: ExportFormat, path: string, schema?: string, table?: string) =>
    call<number>("export_query", { id, sql, format, path, schema, table }),
  tableSqlAll: (id: string, schema: string, table: string) => call<string>("table_sql_all", { id, schema, table }),
  importCsv: (id: string, schema: string, table: string, path: string) =>
    call<number>("import_csv", { id, schema, table, path }),
  listHistory: (id?: string, search?: string, limit = 200) =>
    call<HistoryEntry[]>("list_history", { id, search, limit }),
  clearHistory: (id?: string) => call<void>("clear_history", { id }),
  mcpInfo: () => call<McpInfo>("mcp_info"),
  writeTextFile: (path: string, contents: string) => call<void>("write_text_file", { path, contents }),
  parseConnectionUrl: (url: string) => call<{ config: ConnectionConfig; password: string | null }>("parse_connection_url", { url }),
  tablePage: (id: string, request: PageRequest) => call<Page>("table_page", { id, request }),
  compareData: (sourceId: string, sourceSchema: string, sourceTable: string, targetId: string, targetSchema: string, targetTable: string) =>
    call<DataDiff>("compare_data", { sourceId, sourceSchema, sourceTable, targetId, targetSchema, targetTable }),
  listSnippets: (id?: string, search?: string) => call<Snippet[]>("list_snippets", { id, search }),
  saveSnippet: (snippet: Snippet) => call<Snippet>("save_snippet", { snippet }),
  schedules: {
    list: () => call<Schedule[]>("schedules_list"),
    save: (schedule: Schedule) => call<Schedule>("schedule_save", { schedule }),
    remove: (scheduleId: string) => call<void>("schedule_delete", { scheduleId }),
    runNow: (scheduleId: string) => call<ScheduleRun>("schedule_run_now", { scheduleId }),
    /** Fires after every run (scheduled or manual). */
    async onRun(fn: (e: { schedule: Schedule; run: ScheduleRun }) => void): Promise<() => void> {
      if (!isTauri) {
        mockScheduleListeners.add(fn);
        return () => mockScheduleListeners.delete(fn);
      }
      const { listen } = await import("@tauri-apps/api/event");
      return listen<{ schedule: Schedule; run: ScheduleRun }>("schedule-run", (ev) => fn(ev.payload));
    },
  },
  deleteSnippet: (snippetId: number) => call<void>("delete_snippet", { snippetId }),
  explainPlan: (id: string, sql: string, analyze: boolean) => call<Plan>("explain_plan", { id, sql, analyze }),
  compareSchema: (sourceId: string, sourceSchema: string, targetId: string, targetSchema: string) =>
    call<SchemaDiff>("compare_schema", { sourceId, sourceSchema, targetId, targetSchema }),
  structurePreview: (id: string, schema: string, table: string, ops: ColumnOp[]) =>
    call<string[]>("structure_preview", { id, schema, table, ops }),
  applyStructure: (id: string, schema: string, table: string, ops: ColumnOp[]) =>
    call<string[]>("apply_structure", { id, schema, table, ops }),
  approvals: {
    pending: () => call<import("../components/ApprovalDialog").ApprovalRequest[]>("approvals_pending"),
    answer: (id: string, approved: boolean, reason?: string) => call<void>("approvals_answer", { id, approved, reason }),
  },
  checkDestructive: (sql: string) => call<Destructive[]>("check_destructive", { sql }),
  erd: (id: string, schema?: string) => call<Erd>("erd", { id, schema }),
  importDetect: () => call<DetectedImport[]>("import_detect"),
  importParse: (path: string) => call<{ source: ImportSource; candidates: ImportCandidate[] }>("import_parse", { path }),
  importSave: (configs: ConnectionConfig[]) => call<ConnectionConfig[]>("import_save", { configs }),
  ai: {
    getSettings: () => call<{ settings: AiSettings; hasKey: boolean }>("ai_get_settings"),
    saveSettings: (settings: AiSettings, apiKey?: string) => call<void>("ai_save_settings", { settings, apiKey }),
    listModels: (settings: AiSettings, apiKey?: string) => call<string[]>("ai_list_models", { settings, apiKey }),
    test: (settings: AiSettings, apiKey?: string) => call<string>("ai_test", { settings, apiKey }),
    generate: async (id: string, prompt: string, currentSql?: string, onDelta?: (s: string) => void) =>
      call<AiAnswer>("ai_generate", { id, prompt, currentSql, onDelta: await channel(onDelta) }),
    fix: async (id: string, sql: string, error: string, onDelta?: (s: string) => void) =>
      call<AiAnswer>("ai_fix", { id, sql, error, onDelta: await channel(onDelta) }),
    explain: async (id: string, sql: string, onDelta?: (s: string) => void) =>
      call<AiAnswer>("ai_explain", { id, sql, onDelta: await channel(onDelta) }),
    edit: async (id: string, instruction: string, selection: string, before: string, after: string, onDelta?: (s: string) => void) =>
      call<AiAnswer>("ai_edit", { id, instruction, selection, before, after, onDelta: await channel(onDelta) }),
    completeInline: (id: string, before: string, after: string) => call<string>("ai_complete_inline", { id, before, after }),
  },
};

export type ExportFormat = "csv" | "json" | "sql";

/** Native save / open dialogs (mocked in the browser). */
export const dialogs = {
  async save(defaultName: string, ext: string): Promise<string | null> {
    if (!isTauri) return `/mock/${defaultName}`;
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: defaultName, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  },
  async openAny(name: string, extensions: string[]): Promise<string | null> {
    if (!isTauri) return "/mock/connections.ncx";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: false, filters: [{ name, extensions }] });
    return typeof r === "string" ? r : null;
  },
  async folder(): Promise<string | null> {
    if (!isTauri) return "/Users/you/Reports";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: true });
    return typeof r === "string" ? r : null;
  },
  async openCsv(): Promise<string | null> {
    if (!isTauri) return "/mock/import.csv";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: false, filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }] });
    return typeof r === "string" ? r : null;
  },
};

export function newConnection(kind: DbKind = "postgres"): ConnectionConfig {
  const k = kindInfo(kind);
  return {
    id: crypto.randomUUID(),
    name: "",
    kind,
    host: k.file || k.api ? null : "localhost",
    port: k.port,
    user: k.user,
    database: null,
    file: null,
    readOnly: false,
    mcpEnabled: true,
    color: null,
    ssh: null,
    env: "none",
    tls: k.tls ? { mode: "require" } : null,
  };
}

// ---------------------------------------------------------------------------
// Browser mock
// ---------------------------------------------------------------------------

const mockCancelled = new Set<string>();
const mockScheduleListeners = new Set<(e: { schedule: Schedule; run: ScheduleRun }) => void>();
const nowS = () => Math.floor(Date.now() / 1000);
let mockSchedules: Schedule[] = [
  {
    id: "s1",
    name: "Daily revenue",
    connectionId: "local-pg",
    sql: "SELECT date_trunc('day', created_at) AS day, sum(total) AS revenue\nFROM orders WHERE created_at > now() - interval '7 days'\nGROUP BY 1 ORDER BY 1;",
    every: { kind: "daily", at: "08:30" },
    enabled: true,
    export: { dir: "/Users/you/Reports", format: "csv" },
    notify: "on-change",
    createdAt: nowS() - 86400 * 3,
    runs: [
      { at: nowS() - 3600 * 5, ok: true, rows: 7, elapsedMs: 42, changed: true, file: "/Users/you/Reports/Daily-revenue-20261030-083000.csv" },
      { at: nowS() - 3600 * 29, ok: true, rows: 7, elapsedMs: 38, changed: false, file: "/Users/you/Reports/Daily-revenue-20261029-083000.csv" },
    ],
  },
  {
    id: "s2",
    name: "Failed payments watch",
    connectionId: "prod-my",
    sql: "SELECT id, user_id, amount FROM payments WHERE status = 'failed' AND created_at > NOW() - INTERVAL 15 MINUTE;",
    every: { kind: "minutes", n: 15 },
    enabled: true,
    export: null,
    notify: "on-change",
    createdAt: nowS() - 86400,
    runs: [
      { at: nowS() - 600, ok: false, rows: 0, elapsedMs: 3010, changed: false, error: "connection timed out" },
      { at: nowS() - 1500, ok: true, rows: 2, elapsedMs: 21, changed: false },
    ],
  },
];

const mockConns: ConnectionConfig[] = typeof location !== "undefined" && location.search.includes("empty") ? [] : [
  { ...newConnection("postgres"), id: "local-pg", name: "Local Postgres", database: "app_dev", color: "green" },
  { ...newConnection("mysql"), id: "prod-my", name: "Production MySQL", host: "db.internal", database: "shop", readOnly: true, color: "red", env: "production" },
  { ...newConnection("sqlite"), id: "lite", name: "analytics.db", file: "~/data/analytics.db" },
  { ...newConnection("clickhouse"), id: "ch", name: "Events (ClickHouse)", database: "events", color: "yellow" },
  { ...newConnection("redis"), id: "redis-local", name: "redis-local", host: "127.0.0.1", env: "local" },
  { ...newConnection("mongodb"), id: "mongo-shop", name: "shop (MongoDB)", host: "127.0.0.1", database: "shop", env: "development" },
  { ...newConnection("postgres"), id: "staging-pg", name: "Billing (staging)", host: "staging.db.internal", database: "billing", env: "staging", ssh: { host: "bastion.internal", port: 22, user: "deploy", auth: "agent" } },
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

let mockSnippets: Snippet[] = [
  { id: 1, name: "Revenue by month", sql: "SELECT date_trunc('month', created_at) AS month, sum(total)\nFROM orders WHERE status = 'paid'\nGROUP BY 1 ORDER BY 1;", connectionId: "local-pg", updatedAt: Math.floor(Date.now() / 1000) - 3600 },
  { id: 2, name: "Table sizes", sql: "SELECT relname, pg_size_pretty(pg_total_relation_size(relid))\nFROM pg_catalog.pg_statio_user_tables ORDER BY 2 DESC;", connectionId: null, updatedAt: Math.floor(Date.now() / 1000) - 86400 },
];

const mockAi: { settings: AiSettings; hasKey: boolean } = {
  settings: { enabled: true, provider: "openai", baseUrl: "https://api.openai.com/v1", model: "gpt-mock", inlineCompletions: true },
  hasKey: true,
};

async function mock(cmd: string, a: Record<string, unknown>): Promise<unknown> {
  await sleep(cmd === "run_query" ? 180 : 60);
  switch (cmd) {
    case "list_connections":
      return [...mockConns];
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
    case "cancel_query":
      mockCancelled.add(String(a.runId));
      return null;
    case "run_query": {
      const sql = String(a.sql).trim().toLowerCase();
      if (/sleep\(/.test(sql)) {
        // long query for trying cancel / timeout in the preview
        const until = Date.now() + 8000;
        const limitAt = a.timeoutMs ? Date.now() + Number(a.timeoutMs) : Infinity;
        while (Date.now() < until) {
          await sleep(100);
          if (mockCancelled.has(String(a.runId))) throw "query cancelled";
          if (Date.now() > limitAt) throw `query timed out after ${Number(a.timeoutMs) / 1000} s (Settings → Query timeout)`;
        }
      }
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
    case "list_history": {
      const now = Math.floor(Date.now() / 1000);
      const all: HistoryEntry[] = [
        { sql: "SELECT id, full_name, email\nFROM users\nWHERE active = true\nORDER BY created_at DESC;", source: "app", rows: 214, ms: 18, ago: 40 },
        { sql: "SELECT count(*) FROM orders WHERE status = 'paid'", source: "mcp", rows: 1, ms: 7, ago: 300 },
        { sql: "SELECT * FROM invoices WHERE due_at < now()", source: "app", rows: 0, ms: 4, ago: 1800, error: 'relation "invoices" has no column "due_at"' },
        { sql: "UPDATE users SET active = false WHERE last_login < '2025-01-01'", source: "app", rows: 37, ms: 22, ago: 7200 },
        { sql: "SELECT date_trunc('month', created_at) m, sum(total) FROM orders GROUP BY 1 ORDER BY 1", source: "mcp", rows: 12, ms: 31, ago: 86400 },
      ].map((e, i) => ({
        id: 100 - i, connectionId: String(a.id ?? "local-pg"), sql: e.sql, source: e.source as "app" | "mcp",
        ok: !e.error, error: e.error ?? null, rows: e.rows, elapsedMs: e.ms, at: now - e.ago,
      }));
      const q = String(a.search ?? "").toLowerCase();
      return all.filter((e) => !q || e.sql.toLowerCase().includes(q));
    }
    case "clear_history":
      return null;
    case "ai_get_settings":
      return mockAi;
    case "ai_save_settings":
      mockAi.settings = a.settings as AiSettings;
      if (a.apiKey !== undefined) mockAi.hasKey = !!a.apiKey;
      return null;
    case "ai_list_models":
      return ["gpt-mock", "gpt-mock-mini", "local-llama"];
    case "ai_test":
      return "OK";
    case "ai_complete_inline": {
      await sleep(250);
      const before = String(a.before);
      if (/from\s*$/i.test(before)) return "users u JOIN orders o ON o.user_id = u.id";
      if (/select\s*$/i.test(before)) return "u.id, u.full_name, sum(o.total) AS revenue ";
      if (/where\s*$/i.test(before)) return "o.status = 'paid'";
      return "";
    }
    case "ai_generate":
      for (const piece of ["```sql\nSELECT u.id, u.full_name,", " SUM(o.total) AS revenue\nFROM users u", "\nJOIN orders o ON …"]) {
        await sleep(200);
        (a.onDelta as ((s: string) => void) | undefined)?.(piece);
      }
      return {
        sql: "SELECT u.id, u.full_name, SUM(o.total) AS revenue\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE o.status = 'paid'\nGROUP BY u.id, u.full_name\nORDER BY revenue DESC\nLIMIT 10;",
        explanation: "Top 10 customers by paid order revenue.",
      } satisfies AiAnswer;
    case "ai_edit": {
      const sel = String(a.selection);
      const ins = String(a.instruction).toLowerCase();
      let sql = sel
        ? ins.includes("upper")
          ? sel.toUpperCase()
          : `${sel.replace(/;\s*$/, "")}\nORDER BY created_at DESC\nLIMIT 50;`
        : "SELECT u.id, u.email, count(o.id) AS orders\nFROM users u\nLEFT JOIN orders o ON o.user_id = u.id\nGROUP BY u.id, u.email;";
      for (const piece of ["```sql\n", sql.slice(0, 20), sql.slice(20), "\n```"]) {
        await sleep(150);
        (a.onDelta as ((s: string) => void) | undefined)?.(piece);
      }
      return { sql, explanation: sel ? "Added ordering and a limit." : "" } satisfies AiAnswer;
    }
    case "ai_fix":
      await sleep(500);
      return {
        sql: String(a.sql).replace(/full_nam\b/g, "full_name").replace(/^err\w*/i, "SELECT"),
        explanation: "The column is called full_name.",
      } satisfies AiAnswer;
    case "ai_explain":
      await sleep(400);
      return {
        sql: "",
        explanation:
          "Lists active users with their email, newest signups first. It scans users once; an index on (active, created_at) would avoid a sort on large tables.",
      } satisfies AiAnswer;
    case "parse_connection_url": {
      const u = new URL(String(a.url).replace(/^jdbc:/, ""));
      const host = u.hostname;
      const kind: DbKind = host.endsWith("neon.tech") ? "neon" : host.includes("supabase") ? "supabase" : u.protocol.startsWith("mysql") ? "mysql" : "postgres";
      const db = decodeURIComponent(u.pathname.replace(/^\//, "")) || null;
      return {
        config: {
          ...newConnection(kind),
          name: kind === "neon" ? `Neon · ${db ?? "neondb"}` : kind === "supabase" ? `Supabase · ${host.split(".")[0]}` : `${db}@${host}`,
          host, port: u.port ? Number(u.port) : kindInfo(kind).port, user: decodeURIComponent(u.username) || null, database: db,
          tls: u.searchParams.get("sslmode") || kindInfo(kind).tls ? { mode: (u.searchParams.get("sslmode") as TlsMode) ?? "require" } : null,
        },
        password: decodeURIComponent(u.password) || null,
      };
    }
    case "table_page": {
      await sleep(120);
      const r = a.request as PageRequest;
      let rows = mockRows(1234);
      const names = mockCols.map((c) => c.name);
      for (const f of r.filters) {
        const i = names.indexOf(f.column);
        if (i < 0) continue;
        const v = f.value.toLowerCase();
        rows = rows.filter((row) => {
          const x = row[i];
          const s = x === null ? "" : String(typeof x === "object" ? JSON.stringify(x) : x).toLowerCase();
          switch (f.op) {
            case "is-null": return x === null;
            case "not-null": return x !== null;
            case "contains": return s.includes(v);
            case "starts-with": return s.startsWith(v);
            case "ne": return s !== v;
            case "gt": return Number(x) > Number(v);
            case "lt": return Number(x) < Number(v);
            case "ge": return Number(x) >= Number(v);
            case "le": return Number(x) <= Number(v);
            case "in": return v.split(",").map((y) => y.trim()).includes(s);
            default: return s === v;
          }
        });
      }
      if (r.sort) {
        const i = names.indexOf(r.sort.column), d = r.sort.desc ? -1 : 1;
        rows = [...rows].sort((p, q) => (String(p[i] ?? "") < String(q[i] ?? "") ? -d : d));
        if (["id", "balance"].includes(r.sort.column)) rows = [...rows].sort((p, q) => (Number(p[i] ?? 0) - Number(q[i] ?? 0)) * d);
      }
      const where = r.filters.length ? " WHERE " + r.filters.map((f) => `"${f.column}" ${f.op} ${f.value}`).join(" AND ") : "";
      return {
        result: { columns: mockCols.map((c) => ({ name: c.name, data_type: c.data_type.toUpperCase() })), rows: rows.slice(r.offset, r.offset + r.limit), rows_affected: 0, truncated: false },
        total: rows.length,
        sql: `SELECT * FROM "${r.schema}"."${r.table}"${where}${r.sort ? ` ORDER BY "${r.sort.column}" ${r.sort.desc ? "DESC" : "ASC"}` : ""} LIMIT ${r.limit}${r.offset ? ` OFFSET ${r.offset}` : ""}`,
        elapsed_ms: 9,
      } satisfies Page;
    }
    case "compare_data": {
      await sleep(300);
      return {
        keyColumns: ["id"], comparedColumns: ["id", "full_name", "email", "city", "balance", "active"],
        sourceRows: 1240, targetRows: 1236, onlyInSource: 6, onlyInTarget: 2, changed: 3, identical: 1231,
        samplesOnlyInSource: [1237, 1238, 1239, 1240, 1241, 1242].map((id) => ({ key: { id }, changes: {} })),
        samplesOnlyInTarget: [88, 412].map((id) => ({ key: { id }, changes: {} })),
        samplesChanged: [
          { key: { id: 7 }, changes: { email: ["gita6@example.com", "gita@new-mail.id"] } },
          { key: { id: 15 }, changes: { balance: ["1054.65", "980.00"], active: [true, false] } },
          { key: { id: 301 }, changes: { city: ["Medan", "Bandung"] } },
        ],
        truncated: false,
        sql: `-- Generated by Rowhouse data compare: make users match the source.
-- Deletes are commented out on purpose. Review before running.
UPDATE "public"."users" SET "email" = 'gita@new-mail.id' WHERE "id" = 7;
UPDATE "public"."users" SET "active" = FALSE, "balance" = '980.00' WHERE "id" = 15;
UPDATE "public"."users" SET "city" = 'Bandung' WHERE "id" = 301;
INSERT INTO "public"."users" ("id", "full_name", "email") VALUES (1237, 'Rina P.', 'rina@example.com');
-- DELETE FROM "public"."users" WHERE "id" = 88;
-- DELETE FROM "public"."users" WHERE "id" = 412;`,
      } satisfies DataDiff;
    }
    case "schedules_list":
      return structuredClone(mockSchedules);
    case "schedule_save": {
      const sc = { ...(a.schedule as Schedule) };
      if (!sc.name.trim()) throw "give the schedule a name";
      if (/^\s*(drop|truncate)\b|^\s*delete\s+from\s+\S+\s*;?\s*$/i.test(sc.sql)) throw "scheduled queries can't run destructive statements";
      const cur = mockSchedules.find((x) => x.id === sc.id);
      if (!sc.id) sc.id = crypto.randomUUID();
      sc.runs = cur?.runs ?? [];
      sc.createdAt = cur && JSON.stringify(cur.every) === JSON.stringify(sc.every) ? cur.createdAt : nowS();
      mockSchedules = cur ? mockSchedules.map((x) => (x.id === sc.id ? sc : x)) : [...mockSchedules, sc];
      return sc;
    }
    case "schedule_delete":
      mockSchedules = mockSchedules.filter((x) => x.id !== a.scheduleId);
      return null;
    case "schedule_run_now": {
      await sleep(400);
      const sc = mockSchedules.find((x) => x.id === a.scheduleId)!;
      const run: ScheduleRun = { at: nowS(), ok: true, rows: 7, elapsedMs: 35, changed: false, file: sc.export ? `${sc.export.dir}/${sc.name.replace(/\W+/g, "-")}-now.${sc.export.format}` : null };
      sc.runs = [run, ...sc.runs].slice(0, 20);
      mockScheduleListeners.forEach((f) => f({ schedule: structuredClone(sc), run }));
      return run;
    }
    case "list_snippets": {
      const q = String(a.search ?? "").toLowerCase();
      return mockSnippets.filter((x) => (!x.connectionId || x.connectionId === a.id) && (!q || x.name.toLowerCase().includes(q) || x.sql.toLowerCase().includes(q)));
    }
    case "save_snippet": {
      const sn = a.snippet as Snippet;
      const saved = { ...sn, id: sn.id || Math.max(0, ...mockSnippets.map((x) => x.id)) + 1, updatedAt: Math.floor(Date.now() / 1000) };
      mockSnippets = [saved, ...mockSnippets.filter((x) => x.id !== saved.id)];
      return saved;
    }
    case "delete_snippet":
      mockSnippets = mockSnippets.filter((x) => x.id !== a.snippetId);
      return null;
    case "explain_plan": {
      await sleep(250);
      const an = !!a.analyze;
      const n = (label: string, detail: string | null, cost: number, rows: number, ms: number, children: PlanNode[] = [], warnings: string[] = []): PlanNode => ({
        label, detail, cost, rows, actual_ms: an ? ms : null, actual_rows: an ? rows * 0.9 : null, loops: an ? 1 : null, warnings, children,
      });
      return {
        nodes: [
          n("Limit", null, 1840.2, 10, 48.1, [
            n("Sort", "sort key: (sum(o.total)) DESC", 1840.1, 1200, 47.9, [
              n("HashAggregate", "group key: u.id", 1790.4, 1200, 41.2, [
                n("Hash Join", "hash cond: (o.user_id = u.id)", 1610.0, 48000, 33.5, [
                  n("Seq Scan", "orders o · filter: (status = 'paid'::text)", 1200.0, 48000, 21.8, [], ["full table scan on a large table — consider an index"]),
                  n("Hash", null, 32.0, 1200, 1.4, [n("Seq Scan", "users u", 32.0, 1200, 0.6)]),
                ]),
              ]),
            ]),
          ]),
        ],
        raw: '[{"Plan": {"Node Type": "Limit", "…": "…"}}]',
        analyzed: an,
        execution_ms: an ? 48.6 : null,
        planning_ms: an ? 0.4 : null,
      } satisfies Plan;
    }
    case "compare_schema": {
      await sleep(300);
      const col = (name: string, t: string, nullable = true, def: string | null = null, pk = false): ColumnInfo => ({ name, data_type: t, nullable, default: def, primary_key: pk });
      const changes: SchemaChange[] = [
        { kind: "tableAdded", table: "coupons", columns: [col("id", "bigint", false, null, true), col("code", "varchar(32)", false), col("percent_off", "int")] },
        { kind: "columnAdded", table: "users", column: col("last_login_at", "timestamptz") },
        { kind: "columnChanged", table: "users", column: "email", from: col("email", "varchar(64)"), to: col("email", "varchar(120)", false), aspects: ["type", "nullable"] },
        { kind: "columnChanged", table: "orders", column: "status", from: col("status", "text", true, null), to: col("status", "text", true, "'pending'::text"), aspects: ["default"] },
        { kind: "columnRemoved", table: "orders", column: "legacy_ref" },
        { kind: "tableRemoved", table: "tmp_import_2024" },
      ];
      return {
        changes,
        identical: ["products", "order_items", "invoices", "sessions"],
        crossDialect: false,
        sql: `-- Generated by Rowhouse schema compare. Review before running.
-- Drops are commented out on purpose.
CREATE TABLE "public"."coupons" (
  "id" bigint NOT NULL,
  "code" varchar(32) NOT NULL,
  "percent_off" int,
  PRIMARY KEY ("id")
);
ALTER TABLE "public"."users" ADD COLUMN "last_login_at" timestamptz;
ALTER TABLE "public"."users" ALTER COLUMN "email" TYPE varchar(120) USING "email"::varchar(120);
ALTER TABLE "public"."users" ALTER COLUMN "email" SET NOT NULL;
ALTER TABLE "public"."orders" ALTER COLUMN "status" SET DEFAULT 'pending'::text;
-- ALTER TABLE "public"."orders" DROP COLUMN "legacy_ref";
-- DROP TABLE "public"."tmp_import_2024";`,
      } satisfies SchemaDiff;
    }
    case "structure_preview":
    case "apply_structure": {
      const ops = a.ops as ColumnOp[];
      const t = `"${a.schema}"."${a.table}"`;
      return ops.flatMap((o) => {
        if (o.op === "add") return [`ALTER TABLE ${t} ADD COLUMN "${o.column.name}" ${o.column.data_type}${o.column.nullable ? "" : " NOT NULL"}${o.column.default ? ` DEFAULT ${o.column.default}` : ""}`];
        if (o.op === "drop") return [`ALTER TABLE ${t} DROP COLUMN "${o.name}"`];
        if (o.op === "rename") return [`ALTER TABLE ${t} RENAME COLUMN "${o.from}" TO "${o.to}"`];
        const out: string[] = [];
        if (o.from.data_type !== o.to.data_type) out.push(`ALTER TABLE ${t} ALTER COLUMN "${o.to.name}" TYPE ${o.to.data_type} USING "${o.to.name}"::${o.to.data_type}`);
        if (o.from.nullable !== o.to.nullable) out.push(`ALTER TABLE ${t} ALTER COLUMN "${o.to.name}" ${o.to.nullable ? "DROP" : "SET"} NOT NULL`);
        if ((o.from.default ?? "") !== (o.to.default ?? "")) out.push(o.to.default ? `ALTER TABLE ${t} ALTER COLUMN "${o.to.name}" SET DEFAULT ${o.to.default}` : `ALTER TABLE ${t} ALTER COLUMN "${o.to.name}" DROP DEFAULT`);
        return out;
      });
    }
    case "approvals_pending":
      return (window as unknown as { __mockApprovals?: unknown[] }).__mockApprovals ?? [];
    case "approvals_answer": {
      const w = window as unknown as { __mockApprovals?: { id: string }[] };
      w.__mockApprovals = (w.__mockApprovals ?? []).filter((x) => x.id !== a.id);
      return null;
    }
    case "write_text_file":
      return null;
    case "check_destructive": {
      const out: Destructive[] = [];
      const noComments = String(a.sql).replace(/--[^\n]*/g, "");
      for (const st of noComments.split(";").map((x) => x.trim()).filter(Boolean)) {
        const m =
          /^drop\s+\w+\s+(?:if\s+exists\s+)?([\w."]+)/i.exec(st) ??
          /^truncate\s+(?:table\s+)?([\w."]+)/i.exec(st);
        if (m) out.push({ kind: /^drop/i.test(st) ? "drop" : "truncate", object: m[1].replace(/"/g, ""), statement: st });
        const d = /^delete\s+from\s+([\w."]+)/i.exec(st);
        if (d && !/\bwhere\b/i.test(st)) out.push({ kind: "delete-all", object: d[1].replace(/"/g, ""), statement: st });
        const u = /^update\s+([\w."]+)/i.exec(st);
        if (u && !/\bwhere\b/i.test(st)) out.push({ kind: "update-all", object: u[1].replace(/"/g, ""), statement: st });
      }
      return out;
    }
    case "erd": {
      const col = (name: string, t: string, pk = false, nullable = true): ColumnInfo => ({ name, data_type: t, primary_key: pk, nullable, default: null });
      const tables: ErdTable[] = [
        { schema: "public", name: "users", kind: "table", columns: [col("id", "bigint", true, false), col("full_name", "text"), col("email", "citext", false, false), col("team_id", "bigint"), col("created_at", "timestamptz")] },
        { schema: "public", name: "teams", kind: "table", columns: [col("id", "bigint", true, false), col("name", "text"), col("plan_id", "int")] },
        { schema: "public", name: "orders", kind: "table", columns: [col("id", "bigint", true, false), col("user_id", "bigint", false, false), col("status", "text"), col("total", "numeric(12,2)"), col("created_at", "timestamptz")] },
        { schema: "public", name: "order_items", kind: "table", columns: [col("order_id", "bigint", true, false), col("product_id", "bigint", true, false), col("qty", "int"), col("price", "numeric(12,2)")] },
        { schema: "public", name: "products", kind: "table", columns: [col("id", "bigint", true, false), col("sku", "text"), col("name", "text"), col("price", "numeric(12,2)")] },
        { schema: "public", name: "invoices", kind: "table", columns: [col("id", "bigint", true, false), col("order_id", "bigint"), col("due_at", "date"), col("paid", "boolean")] },
        { schema: "public", name: "sessions", kind: "table", columns: [col("id", "uuid", true, false), col("user_id", "bigint"), col("expires_at", "timestamptz")] },
        { schema: "public", name: "audit_log", kind: "table", columns: [col("id", "bigint", true, false), col("actor_id", "bigint"), col("action", "text"), col("at", "timestamptz")] },
        { schema: "billing", name: "plans", kind: "table", columns: [col("id", "int", true, false), col("name", "text"), col("price", "numeric")] },
      ];
      const fk = (table: string, c: string, ref_table: string, rs = "public", s = "public"): ForeignKey => ({ name: `${table}_${c}_fkey`, schema: s, table, columns: [c], ref_schema: rs, ref_table, ref_columns: ["id"] });
      return {
        tables,
        foreign_keys: [
          fk("users", "team_id", "teams"), fk("orders", "user_id", "users"), fk("order_items", "order_id", "orders"),
          fk("order_items", "product_id", "products"), fk("invoices", "order_id", "orders"), fk("sessions", "user_id", "users"),
          fk("audit_log", "actor_id", "users"), fk("teams", "plan_id", "plans", "billing"),
        ],
        truncated: false,
      } satisfies Erd;
    }
    case "import_detect":
      return [
        { source: "tableplus", path: "/Users/you/Library/Containers/com.tinyapp.TablePlus/Data/Library/Application Support/com.tinyapp.TablePlus/Data/Connections.plist", count: 12 },
        { source: "dbeaver", path: "/Users/you/Library/DBeaverData/workspace6/General/.dbeaver/data-sources.json", count: 3 },
      ] satisfies DetectedImport[];
    case "import_parse":
      return {
        source: "navicat",
        candidates: [
          { config: { ...newConnection("postgres"), name: "Main PG", host: "pg.internal", port: 5433, database: "main", env: "production", readOnly: true }, skippedReason: null },
          { config: { ...newConnection("mariadb"), name: "Legacy shop", host: "10.0.0.7", database: "shop", ssh: { host: "bastion", port: 22, user: "ops", auth: "key", keyPath: "~/.ssh/id_rsa" } }, skippedReason: null },
          { config: { ...newConnection("sqlite"), name: "Notes", file: "/Users/me/notes.db" }, skippedReason: null },
          { config: { ...newConnection("postgres"), name: "Billing (Oracle)" }, skippedReason: "Oracle is not supported yet" },
        ],
      };
    case "import_save": {
      const cs = a.configs as ConnectionConfig[];
      const saved = cs.map((c) => ({ ...c, id: crypto.randomUUID() }));
      mockConns.push(...saved);
      return saved;
    }
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
