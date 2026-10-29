import { Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { ColumnInfo, Filter, FilterOp } from "../lib/api";
import { Button, cx } from "./ui";

const OPS: { value: FilterOp; label: string; noValue?: boolean }[] = [
  { value: "eq", label: "=" },
  { value: "ne", label: "≠" },
  { value: "contains", label: "contains" },
  { value: "starts-with", label: "starts with" },
  { value: "gt", label: ">" },
  { value: "ge", label: "≥" },
  { value: "lt", label: "<" },
  { value: "le", label: "≤" },
  { value: "in", label: "in (a, b, …)" },
  { value: "is-null", label: "is NULL", noValue: true },
  { value: "not-null", label: "is not NULL", noValue: true },
];

const isNumeric = (t: string) => /int|float|double|real|numeric|decimal|serial|money|year/i.test(t);

/** Compact filter rows; Enter or Apply pushes them down into SQL. */
export function FilterBar({
  columns,
  filters,
  onApply,
}: {
  columns: ColumnInfo[];
  filters: Filter[];
  onApply: (f: Filter[]) => void;
}) {
  const blank = (): Filter => {
    const c = columns[0];
    return { column: c?.name ?? "", op: "eq", value: "", numeric: !!c && isNumeric(c.data_type) };
  };
  const [rows, setRows] = useState<Filter[]>(filters.length ? filters : [blank()]);
  useEffect(() => setRows(filters.length ? filters : [blank()]), [filters]); // eslint-disable-line react-hooks/exhaustive-deps

  const set = (i: number, p: Partial<Filter>) => setRows((r) => r.map((x, j) => (j === i ? { ...x, ...p } : x)));
  const apply = (r = rows) => onApply(r.filter((f) => f.column && (OPS.find((o) => o.value === f.op)?.noValue || f.value !== "")));
  const field = "h-6 rounded-md border border-line bg-elevated px-1.5 text-xs outline-none focus:border-accent";

  return (
    <div className="flex shrink-0 flex-col gap-1 border-b border-line bg-panel/50 px-2 py-1.5">
      {rows.map((f, i) => {
        const noValue = OPS.find((o) => o.value === f.op)?.noValue;
        return (
          <div key={i} className="flex items-center gap-1.5">
            <span className="w-10 text-right text-[10.5px] text-subtle">{i === 0 ? "WHERE" : "AND"}</span>
            <select
              value={f.column}
              onChange={(e) => {
                const c = columns.find((x) => x.name === e.target.value);
                set(i, { column: e.target.value, numeric: !!c && isNumeric(c.data_type) });
              }}
              className={cx(field, "w-40 font-mono")}
            >
              {columns.map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name}
                </option>
              ))}
            </select>
            <select value={f.op} onChange={(e) => set(i, { op: e.target.value as FilterOp })} className={cx(field, "w-28")}>
              {OPS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            {!noValue && (
              <input
                value={f.value}
                autoFocus={i === rows.length - 1 && !f.value}
                onChange={(e) => set(i, { value: e.target.value })}
                onKeyDown={(e) => e.key === "Enter" && apply()}
                placeholder={f.op === "in" ? "a, b, c" : "value"}
                spellCheck={false}
                className={cx(field, "min-w-0 flex-1 font-mono")}
              />
            )}
            {noValue && <span className="flex-1" />}
            <Button
              variant="ghost"
              size="icon"
              title="Remove"
              onClick={() => {
                const next = rows.filter((_, j) => j !== i);
                setRows(next.length ? next : [blank()]);
                if (filters.length) apply(next);
              }}
            >
              <X size={12} />
            </Button>
          </div>
        );
      })}
      <div className="flex items-center gap-1.5 pl-11">
        <Button variant="ghost" size="xs" onClick={() => setRows((r) => [...r, blank()])}>
          <Plus size={11} /> Condition
        </Button>
        <span className="flex-1" />
        {filters.length > 0 && (
          <Button variant="ghost" size="xs" onClick={() => (setRows([blank()]), onApply([]))}>
            Clear
          </Button>
        )}
        <Button variant="primary" size="xs" onClick={() => apply()}>
          Apply
        </Button>
      </div>
    </div>
  );
}
