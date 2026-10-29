import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Cell, QueryResult } from "../lib/api";
import { cx } from "./ui";

const ROW_H = 26;
const INDEX_W = 44;

function display(v: Cell): string {
  if (v === null) return "NULL";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

function isNumericType(t: string) {
  return /INT|FLOAT|DOUBLE|REAL|NUMERIC|DECIMAL|SERIAL|OID|YEAR/i.test(t);
}

/** Virtualised, compact result grid. Handles 100k+ rows smoothly. */
export function ResultsGrid({ result }: { result: QueryResult }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [sel, setSel] = useState<{ r: number; c: number } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);
  const [sort, setSort] = useState<{ c: number; dir: 1 | -1 } | null>(null);

  // Initial column widths from header + first rows.
  useEffect(() => {
    setSel(null);
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

  const rows = useMemo(() => {
    if (!sort) return result.rows;
    const { c, dir } = sort;
    return [...result.rows].sort((a, b) => {
      const x = a[c], y = b[c];
      if (x === y) return 0;
      if (x === null) return 1;
      if (y === null) return -1;
      const nx = Number(x), ny = Number(y);
      if (!Number.isNaN(nx) && !Number.isNaN(ny)) return (nx - ny) * dir;
      return display(x).localeCompare(display(y)) * dir;
    });
  }, [result.rows, sort]);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 20,
  });

  const total = INDEX_W + widths.reduce((a, b) => a + b, 0);
  const numeric = result.columns.map((c) => isNumericType(c.data_type));

  // Keyboard nav + copy.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!sel) return;
    const move = (r: number, c: number) => {
      const nr = Math.max(0, Math.min(rows.length - 1, r));
      const nc = Math.max(0, Math.min(result.columns.length - 1, c));
      setSel({ r: nr, c: nc });
      virt.scrollToIndex(nr);
      e.preventDefault();
    };
    if (e.key === "ArrowDown") move(sel.r + 1, sel.c);
    else if (e.key === "ArrowUp") move(sel.r - 1, sel.c);
    else if (e.key === "ArrowLeft") move(sel.r, sel.c - 1);
    else if (e.key === "ArrowRight" || e.key === "Tab") move(sel.r, sel.c + 1);
    else if ((e.metaKey || e.ctrlKey) && e.key === "c") {
      const v = rows[sel.r]?.[sel.c];
      navigator.clipboard.writeText(v === null ? "" : display(v));
      e.preventDefault();
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

  return (
    <div
      ref={scrollRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="relative h-full overflow-auto font-mono text-xs outline-none"
    >
      <div style={{ width: total, minWidth: "100%" }}>
        {/* header */}
        <div className="sticky top-0 z-10 flex border-b border-line bg-panel" style={{ height: ROW_H }}>
          <div className="sticky left-0 z-10 shrink-0 border-r border-line bg-panel" style={{ width: INDEX_W }} />
          {result.columns.map((col, ci) => (
            <div
              key={ci}
              onClick={() =>
                setSort((s) => (s?.c === ci ? (s.dir === 1 ? { c: ci, dir: -1 } : null) : { c: ci, dir: 1 }))
              }
              className="group relative flex shrink-0 cursor-default items-center gap-1 border-r border-line px-2 font-sans hover:bg-hover"
              style={{ width: widths[ci] }}
              title={`${col.name} · ${col.data_type}`}
            >
              <span className="truncate font-medium text-fg">{col.name}</span>
              <span className="truncate text-[10px] text-subtle lowercase">{col.data_type}</span>
              {sort?.c === ci && <span className="ml-auto text-accent">{sort.dir === 1 ? "↑" : "↓"}</span>}
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
            return (
              <div
                key={vr.key}
                className={cx("absolute left-0 flex w-full", vr.index % 2 === 1 && "bg-hover/50")}
                style={{ height: ROW_H, transform: `translateY(${vr.start}px)` }}
              >
                <div
                  className="sticky left-0 shrink-0 border-r border-line bg-bg pr-2 text-right leading-[26px] text-subtle tabular-nums"
                  style={{ width: INDEX_W }}
                >
                  {vr.index + 1}
                </div>
                {row.map((v, ci) => {
                  const selected = sel?.r === vr.index && sel?.c === ci;
                  return (
                    <div
                      key={ci}
                      onMouseDown={() => setSel({ r: vr.index, c: ci })}
                      className={cx(
                        "shrink-0 truncate border-r border-b border-line/60 px-2 leading-[25px]",
                        numeric[ci] && "text-right tabular-nums",
                        v === null && "text-subtle italic",
                        typeof v === "boolean" && (v ? "text-success" : "text-muted"),
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
  );
}
