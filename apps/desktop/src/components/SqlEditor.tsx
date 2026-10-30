import { javascript } from "@codemirror/lang-javascript";
import { sql, PostgreSQL, MySQL, SQLite, MSSQL, StandardSQL, type SQLNamespace } from "@codemirror/lang-sql";
import { keymap, EditorView } from "@codemirror/view";
import { Prec } from "@codemirror/state";
import { autocompletion, closeCompletion, type CompletionContext } from "@codemirror/autocomplete";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import CodeMirror from "@uiw/react-codemirror";
import { useMemo, useRef } from "react";
import { kindInfo, type DbKind, type TableInfo } from "../lib/api";
import { ghostText } from "../lib/ghost";

const theme = EditorView.theme({
  "&": { color: "var(--fg)" },
  ".cm-content": { caretColor: "var(--accent)", padding: "8px 0" },
  ".cm-cursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--accent-soft) !important",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": { background: "var(--accent)", color: "var(--accent-fg)" },
  ".cm-tooltip-autocomplete ul": { fontFamily: "var(--font-mono)", fontSize: "12px" },
});

const MONGO_METHODS = [
  "find", "findOne", "aggregate", "countDocuments", "estimatedDocumentCount", "distinct", "getIndexes",
  "insertOne", "insertMany", "updateOne", "updateMany", "replaceOne", "deleteOne", "deleteMany",
  "createIndex", "dropIndex", "drop", "stats",
];
const MONGO_CURSOR = ["sort", "limit", "skip", "projection", "count"];
const MONGO_OPS = [
  "$eq", "$ne", "$gt", "$gte", "$lt", "$lte", "$in", "$nin", "$exists", "$regex", "$and", "$or", "$not", "$elemMatch",
  "$set", "$unset", "$inc", "$push", "$pull", "$addToSet",
  "$match", "$group", "$project", "$sort", "$limit", "$skip", "$lookup", "$unwind", "$count", "$sum", "$avg", "$min", "$max",
];

/** `db.<collection>.<method>(…).<cursor>()` and `$operators`. */
function mongoCompletions(collections: string[]) {
  return (ctx: CompletionContext) => {
    const line = ctx.state.doc.lineAt(ctx.pos);
    const before = line.text.slice(0, ctx.pos - line.from);
    let m = /\bdb\.(\w*)$/.exec(before);
    if (m) return { from: ctx.pos - m[1].length, options: collections.map((c) => ({ label: c, type: "class" })) };
    m = /\bdb\.\w+\.(\w*)$/.exec(before);
    if (m) return { from: ctx.pos - m[1].length, options: MONGO_METHODS.map((x) => ({ label: x, type: "method", apply: `${x}(` })) };
    m = /\)\s*\.(\w*)$/.exec(before);
    if (m) return { from: ctx.pos - m[1].length, options: MONGO_CURSOR.map((x) => ({ label: x, type: "method", apply: `${x}(` })) };
    m = /(\$\w*)$/.exec(before);
    if (m) return { from: ctx.pos - m[1].length, options: MONGO_OPS.map((x) => ({ label: x, type: "keyword" })) };
    if (/^\s*\w*$/.test(before) && ctx.explicit)
      return { from: ctx.pos - (/\w*$/.exec(before)?.[0].length ?? 0), options: [{ label: "db", type: "variable" }, { label: "show collections", type: "keyword" }] };
    return null;
  };
}

const highlight = HighlightStyle.define([
  { tag: t.keyword, color: "var(--accent)", fontWeight: "500" },
  { tag: [t.string, t.special(t.string)], color: "var(--success)" },
  { tag: [t.number, t.bool, t.null], color: "var(--warning)" },
  { tag: [t.lineComment, t.blockComment], color: "var(--subtle)", fontStyle: "italic" },
  { tag: [t.typeName, t.standard(t.name)], color: "#a855f7" },
  { tag: [t.operator, t.punctuation, t.paren], color: "var(--muted)" },
  { tag: t.special(t.name), color: "var(--fg)", fontWeight: "500" },
]);

/** SQL editor with dialect + schema-aware autocomplete. Mod+Enter runs. */
export function SqlEditor({
  value,
  onChange,
  onRun,
  kind,
  tables,
  suggest,
}: {
  value: string;
  onChange: (v: string) => void;
  onRun: (selection?: string) => void;
  kind: DbKind;
  tables: TableInfo[];
  /** inline AI suggestions; omit to turn them off */
  suggest?: (before: string, after: string) => Promise<string>;
}) {
  const suggestRef = useRef(suggest);
  suggestRef.current = suggest;
  const runRef = useRef(onRun);
  runRef.current = onRun;
  const extensions = useMemo(() => {
    const schema: SQLNamespace = {};
    for (const t of tables) schema[t.name] = [];
    const d = kindInfo(kind).dialect;
    const dialect =
      d === "mysql" || d === "clickhouse"
        ? MySQL
        : d === "sqlite"
          ? SQLite
          : d === "mssql"
            ? MSSQL
            : d === "duckdb" || d === "cql"
              ? StandardSQL
              : PostgreSQL;
    return [
      ...(d === "redis"
        ? []
        : d === "mongodb"
          ? [javascript(), autocompletion({ override: [mongoCompletions(tables.map((t) => t.name))] })]
          : [sql({ dialect, schema, upperCaseKeywords: true })]),
      theme,
      syntaxHighlighting(highlight),
      ...(suggest ? [ghostText((b, a) => suggestRef.current?.(b, a) ?? Promise.resolve(""))] : []),
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: (view) => {
              closeCompletion(view);
              const s = view.state.selection.main;
              runRef.current(s.empty ? undefined : view.state.sliceDoc(s.from, s.to));
              return true;
            },
          },
        ]),
      ),
    ];
  }, [kind, tables, !!suggest]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      extensions={extensions}
      theme="none"
      height="100%"
      basicSetup={{ foldGutter: false, highlightActiveLineGutter: true, searchKeymap: true }}
      className="h-full"
    />
  );
}
