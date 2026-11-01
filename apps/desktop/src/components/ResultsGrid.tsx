import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowUpRight, Copy, CopyPlus, CornerDownRight, PanelRight, PenLine, Plus, Search, Trash2, Undo2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Cell, Filter, ForeignKey, QueryResult, Relations } from "../lib/api";
import { formatCells, parseClipboard, type CopyFormat } from "../lib/gridcopy";
import { countPending, emptyPending, type Pending } from "../lib/pending";
import { mod } from "../lib/types";
import { rowHeight, useAppearance } from "../lib/appearance";
import { Button, Kbd, cx } from "./ui";
import { columnKind, maskCell } from "../lib/mask";

const INDEX_W = 44;
/** above this many columns only the ones in view are rendered */
const WIDE_COLS = 40;

/** A row on screen: an existing result row (by original index) or a new, unsaved one. */
type DRow = { kind: "orig"; i: number } | { kind: "new"; i: number };

export function display(v: Cell | undefined): string {
  if (v === undefined) return "DEFAULT";
  if (v === null) return "NULL";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

function isNumericType(t: string) {
  return /INT|FLOAT|DOUBLE|REAL|NUMERIC|DECIMAL|SERIAL|OID|YEAR/i.test(t);
}

/** Turn what the user typed back into a value of the same flavour as the original. */
function parseDraft(draft: string, original: Cell | undefined): Cell {
  if (typeof original === "boolean") {
    const d = draft.trim().toLowerCase();
    if (["true", "t", "1", "yes"].includes(d)) return true;
    if (["false", "f", "0", "no"].includes(d)) return false;
  }
  if (typeof original === "number" && draft.trim() !== "" && !Number.isNaN(Number(draft))) return Number(draft);
  return draft;
}

const SKIP = Symbol("skip");

function same(a: Cell | undefined, b: Cell | undefined) {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Virtualised, compact result grid. Handles 100k+ rows smoothly.
 * Pass `edit` to make it editable: changes go into a pending buffer, nothing is
 * written until the parent commits.
 */
export function ResultsGrid({
  result,
  edit: editProp,
  mask,
  serverSort,
  onServerSort,
  table,
  pkColumns = [],
  relations,
  onFollow,
}: {
  result: QueryResult;
  /** mask personal data and secrets (also on in screenshot mode) */
  mask?: boolean;
  /** table name for "Copy as INSERT" */
  table?: string;
  /** result-column indexes of the primary key (left out when duplicating rows) */
  pkColumns?: number[];
  edit?: { pending: Pending; onChange: (p: Pending) => void };
  /** when set, header clicks sort on the server instead of in memory */
  serverSort?: { column: string; desc: boolean } | null;
  onServerSort?: (s: { column: string; desc: boolean } | null) => void;
  /** foreign keys from / into this table (table tabs) */
  relations?: Relations;
  onFollow?: (target: { schema: string; name: string }, filters: Filter[]) => void;
}) {
  const appearance = useAppearance();
  const ROW_H = rowHeight(appearance.density);
  const masking = !!mask || !!appearance.maskAll;
  // masked values must never be written back: no editing while masked
  const edit = masking ? undefined : editProp;
  const maskKinds = useMemo(() => result.columns.map((c) => columnKind(c.name)), [result.columns]);
  const shown = (v: Cell | undefined, ci: number) => (masking ? (maskCell(maskKinds[ci], v) ?? display(v)) : display(v));
  const scrollRef = useRef<HTMLDivElement>(null);
  const [sel, setSel] = useState<{ r: number; c: number } | null>(null);
  /** other corner of a range selection (shift-click / shift-arrows); null = single cell */
  const [anchor, setAnchor] = useState<{ r: number; c: number } | null>(null);
  const [ctx, setCtx] = useState<{ x: number; y: number } | null>(null);
  const [bulk, setBulk] = useState<string | null>(null);
  const [find, setFind] = useState<string | null>(null);
  const [details, setDetails] = useState(() => {
    try {
      return localStorage.getItem("rh-row-details") === "1";
    } catch {
      return false;
    }
  });
  const toggleDetails = () =>
    setDetails((d) => {
      try {
        localStorage.setItem("rh-row-details", d ? "0" : "1");
      } catch {
        /* not remembered */
      }
      return !d;
    });
  const [editing, setEditing] = useState<{ r: number; c: number; draft: string } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);
  const [sort, setSort] = useState<{ c: number; dir: 1 | -1 } | null>(null);
  const pending = edit?.pending ?? emptyPending();

  // Initial column widths from header + first rows.
  useEffect(() => {
    setSel(null);
    setAnchor(null);
    setEditing(null);
    setSort(null);
    setWidths(
      result.columns.map((col, ci) => {
        let max = col.name.length + Math.min(col.data_type.length, 16) * 0.85 + 2;
        for (let r = 0; r < Math.min(result.rows.length, 60); r++) {
          max = Math.max(max, display(result.rows[r][ci]).length);
        }
        return Math.max(72, Math.min(320, max * 7.4 + 20));
      }),
    );
  }, [result]);

  const rows: DRow[] = useMemo(() => {
    const idx = result.rows.map((_, i) => i);
    if (sort) {
      const { c, dir } = sort;
      idx.sort((ia, ib) => {
        const x = result.rows[ia][c],
          y = result.rows[ib][c];
        if (x === y) return 0;
        if (x === null) return 1;
        if (y === null) return -1;
        const nx = Number(x),
          ny = Number(y);
        if (!Number.isNaN(nx) && !Number.isNaN(ny)) return (nx - ny) * dir;
        return display(x).localeCompare(display(y)) * dir;
      });
    }
    const needle = (find ?? "").trim().toLowerCase();
    const hit = (row: Cell[]) => row.some((v) => v !== null && display(v).toLowerCase().includes(needle));
    return [
      ...(needle ? idx.filter((i) => hit(result.rows[i])) : idx).map((i) => ({ kind: "orig" as const, i })),
      ...pending.inserts.map((_, i) => ({ kind: "new" as const, i })),
    ];
  }, [result.rows, sort, pending.inserts, find]);

  const valueAt = (row: DRow, c: number): Cell | undefined => {
    if (row.kind === "new") return pending.inserts[row.i][c];
    const e = pending.edits[row.i];
    return e && c in e ? e[c] : result.rows[row.i][c];
  };

  // ---- foreign keys --------------------------------------------------------
  const colIndex = (name: string) => result.columns.findIndex((c) => c.name === name);
  const fkByCol = useMemo(() => {
    const m = new Map<number, ForeignKey>();
    if (!onFollow) return m;
    for (const fk of relations?.outgoing ?? []) for (const c of fk.columns) if (colIndex(c) >= 0 && !m.has(colIndex(c))) m.set(colIndex(c), fk);
    return m;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [relations, result, onFollow]);
  const linkFilters = (row: DRow, from: string[], to: string[]): Filter[] | null => {
    const out: Filter[] = [];
    for (let k = 0; k < from.length; k++) {
      const ci = colIndex(from[k]);
      const v = ci >= 0 ? valueAt(row, ci) : undefined;
      if (v === null || v === undefined) return null;
      out.push({ column: to[k], op: "eq", value: typeof v === "object" ? JSON.stringify(v) : String(v), numeric: typeof v === "number" });
    }
    return out;
  };
  /** this row → the row it points at */
  const followOut = (row: DRow, fk: ForeignKey) => {
    const f = linkFilters(row, fk.columns, fk.ref_columns);
    if (f) onFollow?.({ schema: fk.ref_schema, name: fk.ref_table }, f);
  };
  /** rows elsewhere that point at this row */
  const followIn = (row: DRow, fk: ForeignKey) => {
    const f = linkFilters(row, fk.ref_columns, fk.columns);
    if (f) onFollow?.({ schema: fk.schema, name: fk.table }, f);
  };
  const isEdited = (row: DRow, c: number) => row.kind === "orig" && !!pending.edits[row.i] && c in pending.edits[row.i];
  const isDeleted = (row: DRow) => row.kind === "orig" && pending.deletes.includes(row.i);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 20,
  });
  useEffect(() => virt.measure(), [ROW_H]); // eslint-disable-line react-hooks/exhaustive-deps

  // Wide results: only render the columns in view (300 columns × visible rows
  // otherwise means thousands of cells per scroll frame).
  const widthsRef = useRef(widths);
  widthsRef.current = widths;
  const colVirt = useVirtualizer({
    horizontal: true,
    count: result.columns.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => widthsRef.current[i] ?? 120,
    paddingStart: INDEX_W,
    scrollPaddingStart: INDEX_W,
    overscan: 4,
  });
  useEffect(() => colVirt.measure(), [widths]); // eslint-disable-line react-hooks/exhaustive-deps
  const virtCols = result.columns.length > WIDE_COLS;
  const visibleCols: number[] = virtCols ? colVirt.getVirtualItems().map((c) => c.index) : result.columns.map((_, i) => i);
  const leadW = virtCols && visibleCols.length ? colVirt.getVirtualItems()[0].start - INDEX_W : 0;

  const total = INDEX_W + widths.reduce((a, b) => a + b, 0);
  const numeric = result.columns.map((c) => isNumericType(c.data_type));

  // ---- selection range -----------------------------------------------------
  const range = sel
    ? {
        r0: Math.min(sel.r, anchor?.r ?? sel.r),
        r1: Math.max(sel.r, anchor?.r ?? sel.r),
        c0: Math.min(sel.c, anchor?.c ?? sel.c),
        c1: Math.max(sel.c, anchor?.c ?? sel.c),
      }
    : null;
  const inRange = (r: number, c: number) => !!range && r >= range.r0 && r <= range.r1 && c >= range.c0 && c <= range.c1;
  const rangeRows = range ? range.r1 - range.r0 + 1 : 0;
  const rangeCells = range ? rangeRows * (range.c1 - range.c0 + 1) : 0;

  const pick = (r: number, c: number, extend: boolean) => {
    if (extend && sel) {
      setAnchor((a) => a ?? sel);
      setSel({ r, c });
    } else {
      setAnchor(null);
      setSel({ r, c });
    }
  };
  /** whole rows (click on the row number) */
  const pickRow = (r: number, extend: boolean) => {
    const last = result.columns.length - 1;
    if (extend && sel) {
      setAnchor({ r: anchor?.r ?? sel.r, c: 0 });
      setSel({ r, c: last });
    } else {
      setAnchor({ r, c: 0 });
      setSel({ r, c: last });
    }
  };

  const copyRange = (format: CopyFormat) => {
    if (!range) return;
    const cols = result.columns.slice(range.c0, range.c1 + 1).map((c) => c.name);
    const block: (Cell | undefined)[][] = [];
    for (let r = range.r0; r <= range.r1; r++) {
      const row = rows[r];
      if (!row) continue;
      const vals: (Cell | undefined)[] = [];
      for (let c = range.c0; c <= range.c1; c++) {
        const v = valueAt(row, c);
        vals.push(masking ? (maskCell(maskKinds[c], v) ?? v) : v);
      }
      block.push(vals);
    }
    navigator.clipboard.writeText(formatCells(format, cols, block, table));
  };

  /** Apply `fn` to every editable cell in the range in one pending update. */
  const mapRange = (fn: (row: DRow, c: number) => Cell | typeof SKIP) => {
    if (!edit || !range) return;
    let next = pending;
    for (let r = range.r0; r <= range.r1; r++) {
      const row = rows[r];
      if (!row || (row.kind === "orig" && next.deletes.includes(row.i))) continue;
      for (let c = range.c0; c <= range.c1; c++) {
        const v = fn(row, c);
        if (v === SKIP) continue;
        next = applyCell(next, row, c, v);
      }
    }
    edit.onChange(next);
  };

  const applyCell = (p: typeof pending, row: DRow, c: number, value: Cell) => {
    if (row.kind === "new") return { ...p, inserts: p.inserts.map((x, i) => (i === row.i ? { ...x, [c]: value } : x)) };
    const rowEdits = { ...(p.edits[row.i] ?? {}) };
    if (same(value, result.rows[row.i][c])) delete rowEdits[c];
    else rowEdits[c] = value;
    const edits = { ...p.edits };
    if (Object.keys(rowEdits).length) edits[row.i] = rowEdits;
    else delete edits[row.i];
    return { ...p, edits };
  };

  const deleteRange = () => {
    if (!edit || !range) return;
    const picked = rows.slice(range.r0, range.r1 + 1);
    const orig = picked.filter((r) => r.kind === "orig").map((r) => r.i);
    const newIdx = new Set(picked.filter((r) => r.kind === "new").map((r) => r.i));
    // toggle: if every picked row is already deleted, restore them
    const allDeleted = orig.length > 0 && orig.every((i) => pending.deletes.includes(i));
    const deletes = allDeleted ? pending.deletes.filter((i) => !orig.includes(i)) : [...new Set([...pending.deletes, ...orig])];
    edit.onChange({ ...pending, deletes, inserts: pending.inserts.filter((_, i) => !newIdx.has(i)) });
    if (newIdx.size) (setSel(null), setAnchor(null));
  };

  const duplicateRange = () => {
    if (!edit || !range) return;
    const copies = rows.slice(range.r0, range.r1 + 1).map((row) => {
      const out: Record<number, Cell> = {};
      result.columns.forEach((_, c) => {
        const v = valueAt(row, c);
        if (!pkColumns.includes(c) && v !== undefined) out[c] = v;
      });
      return out;
    });
    edit.onChange({ ...pending, inserts: [...pending.inserts, ...copies] });
    const r = rows.length;
    setAnchor(null);
    setSel({ r, c: 0 });
    requestAnimationFrame(() => virt.scrollToIndex(r + copies.length - 1));
  };

  const onPaste = (e: React.ClipboardEvent) => {
    if (!edit || !sel || editing) return;
    const block = parseClipboard(e.clipboardData.getData("text/plain"));
    if (!block.length) return;
    e.preventDefault();
    let next = pending;
    // one value pasted into a range fills the range; otherwise paste the block at the cursor
    if (block.length === 1 && block[0].length === 1 && range && rangeCells > 1) {
      for (let r = range.r0; r <= range.r1; r++)
        for (let c = range.c0; c <= range.c1; c++) {
          const row = rows[r];
          if (row && !(row.kind === "orig" && next.deletes.includes(row.i)))
            next = applyCell(next, row, c, parseDraft(block[0][0], row.kind === "orig" ? result.rows[row.i][c] : undefined));
        }
    } else {
      const r0 = range?.r0 ?? sel.r, c0 = range?.c0 ?? sel.c;
      block.forEach((vals, dr) =>
        vals.forEach((v, dc) => {
          const row = rows[r0 + dr];
          const c = c0 + dc;
          if (!row || c >= result.columns.length || (row.kind === "orig" && next.deletes.includes(row.i))) return;
          next = applyCell(next, row, c, parseDraft(v, row.kind === "orig" ? result.rows[row.i][c] : undefined));
        }),
      );
    }
    edit.onChange(next);
  };

  // ---- edit operations -----------------------------------------------------
  const setCell = (r: number, c: number, value: Cell) => {
    if (!edit) return;
    const row = rows[r];
    if (!row || isDeleted(row)) return;
    if (row.kind === "new") {
      const inserts = pending.inserts.map((x, i) => (i === row.i ? { ...x, [c]: value } : x));
      return edit.onChange({ ...pending, inserts });
    }
    const rowEdits = { ...(pending.edits[row.i] ?? {}) };
    if (same(value, result.rows[row.i][c])) delete rowEdits[c];
    else rowEdits[c] = value;
    const edits = { ...pending.edits };
    if (Object.keys(rowEdits).length) edits[row.i] = rowEdits;
    else delete edits[row.i];
    edit.onChange({ ...pending, edits });
  };

  const startEdit = (r: number, c: number) => {
    if (!edit || !rows[r] || isDeleted(rows[r])) return;
    const v = valueAt(rows[r], c);
    setEditing({ r, c, draft: v === null || v === undefined ? "" : display(v) });
  };

  const commitEdit = (move?: { dr: number; dc: number }) => {
    if (!editing) return;
    const row = rows[editing.r];
    const original = row.kind === "orig" ? result.rows[row.i][editing.c] : undefined;
    setCell(editing.r, editing.c, parseDraft(editing.draft, original));
    setEditing(null);
    if (move) moveSel(editing.r + move.dr, editing.c + move.dc);
    scrollRef.current?.focus();
  };

  const addRow = () => {
    if (!edit) return;
    edit.onChange({ ...pending, inserts: [...pending.inserts, {}] });
    const r = rows.length;
    setSel({ r, c: 0 });
    requestAnimationFrame(() => virt.scrollToIndex(r));
  };

  const toggleDelete = () => {
    if (!edit || !sel) return;
    const row = rows[sel.r];
    if (!row) return;
    if (row.kind === "new") {
      edit.onChange({ ...pending, inserts: pending.inserts.filter((_, i) => i !== row.i) });
      setSel(null);
      return;
    }
    const deletes = pending.deletes.includes(row.i)
      ? pending.deletes.filter((i) => i !== row.i)
      : [...pending.deletes, row.i];
    edit.onChange({ ...pending, deletes });
  };

  const revertCell = () => {
    if (!edit || !sel) return;
    const row = rows[sel.r];
    if (row?.kind === "orig") setCell(sel.r, sel.c, result.rows[row.i][sel.c]);
  };

  function moveSel(r: number, c: number, extend = false) {
    const nr = Math.max(0, Math.min(rows.length - 1, r));
    const nc = Math.max(0, Math.min(result.columns.length - 1, c));
    if (extend && sel) setAnchor((a) => a ?? sel);
    else setAnchor(null);
    setSel({ r: nr, c: nc });
    virt.scrollToIndex(nr);
    colVirt.scrollToIndex(nc);
  }

  // Keyboard nav, copy, edit shortcuts.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editing || !sel) return;
    const m = e.metaKey || e.ctrlKey;
    const handled = () => e.preventDefault();
    const x = e.shiftKey;
    if (e.key === "ArrowDown") (moveSel(sel.r + 1, sel.c, x), handled());
    else if (e.key === "ArrowUp") (moveSel(sel.r - 1, sel.c, x), handled());
    else if (e.key === "ArrowLeft") (moveSel(sel.r, sel.c - 1, x), handled());
    else if (e.key === "ArrowRight") (moveSel(sel.r, sel.c + 1, x), handled());
    else if (e.key === "Tab") (moveSel(sel.r, sel.c + 1), handled());
    else if (e.key === "Escape") (setAnchor(null), setCtx(null), handled());
    else if (m && e.key.toLowerCase() === "f") (setFind((f) => f ?? ""), handled());
    else if (m && e.key.toLowerCase() === "a") {
      setAnchor({ r: 0, c: 0 });
      setSel({ r: rows.length - 1, c: result.columns.length - 1 });
      handled();
    } else if (m && e.key.toLowerCase() === "c") {
      if (rangeCells > 1) copyRange(e.shiftKey ? "csv" : "tsv");
      else {
        const v = valueAt(rows[sel.r], sel.c);
        navigator.clipboard.writeText(v === null || v === undefined ? "" : shown(v, sel.c));
      }
      handled();
    } else if (!edit) return;
    else if (e.key === "Enter" || e.key === "F2") (startEdit(sel.r, sel.c), handled());
    else if (m && e.shiftKey && e.key === "Backspace") (deleteRange(), handled());
    else if (m && e.key.toLowerCase() === "d") (duplicateRange(), handled());
    else if (m && e.key === "Backspace") (rangeCells > 1 ? mapRange(() => null) : setCell(sel.r, sel.c, null), handled());
    else if (m && e.key === "z") (revertCell(), handled());
    else if (e.key.length === 1 && !m && !e.altKey) {
      // start typing replaces the value, like a spreadsheet
      setEditing({ r: sel.r, c: sel.c, draft: e.key });
      handled();
    }
  };

  const startResize = (ci: number, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = widths[ci];
    const onMove = (ev: MouseEvent) =>
      setWidths((w) => w.map((v, i) => (i === ci ? Math.max(48, w0 + ev.clientX - x0) : v)));
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  if (result.columns.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted">
        Query OK · {result.rows_affected} row{result.rows_affected === 1 ? "" : "s"} affected
      </div>
    );
  }

  const selRow = sel ? rows[sel.r] : undefined;

  return (
    <div className="relative flex h-full flex-col">
      {find !== null && (
        <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line bg-panel px-2 font-sans text-xs">
          <Search size={12} className="text-subtle" />
          <input
            autoFocus
            value={find}
            onChange={(e) => (setFind(e.target.value), setSel(null), setAnchor(null))}
            onKeyDown={(e) => {
              if (e.key === "Escape") (setFind(null), scrollRef.current?.focus());
              if (e.key === "Enter" && rows.length) (setSel({ r: 0, c: 0 }), scrollRef.current?.focus());
            }}
            placeholder="Find in results…"
            className="h-6 flex-1 bg-transparent outline-none placeholder:text-subtle"
          />
          {find && (
            <span className="text-2xs text-muted tabular-nums">
              {rows.length - pending.inserts.length} of {result.rows.length} rows
            </span>
          )}
          <button className="rounded p-0.5 text-subtle hover:bg-hover hover:text-fg" onClick={() => setFind(null)} title="Close (Esc)">
            <X size={12} />
          </button>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
      <div
        ref={scrollRef}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onMouseDown={() => setCtx(null)}
        className="relative min-h-0 min-w-0 flex-1 overflow-auto font-mono text-xs outline-none"
      >
        <div style={{ width: total, minWidth: "100%" }}>
          {/* header */}
          <div className="sticky top-0 z-10 flex border-b border-line bg-panel" style={{ height: ROW_H }}>
            <div className="sticky left-0 z-10 shrink-0 border-r border-line bg-panel" style={{ width: INDEX_W }} />
            {leadW > 0 && <div className="shrink-0" style={{ width: leadW }} />}
            {visibleCols.map((ci) => { const col = result.columns[ci]; return (
              <div
                key={ci}
                onClick={() =>
                  onServerSort
                    ? onServerSort(
                        serverSort?.column === col.name
                          ? serverSort.desc
                            ? null
                            : { column: col.name, desc: true }
                          : { column: col.name, desc: false },
                      )
                    : setSort((s) => (s?.c === ci ? (s.dir === 1 ? { c: ci, dir: -1 } : null) : { c: ci, dir: 1 }))
                }
                className="group relative flex shrink-0 cursor-default items-center gap-1 border-r border-line px-2 font-sans hover:bg-hover"
                style={{ width: widths[ci] }}
                title={`${col.name} · ${col.data_type}`}
              >
                <span className="truncate font-medium text-fg">{col.name}</span>
                <span className="truncate text-[10px] text-subtle lowercase">{col.data_type}</span>
                {onServerSort
                  ? serverSort?.column === col.name && <span className="ml-auto text-accent">{serverSort.desc ? "↓" : "↑"}</span>
                  : sort?.c === ci && <span className="ml-auto text-accent">{sort.dir === 1 ? "↑" : "↓"}</span>}
                <span
                  onMouseDown={(e) => startResize(ci, e)}
                  className="absolute top-0 -right-1 z-10 h-full w-2 cursor-col-resize"
                />
              </div>
            ); })}
          </div>

          {/* body */}
          <div style={{ height: virt.getTotalSize(), position: "relative" }}>
            {virt.getVirtualItems().map((vr) => {
              const row = rows[vr.index];
              const deleted = isDeleted(row);
              const isNew = row.kind === "new";
              return (
                <div
                  key={vr.key}
                  className={cx(
                    "absolute left-0 flex w-full",
                    vr.index % 2 === 1 && "bg-hover/50",
                    deleted && "bg-danger/8 text-danger line-through decoration-danger/50",
                    isNew && "bg-success/8",
                  )}
                  style={{ height: ROW_H, transform: `translateY(${vr.start}px)` }}
                >
                  <div
                    className={cx(
                      "sticky left-0 shrink-0 border-r border-line bg-bg pr-2 text-right tabular-nums",
                      deleted ? "text-danger" : isNew ? "text-success" : "text-subtle",
                    )}
                    style={{ width: INDEX_W, lineHeight: `${ROW_H}px` }}
                    onMouseDown={(e) => (e.preventDefault(), scrollRef.current?.focus(), pickRow(vr.index, e.shiftKey))}
                  >
                    {isNew ? "+" : deleted ? "−" : row.i + 1}
                  </div>
                  {leadW > 0 && <div className="shrink-0" style={{ width: leadW }} />}
                  {visibleCols.map((ci) => {
                    const v = valueAt(row, ci);
                    const selected = sel?.r === vr.index && sel?.c === ci;
                    const ranged = rangeCells > 1 && inRange(vr.index, ci);
                    const isEditingCell = editing?.r === vr.index && editing?.c === ci;
                    const edited = isEdited(row, ci) || (isNew && v !== undefined);
                    if (isEditingCell) {
                      return (
                        <input
                          key={ci}
                          autoFocus
                          spellCheck={false}
                          value={editing.draft}
                          onChange={(e) => setEditing({ ...editing, draft: e.target.value })}
                          onBlur={() => commitEdit()}
                          onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Enter") (e.preventDefault(), commitEdit({ dr: 1, dc: 0 }));
                            else if (e.key === "Tab") (e.preventDefault(), commitEdit({ dr: 0, dc: e.shiftKey ? -1 : 1 }));
                            else if (e.key === "Escape") (setEditing(null), scrollRef.current?.focus());
                          }}
                          className="z-[1] shrink-0 bg-elevated px-2 font-mono text-xs text-fg shadow-pop ring-2 ring-accent outline-none"
                          style={{ width: widths[ci], height: ROW_H }}
                        />
                      );
                    }
                    return (
                      <div
                        key={ci}
                        onMouseDown={(e) => {
                          if (e.button === 2 && inRange(vr.index, ci)) return; // keep the range for the context menu
                          pick(vr.index, ci, e.shiftKey);
                        }}
                        onMouseEnter={(e) => e.buttons === 1 && sel && !editing && (setAnchor((a) => a ?? sel), setSel({ r: vr.index, c: ci }))}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          const box = scrollRef.current!.parentElement!.getBoundingClientRect();
                          setCtx({ x: e.clientX - box.left, y: e.clientY - box.top });
                        }}
                        onDoubleClick={() => startEdit(vr.index, ci)}
                        className={cx(
                          "shrink-0 truncate border-r border-b border-line/60 px-2",
                          fkByCol.has(ci) && "group/cell relative underline decoration-line-strong decoration-dotted underline-offset-2",
                          numeric[ci] && "text-right tabular-nums",
                          (v === null || v === undefined) && "text-subtle italic",
                          !deleted && typeof v === "boolean" && (v ? "text-success" : "text-muted"),
                          edited && !deleted && "bg-warning/15 text-fg",
                          ranged && "bg-accent-soft/70",
                          selected && "bg-accent-soft ring-1 ring-accent ring-inset",
                        )}
                        style={{ width: widths[ci], lineHeight: `${ROW_H - 1}px` }}
                      >
                        {shown(v, ci)}
                        {fkByCol.has(ci) && v !== null && v !== undefined && !isNew && (
                          <button
                            title={`Open ${fkByCol.get(ci)!.ref_table} where ${fkByCol.get(ci)!.ref_columns.join(", ")} = ${display(v)}`}
                            onMouseDown={(e) => e.stopPropagation()}
                            onClick={(e) => (e.stopPropagation(), followOut(row, fkByCol.get(ci)!))}
                            className={cx(
                              "absolute top-1/2 right-1 -translate-y-1/2 rounded bg-elevated p-0.5 text-accent shadow-sm ring-1 ring-line",
                              selected ? "block" : "hidden group-hover/cell:block",
                            )}
                          >
                            <ArrowUpRight size={11} />
                          </button>
                        )}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {details && (
        <RowDetails
          columns={result.columns}
          row={selRow}
          values={selRow ? result.columns.map((_, c) => (masking ? (maskCell(maskKinds[c], valueAt(selRow, c)) ?? valueAt(selRow, c)) : valueAt(selRow, c))) : []}
          edited={(c) => !!selRow && isEdited(selRow, c)}
          focus={sel?.c}
          onEdit={edit && selRow && !isDeleted(selRow) ? (c, v) => setCell(sel!.r, c, v) : undefined}
          onClose={toggleDetails}
          links={
            selRow && onFollow && relations && (relations.outgoing.length > 0 || relations.incoming.length > 0) ? (
              <div className="border-t border-line px-3 py-2">
                <div className="mb-1 text-2xs font-medium text-subtle">Relations</div>
                {relations.outgoing.map((fk) => {
                  const f = linkFilters(selRow, fk.columns, fk.ref_columns);
                  return (
                    <button key={"o" + fk.name} disabled={!f} onClick={() => followOut(selRow, fk)} className="flex h-7 w-full items-center gap-2 rounded-md px-1.5 text-left text-xs hover:bg-hover disabled:opacity-40">
                      <ArrowUpRight size={12} className="shrink-0 text-accent" />
                      <span className="truncate">{fk.ref_table}</span>
                      <span className="ml-auto truncate font-mono text-2xs text-subtle">{fk.columns.join(", ")}</span>
                    </button>
                  );
                })}
                {relations.incoming.map((fk) => (
                  <button key={"i" + fk.name} onClick={() => followIn(selRow, fk)} className="flex h-7 w-full items-center gap-2 rounded-md px-1.5 text-left text-xs hover:bg-hover">
                    <CornerDownRight size={12} className="shrink-0 text-subtle" />
                    <span className="truncate">{fk.table}</span>
                    <span className="ml-auto truncate font-mono text-2xs text-subtle">{fk.columns.join(", ")}</span>
                  </button>
                ))}
              </div>
            ) : null
          }
        />
      )}
      </div>

      {ctx && range && (
        <div
          className="animate-pop absolute z-40 min-w-52 rounded-lg border border-line bg-elevated p-1 font-sans text-xs shadow-pop"
          style={{ left: ctx.x, top: ctx.y }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <div className="px-2 py-1 text-2xs text-subtle">
            {rangeRows} row{rangeRows === 1 ? "" : "s"} · {rangeCells} cell{rangeCells === 1 ? "" : "s"}
          </div>
          {(
            [
              ["tsv", "Copy", `${mod}C`],
              ["csv", "Copy as CSV", `${mod}⇧C`],
              ["json", "Copy as JSON", ""],
              ["markdown", "Copy as Markdown", ""],
              ["insert", "Copy as INSERT", ""],
            ] as const
          ).map(([f, label, hint]) => (
            <button key={f} className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (copyRange(f), setCtx(null))}>
              <Copy size={12} className="text-subtle" /> {label}
              {hint && <span className="ml-auto text-2xs text-subtle">{hint}</span>}
            </button>
          ))}
          {selRow && sel && fkByCol.has(sel.c) && (
            <button className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (followOut(selRow, fkByCol.get(sel.c)!), setCtx(null))}>
              <ArrowUpRight size={12} className="text-accent" /> Open {fkByCol.get(sel.c)!.ref_table} row
            </button>
          )}
          {selRow && onFollow && (relations?.incoming.length ?? 0) > 0 && (
            <>
              <div className="my-1 h-px bg-line" />
              <div className="px-2 py-0.5 text-2xs text-subtle">Referenced by</div>
              {relations!.incoming.slice(0, 8).map((fk) => (
                <button key={fk.name} className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (followIn(selRow, fk), setCtx(null))}>
                  <CornerDownRight size={12} className="text-subtle" /> {fk.table}
                  <span className="ml-auto text-2xs text-subtle">{fk.columns.join(", ")}</span>
                </button>
              ))}
            </>
          )}
          {edit && (
            <>
              <div className="my-1 h-px bg-line" />
              <button className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (setBulk(""), setCtx(null))}>
                <PenLine size={12} className="text-subtle" /> Set value…
              </button>
              <button className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (mapRange(() => null), setCtx(null))}>
                <span className="w-3 text-center text-[10px] text-subtle">∅</span> Set NULL
                <span className="ml-auto text-2xs text-subtle">{mod}⌫</span>
              </button>
              <button className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" onClick={() => (duplicateRange(), setCtx(null))}>
                <CopyPlus size={12} className="text-subtle" /> Duplicate row{rangeRows === 1 ? "" : "s"}
                <span className="ml-auto text-2xs text-subtle">{mod}D</span>
              </button>
              <button className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-danger hover:bg-danger/10" onClick={() => (deleteRange(), setCtx(null))}>
                <Trash2 size={12} /> Delete row{rangeRows === 1 ? "" : "s"}
                <span className="ml-auto text-2xs opacity-70">{mod}⇧⌫</span>
              </button>
            </>
          )}
        </div>
      )}

      {bulk !== null && range && (
        <form
          className="flex h-9 shrink-0 items-center gap-2 border-t border-line bg-accent-soft/40 px-2 font-sans text-xs"
          onSubmit={(e) => {
            e.preventDefault();
            mapRange((row, c) => parseDraft(bulk, row.kind === "orig" ? result.rows[row.i][c] : undefined));
            setBulk(null);
            scrollRef.current?.focus();
          }}
        >
          <PenLine size={12} className="text-accent" />
          <span className="whitespace-nowrap text-muted">
            Set {rangeCells} cell{rangeCells === 1 ? "" : "s"} to
          </span>
          <input
            autoFocus
            value={bulk}
            onChange={(e) => setBulk(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && (setBulk(null), scrollRef.current?.focus())}
            className="h-6 flex-1 rounded-md border border-line bg-elevated px-2 font-mono text-xs outline-none focus:border-accent"
          />
          <Button size="xs" variant="primary" type="submit">
            Apply
          </Button>
          <Button size="xs" variant="ghost" type="button" onClick={() => setBulk(null)}>
            Cancel
          </Button>
        </form>
      )}

      <div className="flex h-8 shrink-0 items-center gap-1 border-t border-line bg-panel px-2 text-2xs text-muted">
        {edit && (
          <>
          <Button variant="ghost" size="xs" onClick={addRow}>
            <Plus size={12} /> Row
          </Button>
          <Button variant="ghost" size="xs" onClick={rangeRows > 1 ? deleteRange : toggleDelete} disabled={!selRow}>
            <Trash2 size={12} /> {rangeRows <= 1 && selRow && isDeleted(selRow) ? "Restore" : rangeRows > 1 ? `Delete ${rangeRows}` : "Delete"}
          </Button>
          <Button variant="ghost" size="xs" onClick={duplicateRange} disabled={!selRow} title={`Duplicate (${mod}D)`}>
            <CopyPlus size={12} /> Duplicate
          </Button>
          <Button variant="ghost" size="xs" onClick={() => setBulk("")} disabled={!sel} title="Set every selected cell to one value">
            <PenLine size={12} /> Set{rangeCells > 1 ? ` ${rangeCells}` : ""}
          </Button>
          <Button variant="ghost" size="xs" onClick={() => (rangeCells > 1 ? mapRange(() => null) : sel && setCell(sel.r, sel.c, null))} disabled={!sel}>
            NULL
          </Button>
          <Button variant="ghost" size="xs" onClick={revertCell} disabled={!sel || !selRow || !isEdited(selRow, sel.c)}>
            <Undo2 size={12} /> Revert
          </Button>
          <span className="flex-1" />
          <span className="hidden items-center gap-1 whitespace-nowrap @3xl:flex">
            Double-click or <Kbd>↵</Kbd> to edit · <Kbd>{mod}</Kbd>
            <Kbd>⌫</Kbd> NULL · <Kbd>{mod}</Kbd>
            <Kbd>S</Kbd> commit
          </span>
          {countPending(pending) > 0 && <span className="ml-2 whitespace-nowrap text-warning">{countPending(pending)} pending</span>}
          </>
        )}
        {!edit && <span className="flex-1" />}
        <Button variant="ghost" size="xs" onClick={() => setFind((f) => (f === null ? "" : null))} className={cx(find !== null && "bg-selected text-fg")} title={`Find in results (${mod}F)`}>
          <Search size={12} />
        </Button>
        <Button variant="ghost" size="xs" onClick={toggleDetails} className={cx(details && "bg-selected text-fg")} title="Row details">
          <PanelRight size={12} />
        </Button>
      </div>
    </div>
  );
}

/** Selected row, one field per line; long text and JSON get room to breathe. */
function RowDetails({
  columns,
  row,
  values,
  edited,
  focus,
  onEdit,
  onClose,
  links,
}: {
  links?: React.ReactNode;
  columns: { name: string; data_type: string }[];
  row: DRow | undefined;
  values: (Cell | undefined)[];
  edited: (c: number) => boolean;
  focus?: number;
  onEdit?: (c: number, v: Cell) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focus === undefined) return;
    ref.current?.querySelector(`[data-field="${focus}"]`)?.scrollIntoView({ block: "nearest" });
  }, [focus, row]);
  return (
    <div className="flex w-80 shrink-0 flex-col border-l border-line bg-panel font-sans">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3 text-xs">
        <span className="font-medium">{row ? (row.kind === "new" ? "New row" : `Row ${row.i + 1}`) : "Row details"}</span>
        <button className="ml-auto rounded p-0.5 text-subtle hover:bg-hover hover:text-fg" onClick={onClose}>
          <X size={12} />
        </button>
      </div>
      {!row ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-xs text-subtle">Select a cell to see its row here</div>
      ) : (
        <div ref={ref} className="flex-1 overflow-y-auto">
          {columns.map((col, c) => (
            <Field
              key={c}
              index={c}
              name={col.name}
              type={col.data_type}
              value={values[c]}
              active={focus === c}
              edited={edited(c)}
              onEdit={onEdit ? (v) => onEdit(c, v) : undefined}
            />
          ))}
        </div>
      )}
      {row && links}
    </div>
  );
}

