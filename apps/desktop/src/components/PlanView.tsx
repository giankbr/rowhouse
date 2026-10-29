import { AlertTriangle, ChevronRight, Code2, Loader2 } from "lucide-react";
import { useMemo, useState } from "react";
import type { Plan, PlanNode } from "../lib/api";
import { Button, Switch, cx } from "./ui";

const fmt = (n: number | null | undefined, digits = 0) =>
  n == null ? "—" : n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e4 ? `${(n / 1e3).toFixed(1)}k` : n.toFixed(digits);

/** Self time = node time minus children (analyze) or cost minus children cost. */
function selfOf(n: PlanNode, analyzed: boolean) {
  const v = (x: PlanNode) => (analyzed ? (x.actual_ms ?? 0) * (x.loops ?? 1) : (x.cost ?? 0));
  return Math.max(0, v(n) - n.children.reduce((a, c) => a + v(c), 0));
}

function flatten(nodes: PlanNode[], depth = 0, out: { n: PlanNode; depth: number; id: string }[] = [], prefix = "") {
  nodes.forEach((n, i) => {
    const id = `${prefix}${i}`;
    out.push({ n, depth, id });
    flatten(n.children, depth + 1, out, `${id}.`);
  });
  return out;
}

export function PlanView({
  plan,
  loading,
  error,
  canAnalyze,
  analyze,
  onAnalyze,
}: {
  plan?: Plan;
  loading?: boolean;
  error?: string;
  canAnalyze: boolean;
  analyze: boolean;
  onAnalyze: (v: boolean) => void;
}) {
  const [raw, setRaw] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const rows = useMemo(() => (plan ? flatten(plan.nodes) : []), [plan]);
  const selfs = useMemo(() => rows.map((r) => selfOf(r.n, !!plan?.analyzed)), [rows, plan]);
  const max = Math.max(1e-9, ...selfs);
  const total = Math.max(1e-9, selfs.reduce((a, b) => a + b, 0));
  const hidden = (id: string) => [...collapsed].some((c) => id.startsWith(c + "."));
  const warnings = rows.flatMap((r) => r.n.warnings);

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-8 shrink-0 items-center gap-3 border-b border-line bg-panel/60 px-3 text-2xs text-muted">
        <span className="font-medium text-fg">Query plan</span>
        {plan?.analyzed && (
          <span>
            executed in <span className="font-mono text-fg">{plan.execution_ms?.toFixed(1)} ms</span> · planned in{" "}
            <span className="font-mono">{plan.planning_ms?.toFixed(1)} ms</span>
          </span>
        )}
        {warnings.length > 0 && (
          <span className="flex items-center gap-1 text-warning">
            <AlertTriangle size={11} /> {warnings.length} hint{warnings.length > 1 ? "s" : ""}
          </span>
        )}
        <span className="flex-1" />
        {canAnalyze && (
          <Switch checked={analyze} onChange={onAnalyze} label={<span className="text-2xs text-muted">ANALYZE (runs the query, read-only)</span>} />
        )}
        <Button variant="ghost" size="xs" onClick={() => setRaw((r) => !r)} disabled={!plan}>
          <Code2 size={11} /> {raw ? "Tree" : "Raw"}
        </Button>
      </div>

      {loading && (
        <div className="flex flex-1 items-center justify-center gap-2 text-xs text-muted">
          <Loader2 size={14} className="animate-spin" /> Explaining…
        </div>
      )}
      {error && !loading && (
        <div className="selectable m-3 rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-xs whitespace-pre-wrap text-danger">{error}</div>
      )}
      {plan && !loading && !error && (raw || plan.nodes.length === 0) && (
        <pre className="selectable flex-1 overflow-auto p-3 font-mono text-[11.5px] leading-relaxed">{plan.raw}</pre>
      )}
      {plan && !loading && !error && !raw && plan.nodes.length > 0 && (
        <div className="flex-1 overflow-auto">
          <div className="sticky top-0 z-10 grid grid-cols-[1fr_88px_80px_88px_160px] border-b border-line bg-panel px-3 py-1 text-[10.5px] font-medium text-subtle">
            <span>Operation</span>
            <span className="text-right">{plan.analyzed ? "rows" : "est. rows"}</span>
            <span className="text-right">cost</span>
            <span className="text-right">{plan.analyzed ? "time" : ""}</span>
            <span className="pl-3">{plan.analyzed ? "self time" : "self cost"}</span>
          </div>
          {rows.map(({ n, depth, id }) => {
            if (hidden(id)) return null;
            const self = selfOf(n, plan.analyzed);
            const pct = self / total; // share of the whole query
            const width = self / max;
            const open = !collapsed.has(id);
            return (
              <div key={id} className="group grid grid-cols-[1fr_88px_80px_88px_160px] items-start border-b border-line/50 px-3 py-1.5 text-xs hover:bg-hover">
                <div className="flex min-w-0 items-start gap-1" style={{ paddingLeft: depth * 16 }}>
                  <button
                    className={cx("mt-0.5 shrink-0 text-subtle", n.children.length === 0 && "invisible")}
                    onClick={() =>
                      setCollapsed((c) => {
                        const x = new Set(c);
                        if (x.has(id)) x.delete(id);
                        else x.add(id);
                        return x;
                      })
                    }
                  >
                    <ChevronRight size={12} className={cx("transition-transform", open && "rotate-90")} />
                  </button>
                  <div className="min-w-0">
                    <div className="font-medium">{n.label}</div>
                    {n.detail && <div className="selectable truncate font-mono text-[11px] text-muted" title={n.detail}>{n.detail}</div>}
                    {n.warnings.map((w) => (
                      <div key={w} className="mt-0.5 flex items-center gap-1 text-[11px] text-warning">
                        <AlertTriangle size={10} /> {w}
                      </div>
                    ))}
                  </div>
                </div>
                <span className="text-right font-mono tabular-nums">
                  {fmt(plan.analyzed ? n.actual_rows : n.rows)}
                  {plan.analyzed && n.loops && n.loops > 1 && <span className="text-subtle"> ×{fmt(n.loops)}</span>}
                </span>
                <span className="text-right font-mono text-muted tabular-nums">{fmt(n.cost, 1)}</span>
                <span className="text-right font-mono tabular-nums">{plan.analyzed ? `${fmt(n.actual_ms, 2)} ms` : ""}</span>
                <span className="flex items-center gap-2 pl-3">
                  <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-hover">
                    <span
                      className={cx("block h-full rounded-full", pct > 0.5 ? "bg-danger" : pct > 0.2 ? "bg-warning" : "bg-accent")}
                      style={{ width: `${Math.max(2, width * 100)}%` }}
                    />
                  </span>
                  <span className="w-9 text-right font-mono text-[10.5px] text-subtle tabular-nums">{Math.round(pct * 100)}%</span>
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
