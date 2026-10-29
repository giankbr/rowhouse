import type { ColumnInfo, QueryOutput } from "./api";

export interface Tab {
  id: string;
  kind: "query" | "table";
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
}

export const uid = () => Math.random().toString(36).slice(2, 10);
export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
export const mod = isMac ? "⌘" : "Ctrl";