function isJsonish(v: Cell | undefined, type: string) {
  if (v !== null && typeof v === "object") return true;
  if (/json/i.test(type)) return true;
  return typeof v === "string" && /^\s*[[{]/.test(v) && (() => {
    try {
      JSON.parse(v);
      return true;
    } catch {
      return false;
    }
  })();
}

function Field({
  index,
  name,
  type,
  value,
  active,
  edited,
  onEdit,
}: {
  index: number;
  name: string;
  type: string;
  value: Cell | undefined;
  active: boolean;
  edited: boolean;
  onEdit?: (v: Cell) => void;
}) {
  const json = isJsonish(value, type);
  const shown = useMemo(() => {
    if (value === null || value === undefined) return "";
    if (json) {
      try {
        return JSON.stringify(typeof value === "string" ? JSON.parse(value) : value, null, 2);
      } catch {
        return String(value);
      }
    }
    return typeof value === "object" ? JSON.stringify(value) : String(value);
  }, [value, json]);
  const [draft, setDraft] = useState(shown);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => (setDraft(shown), setErr(null)), [shown]);
  const multi = json || shown.length > 40 || shown.includes("\n");
  const commit = () => {
    if (!onEdit || draft === shown) return;
    if (json && draft.trim()) {
      try {
        onEdit(JSON.stringify(JSON.parse(draft)));
        setErr(null);
      } catch (e) {
        setErr(String(e).replace(/^SyntaxError: /, ""));
      }
      return;
    }
    onEdit(parseDraft(draft, value));
  };
  return (
    <div data-field={index} className={cx("border-b border-line/70 px-3 py-2", active && "bg-accent-soft/40")}>
      <div className="mb-1 flex items-baseline gap-1.5">
        <span className="truncate text-xs font-medium">{name}</span>
        <span className="truncate text-[10px] text-subtle">{type.toLowerCase()}</span>
        {edited && <span className="ml-auto text-[10px] text-warning">edited</span>}
        {json && shown && (
          <button
            className={cx("text-[10px] text-subtle hover:text-fg", !edited && "ml-auto")}
            onClick={() => navigator.clipboard.writeText(shown)}
            title="Copy"
          >
            <Copy size={10} />
          </button>
        )}
      </div>
      {multi ? (
        <textarea
          value={value === null && !onEdit ? "NULL" : draft}
          readOnly={!onEdit}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          spellCheck={false}
          rows={json ? Math.min(14, Math.max(3, shown.split("\n").length)) : 3}
          className="w-full resize-y rounded-md border border-line bg-elevated px-2 py-1.5 font-mono text-[11.5px] leading-relaxed outline-none focus:border-accent"
        />
      ) : (
        <input
          value={draft}
          placeholder={value === null ? "NULL" : value === undefined ? "DEFAULT" : ""}
          readOnly={!onEdit}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
          spellCheck={false}
          className="h-7 w-full rounded-md border border-line bg-elevated px-2 font-mono text-[11.5px] outline-none placeholder:text-subtle placeholder:italic focus:border-accent"
        />
      )}
      {err && <div className="mt-1 text-[10.5px] text-danger">Invalid JSON: {err}</div>}
    </div>
  );
}
