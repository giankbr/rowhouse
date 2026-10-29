import { sql, PostgreSQL, MySQL, SQLite, MSSQL, StandardSQL, type SQLNamespace } from "@codemirror/lang-sql";
import { keymap, EditorView } from "@codemirror/view";
import { Prec } from "@codemirror/state";
import { closeCompletion } from "@codemirror/autocomplete";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import CodeMirror from "@uiw/react-codemirror";
import { useMemo, useRef } from "react";
import { kindInfo, type DbKind, type TableInfo } from "../lib/api";

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
}: {
  value: string;
  onChange: (v: string) => void;
  onRun: (selection?: string) => void;
  kind: DbKind;
  tables: TableInfo[];
}) {
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
            : d === "duckdb"
              ? StandardSQL
              : PostgreSQL;
    return [
      sql({ dialect, schema, upperCaseKeywords: true }),
      theme,
      syntaxHighlighting(highlight),
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
  }, [kind, tables]);

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
