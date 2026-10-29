import type { ColumnInfo, QueryOutput } from "./api";
import type { Pending } from "./pending";

export interface Tab {
  id: string;
  kind: "query" | "table" | "erd";
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
  /** unsaved grid edits (table tabs only) */
  pending?: Pending;
}

export const uid = () => Math.random().toString(36).slice(2, 10);
export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
export const mod = isMac ? "⌘" : "Ctrl";
