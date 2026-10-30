import type { ColumnInfo, QueryOutput } from "./api";
import type { Pending } from "./pending";

export interface Tab {
  id: string;
  kind: "query" | "table" | "erd" | "compare" | "builder";
  /** split pane the tab lives in */
  pane?: 0 | 1;
  title: string;
  connId: string;
  sql: string;
  schema?: string;
  table?: string;
  view?: "data" | "structure";
  columns?: ColumnInfo[];
  output?: QueryOutput;
  /** which result set is shown when a script returns several */
  resultIndex?: number;
  error?: string;
  running?: boolean;
  /** id of the running query (for cancel) */
  runId?: string;
  runStarted?: number;
  /** restored from the last session; reload data when shown */
  stale?: boolean;
  /** table tabs: server-side browsing */
  filters?: import("./api").Filter[];
  sort?: { column: string; desc: boolean } | null;
  offset?: number;
  pageSize?: number;
  total?: number | null;
  /** results area shows the query plan instead of rows */
  showPlan?: boolean;
  plan?: import("./api").Plan;
  planError?: string;
  planLoading?: boolean;
  /** saved-query id this tab was opened from / saved as */
  snippetId?: number;
  /** visual query builder state (builder tabs) */
  qb?: import("./qb").QbState;
  /** unsaved grid edits (table tabs only) */
  pending?: Pending;
}

export const uid = () => Math.random().toString(36).slice(2, 10);
export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
export const mod = isMac ? "⌘" : "Ctrl";
