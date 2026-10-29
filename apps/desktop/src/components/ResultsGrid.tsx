import { useVirtualizer } from "@tanstack/react-virtual";
import { Plus, Trash2, Undo2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Cell, QueryResult } from "../lib/api";
import { countPending, emptyPending, type Pending } from "../lib/pending";
import { mod } from "../lib/types";
import { Button, Kbd, cx } from "./ui";

const ROW_H = 26;
const INDEX_W = 44;

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
  edit,
  serverSort,
  onServerSort,
}: {
  result: QueryResult;
  edit?: { pending: Pending; onChange: (p: Pending) => void };
  /** when set, header clicks sort on the server instead of in memory */
  serverSort?: { column: string; desc: boolean } | null;
  onServerSort?: (s: { column: string; desc: boolean } | null) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [sel, setSel] = useState<{ r: number; c: number } | null>(null);
  const [editing, setEditing] = useState<{ r: number; c: number; draft: string } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);
  const [sort, setSort] = useState<{ c: number; dir: 1 | -1 } | null>(null);
  const pending = edit?.pending ?? emptyPending();

  // Initial column widths from header + first rows.
  useEffect(() => {
    setSel(null);
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
    return [
      ...idx.map((i) => ({ kind: "orig" as const, i })),
      ...pending.inserts.map((_, i) => ({ kind: "new" as const, i })),
    ];
  }, [result.rows, sort, pending.inserts]);

  const valueAt = (row: DRow, c: number): Cell | undefined => {
    if (row.kind === "new") return pending.inserts[row.i][c];
    const e = pending.edits[row.i];
    return e && c in e ? e[c] : result.rows[row.i][c];
  };
  const isEdited = (row: DRow, c: number) => row.kind === "orig" && !!pending.edits[row.i] && c in pending.edits[row.i];
  const isDeleted = (row: DRow) => row.kind === "orig" && pending.deletes.includes(row.i);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 20,
  });

  const total = INDEX_W + widths.reduce((a, b) => a + b, 0);
  const numeric = result.columns.map((c) => isNumericType(c.data_type));

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

  function moveSel(r: number, c: number) {
    const nr = Math.max(0, Math.min(rows.length - 1, r));
    const nc = Math.max(0, Math.min(result.columns.length - 1, c));
    setSel({ r: nr, c: nc });
    virt.scrollToIndex(nr);
  }

  // Keyboard nav, copy, edit shortcuts.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editing || !sel) return;
    const m = e.metaKey || e.ctrlKey;
    const handled = () => e.preventDefault();
    if (e.key === "ArrowDown") (moveSel(sel.r + 1, sel.c), handled());
    else if (e.key === "ArrowUp") (moveSel(sel.r - 1, sel.c), handled());
    else if (e.key === "ArrowLeft") (moveSel(sel.r, sel.c - 1), handled());
    else if (e.key === "ArrowRight" || e.key === "Tab") (moveSel(sel.r, sel.c + 1), handled());
    else if (m && e.key === "c") {
      const v = valueAt(rows[sel.r], sel.c);
      navigator.clipboard.writeText(v === null || v === undefined ? "" : display(v));
      handled();
    } else if (!edit) return;
    else if (e.key === "Enter" || e.key === "F2") (startEdit(sel.r, sel.c), handled());
    else if (m && e.shiftKey && e.key === "Backspace") (toggleDelete(), handled());
    else if (m && e.key === "Backspace") (setCell(sel.r, sel.c, null), handled());
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
    <div className="flex h-full flex-col">
      <div
        ref={scrollRef}
        tabIndex={0}
        onKeyDown={onKeyDown}
        className="relative min-h-0 flex-1 overflow-auto font-mono text-xs outline-none"
      >
        <div style={{ width: total, minWidth: "100%" }}>
          {/* header */}
          <div className="sticky top-0 z-10 flex border-b border-line bg-panel" style={{ height: ROW_H }}>
            <div className="sticky left-0 z-10 shrink-0 border-r border-line bg-panel" style={{ width: INDEX_W }} />
            {result.columns.map((col, ci) => (
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
            ))}
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
                      "sticky left-0 shrink-0 border-r border-line bg-bg pr-2 text-right leading-[26px] tabular-nums",
                      deleted ? "text-danger" : isNew ? "text-success" : "text-subtle",
                    )}
                    style={{ width: INDEX_W }}
                  >
                    {isNew ? "+" : deleted ? "−" : row.i + 1}
                  </div>
                  {result.columns.map((_, ci) => {
                    const v = valueAt(row, ci);
                    const selected = sel?.r === vr.index && sel?.c === ci;
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
                        onMouseDown={() => setSel({ r: vr.index, c: ci })}
                        onDoubleClick={() => startEdit(vr.index, ci)}
                        className={cx(
                          "shrink-0 truncate border-r border-b border-line/60 px-2 leading-[25px]",
                          numeric[ci] && "text-right tabular-nums",
                          (v === null || v === undefined) && "text-subtle italic",
                          !deleted && typeof v === "boolean" && (v ? "text-success" : "text-muted"),
                          edited && !deleted && "bg-warning/15 text-fg",
                          selected && "bg-accent-soft ring-1 ring-accent ring-inset",
                        )}
                        style={{ width: widths[ci] }}
                      >
                        {display(v)}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {edit && (
        <div className="flex h-8 shrink-0 items-center gap-1 border-t border-line bg-panel px-2 text-2xs text-muted">
          <Button variant="ghost" size="xs" onClick={addRow}>
            <Plus size={12} /> Row
          </Button>
          <Button variant="ghost" size="xs" onClick={toggleDelete} disabled={!selRow}>
            <Trash2 size={12} /> {selRow && isDeleted(selRow) ? "Restore" : "Delete"}
          </Button>
          <Button variant="ghost" size="xs" onClick={() => sel && setCell(sel.r, sel.c, null)} disabled={!sel}>
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
        </div>
      )}
    </div>
  );
}
