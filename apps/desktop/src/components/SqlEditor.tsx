import { javascript } from "@codemirror/lang-javascript";
import { sql, PostgreSQL, MySQL, SQLite, MSSQL, StandardSQL, type SQLNamespace } from "@codemirror/lang-sql";
import { keymap, EditorView } from "@codemirror/view";
import { Prec } from "@codemirror/state";
import { autocompletion, closeCompletion, type CompletionContext } from "@codemirror/autocomplete";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import CodeMirror from "@uiw/react-codemirror";
import { useMemo, useRef, useState } from "react";
import { Sparkles } from "lucide-react";
import { isMac } from "../lib/types";
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
  inlineEdit,
}: {
  value: string;
  onChange: (v: string) => void;
  onRun: (selection?: string) => void;
  kind: DbKind;
  tables: TableInfo[];
  /** inline AI suggestions; omit to turn them off */
  suggest?: (before: string, after: string) => Promise<string>;
  /** Mod+I in the editor: rewrite the selection / generate at the cursor */
  inlineEdit?: (
    instruction: string,
    selection: string,
    before: string,
    after: string,
    onDelta: (s: string) => void,
  ) => Promise<{ sql: string; explanation: string }>;
}) {
  const suggestRef = useRef(suggest);
  suggestRef.current = suggest;
  const runRef = useRef(onRun);
  runRef.current = onRun;
  const viewRef = useRef<EditorView | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [edit, setEdit] = useState<InlineEditState | null>(null);
  const inlineEditRef = useRef(inlineEdit);
  inlineEditRef.current = inlineEdit;

  const openEdit = (view: EditorView) => {
    if (!inlineEditRef.current) return false;
    const s = view.state.selection.main;
    const c = view.coordsAtPos(s.from);
    const box = wrapRef.current?.getBoundingClientRect();
    if (!box) return false;
    const below = view.coordsAtPos(s.to);
    setEdit({
      from: s.from,
      to: s.to,
      hasSelection: !s.empty,
      prompt: "",
      top: Math.max(4, (below?.bottom ?? c?.bottom ?? box.top + 24) - box.top + 6),
      left: Math.min(Math.max(8, (c?.left ?? box.left) - box.left - 12), Math.max(8, box.width - 572)),
    });
    return true;
  };
  const submitEdit = async () => {
    const view = viewRef.current;
    const fn = inlineEditRef.current;
    if (!view || !fn || !edit) return;
    const doc = view.state.doc;
    const cur = { ...edit, loading: true, streamed: "", proposal: undefined, error: undefined };
    setEdit(cur);
    try {
      const r = await fn(
        edit.prompt,
        doc.sliceString(edit.from, edit.to),
        doc.sliceString(0, edit.from),
        doc.sliceString(edit.to),
        (d) => setEdit((e) => e && { ...e, streamed: (e.streamed ?? "") + d }),
      );
      setEdit((e) => e && { ...e, loading: false, proposal: r.sql, note: r.explanation });
    } catch (err) {
      setEdit((e) => e && { ...e, loading: false, error: String(err) });
    }
  };
  const acceptEdit = () => {
    const view = viewRef.current;
    if (!view || !edit?.proposal) return;
    const insert = edit.hasSelection ? edit.proposal : edit.proposal + "\n";
    view.dispatch({
      changes: { from: edit.from, to: edit.to, insert },
      selection: { anchor: edit.from, head: edit.from + insert.length },
      userEvent: "input.ai",
    });
    setEdit(null);
    view.focus();
  };
  const closeEdit = () => {
    setEdit(null);
    viewRef.current?.focus();
  };
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
            key: "Mod-i",
            run: (view) => openEdit(view),
          },
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
    <div ref={wrapRef} className="relative h-full">
      <CodeMirror
        value={value}
        onChange={onChange}
        extensions={extensions}
        theme="none"
        height="100%"
        basicSetup={{ foldGutter: false, highlightActiveLineGutter: true, searchKeymap: true }}
        className="h-full"
        onCreateEditor={(v) => (viewRef.current = v)}
      />
      {edit && (
        <InlineEditBox
          state={edit}
          onChange={setEdit}
          onSubmit={submitEdit}
          onAccept={acceptEdit}
          onClose={closeEdit}
        />
      )}
    </div>
  );
}

export interface InlineEditState {
  from: number;
  to: number;
  top: number;
  left: number;
  prompt: string;
  hasSelection: boolean;
  loading?: boolean;
  streamed?: string;
  proposal?: string;
  note?: string;
  error?: string;
}

/** Floating "edit with AI" box anchored at the selection (Mod+I in the editor). */
function InlineEditBox({
  state,
  onChange,
  onSubmit,
  onAccept,
  onClose,
}: {
  state: InlineEditState;
  onChange: (s: InlineEditState) => void;
  onSubmit: () => void;
  onAccept: () => void;
  onClose: () => void;
}) {
  const shown = state.proposal ?? (state.streamed ?? "").replace(/```[a-zA-Z]*\n?/g, "");
  return (
    <div
      className="animate-pop absolute z-30 w-[min(560px,calc(100%-24px))] overflow-hidden rounded-xl border border-accent/40 bg-elevated shadow-pop"
      style={{ top: state.top, left: state.left }}
      onKeyDown={(e) => {
        if (e.key === "Escape") (e.preventDefault(), e.stopPropagation(), onClose());
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && state.proposal) (e.preventDefault(), onAccept());
      }}
    >
      <form
        className="flex items-center gap-2 px-2.5 py-1.5"
        onSubmit={(e) => (e.preventDefault(), state.prompt.trim() && !state.loading && onSubmit())}
      >
        <Sparkles size={13} className="shrink-0 text-accent" />
        <input
          autoFocus
          value={state.prompt}
          onChange={(e) => onChange({ ...state, prompt: e.target.value })}
          placeholder={state.hasSelection ? "Edit selection… e.g. only paid orders, add a date filter" : "Generate SQL here… e.g. users without orders"}
          className="h-6 flex-1 bg-transparent text-xs outline-none placeholder:text-subtle"
          spellCheck={false}
        />
        {state.loading && <span className="size-3 shrink-0 animate-spin rounded-full border-2 border-accent border-t-transparent" />}
        <kbd className="text-[10px] text-subtle">esc</kbd>
      </form>
      {(shown || state.error) && (
        <div className="border-t border-line">
          {state.error ? (
            <div className="px-3 py-2 text-xs text-danger">{state.error}</div>
          ) : (
            <pre className="max-h-48 overflow-auto bg-success/8 px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-fg">{shown}</pre>
          )}
          {state.proposal !== undefined && (
            <div className="flex items-center gap-2 border-t border-line px-2.5 py-1.5">
              <span className="min-w-0 flex-1 truncate text-2xs text-muted">{state.note}</span>
              <button type="button" onClick={onClose} className="h-6 rounded-md px-2 text-2xs text-muted hover:bg-hover hover:text-fg">
                Reject
              </button>
              <button type="button" onClick={onAccept} className="h-6 rounded-md bg-accent px-2.5 text-2xs font-medium text-accent-fg hover:brightness-110">
                Accept <span className="opacity-70">{isMac ? "⌘" : "Ctrl"}↵</span>
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
