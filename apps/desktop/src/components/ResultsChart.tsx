import { AreaChart, BarChart3, Copy, Download, LineChart, ScatterChart } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, dialogs, type Cell, type QueryResult } from "../lib/api";
import { Button, Segmented, cx } from "./ui";

export type ChartType = "bar" | "line" | "area" | "scatter";
export interface ChartSpec {
  type: ChartType;
  x: string;
  y: string[];
}

const MAX_SERIES = 8;
const MAX_POINTS = 2000;
const MAX_BARS = 120;

const num = (v: Cell): number | null => {
  if (typeof v === "number") return Number.isFinite(v) ? v : null;
  if (typeof v === "string" && v.trim() !== "" && /^-?\d+(\.\d+)?(e[+-]?\d+)?$/i.test(v.trim())) return Number(v);
  if (typeof v === "boolean") return v ? 1 : 0;
  return null;
};

/** Columns where (almost) every non-null value is a number. */
export function numericColumns(r: QueryResult): boolean[] {
  return r.columns.map((_, c) => {
    let seen = 0;
    let ok = 0;
    for (let i = 0; i < Math.min(r.rows.length, 200); i++) {
      const v = r.rows[i][c];
      if (v === null || v === undefined) continue;
      seen++;
      if (typeof v !== "boolean" && num(v) !== null) ok++;
    }
    return seen > 0 && ok / seen > 0.9;
  });
}

/** Sensible first chart for a result: first text/date column on x, numbers on y. */
export function defaultSpec(r: QueryResult): ChartSpec | null {
  const numeric = numericColumns(r);
  const names = r.columns.map((c) => c.name);
  const xi = numeric.findIndex((n) => !n);
  const x = xi >= 0 ? names[xi] : names[0];
  const y = names.filter((n, i) => numeric[i] && n !== x && !/^id$|_id$/i.test(n)).slice(0, 1);
  const y2 = y.length ? y : names.filter((n, i) => numeric[i] && n !== x).slice(0, 1);
  if (!y2.length) return null;
  const xType = r.columns[names.indexOf(x)]?.data_type.toLowerCase() ?? "";
  const timeLike = /date|time/.test(xType) || /date|day|month|week|year|time|_at$/i.test(x);
  return { type: timeLike ? "line" : "bar", x, y: y2 };
}

function niceTicks(lo: number, hi: number, count = 5): number[] {
  if (lo === hi) (hi = lo + 1), (lo = Math.min(0, lo));
  const step0 = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(step0));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= step0) ?? 10 * mag;
  const start = Math.floor(lo / step) * step;
  const out: number[] = [];
  for (let v = start; v <= hi + step * 0.5; v += step) out.push(Number(v.toPrecision(12)));
  return out;
}

const fmt = (v: number) => {
  const a = Math.abs(v);
  if (a >= 1e9) return `${+(v / 1e9).toFixed(1)}B`;
  if (a >= 1e6) return `${+(v / 1e6).toFixed(1)}M`;
  if (a >= 1e4) return `${+(v / 1e3).toFixed(1)}k`;
  return Number.isInteger(v) ? v.toLocaleString() : (+v.toFixed(2)).toLocaleString();
};
const label = (v: Cell) => (v === null || v === undefined ? "∅" : typeof v === "object" ? JSON.stringify(v) : String(v));
const short = (s: string, n = 14) => (s.length > n ? s.slice(0, n - 1) + "…" : s);

function tokens() {
  const cs = getComputedStyle(document.documentElement);
  const g = (n: string) => cs.getPropertyValue(n).trim();
  return {
    series: Array.from({ length: MAX_SERIES }, (_, i) => g(`--series-${i + 1}`) || "#2a78d6"),
    fg: g("--fg"),
    muted: g("--muted"),
    subtle: g("--subtle"),
    line: g("--line") || "rgba(0,0,0,.08)",
    surface: g("--bg"),
    elevated: g("--elevated"),
  };
}

export function ResultsChart({
  result,
  spec,
  onSpec,
  title,
  onToast,
  compact = false,
  extraActions,
}: {
  /** no toolbar: just the plot (dashboard tiles) */
  compact?: boolean;
  /** extra buttons at the end of the toolbar */
  extraActions?: React.ReactNode;
  result: QueryResult;
  spec?: ChartSpec;
  onSpec: (s: ChartSpec) => void;
  title: string;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const numeric = useMemo(() => numericColumns(result), [result]);
  const names = result.columns.map((c) => c.name);
  const valid = spec && names.includes(spec.x) && spec.y.every((y) => names.includes(y)) && spec.y.length > 0;
  const fallback = useMemo(() => defaultSpec(result), [result]);
  const s = valid ? spec! : fallback;

  const box = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ w: 800, h: 360 });
  const [hover, setHover] = useState<{ i: number; px: number; py: number } | null>(null);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setSize({ w: Math.max(compact ? 200 : 320, e.contentRect.width), h: Math.max(compact ? 110 : 220, e.contentRect.height) }));
    ro.observe(el);
    return () => ro.disconnect();
  }, [!!fallback || !!valid]);

  if (!s) {
    return <div className="flex h-full items-center justify-center text-xs text-subtle">No numeric columns to chart</div>;
  }
  const set = (p: Partial<ChartSpec>) => onSpec({ ...s, ...p });
  const xi = names.indexOf(s.x);
  const yi = s.y.map((y) => names.indexOf(y));
  const scatter = s.type === "scatter";
  const cap = s.type === "bar" ? MAX_BARS : MAX_POINTS;
  const rows = result.rows.slice(0, cap);
  const cut = result.rows.length > cap;

  const t = tokens();
  const { w, h } = size;
  const legend = s.y.length > 1;
  const m = compact ? { top: legend ? 28 : 10, right: 12, bottom: 22, left: 44 } : { top: legend ? 34 : 16, right: 20, bottom: 38, left: 56 };
  const iw = w - m.left - m.right;
  const ih = h - m.top - m.bottom;

  // y domain (bars and areas include 0)
  let lo = Infinity;
  let hi = -Infinity;
  for (const r of rows)
    for (const c of yi) {
      const v = num(r[c]);
      if (v === null) continue;
      lo = Math.min(lo, v);
      hi = Math.max(hi, v);
    }
  if (!Number.isFinite(lo)) (lo = 0), (hi = 1);
  if (s.type !== "line" && s.type !== "scatter") (lo = Math.min(0, lo)), (hi = Math.max(0, hi));
  const yt = niceTicks(lo, hi);
  const y0 = yt[0];
  const y1 = yt[yt.length - 1];
  const Y = (v: number) => m.top + ih - ((v - y0) / (y1 - y0 || 1)) * ih;

  // x scale: numeric for scatter, ordinal otherwise
  let X: (i: number) => number;
  let xt: { at: number; text: string }[] = [];
  let band = 0;
  if (scatter) {
    const xs = rows.map((r) => num(r[xi])).filter((v): v is number => v !== null);
    const ticks = xs.length ? niceTicks(Math.min(...xs), Math.max(...xs)) : [0, 1];
    const a = ticks[0];
    const b = ticks[ticks.length - 1];
    X = (i) => m.left + (((num(rows[i][xi]) ?? a) - a) / (b - a || 1)) * iw;
    xt = ticks.map((v) => ({ at: m.left + ((v - a) / (b - a || 1)) * iw, text: fmt(v) }));
  } else {
    const n = Math.max(1, rows.length);
    band = iw / n;
    X = s.type === "bar" ? (i) => m.left + band * i + band / 2 : (i) => m.left + (n === 1 ? iw / 2 : (i / (n - 1)) * iw);
    const every = Math.max(1, Math.ceil(n / Math.max(1, Math.floor(iw / 84))));
    xt = rows.flatMap((r, i) => (i % every === 0 ? [{ at: X(i), text: short(label(r[xi])) }] : []));
  }

  const series = s.y.map((name, k) => ({ name, c: yi[k], color: t.series[k % MAX_SERIES] }));
  const base = Y(Math.max(y0, Math.min(0, y1)));

  const path = (c: number) => {
    let d = "";
    let pen = false;
    rows.forEach((r, i) => {
      const v = num(r[c]);
      if (v === null) return (pen = false);
      d += `${pen ? "L" : "M"}${X(i).toFixed(1)},${Y(v).toFixed(1)}`;
      pen = true;
    });
    return d;
  };
  const area = (c: number) => {
    const pts = rows.map((r, i) => [X(i), num(r[c])] as const).filter((p) => p[1] !== null) as [number, number][];
    if (!pts.length) return "";
    return `M${pts[0][0]},${base}` + pts.map(([x, v]) => `L${x.toFixed(1)},${Y(v).toFixed(1)}`).join("") + `L${pts[pts.length - 1][0]},${base}Z`;
  };

  // bars: grouped, 2px gap between neighbours, 4px rounded data end
  const groupW = Math.max(2, band * 0.72);
  const barW = Math.max(1, (groupW - 2 * (series.length - 1)) / series.length);
  const bar = (x: number, v: number) => {
    const yv = Y(v);
    const top = Math.min(yv, base);
    const hgt = Math.abs(base - yv);
    const r = Math.min(4, barW / 2, hgt);
    if (hgt < 0.5) return "";
    return v >= 0
      ? `M${x},${base}V${top + r}Q${x},${top} ${x + r},${top}H${x + barW - r}Q${x + barW},${top} ${x + barW},${top + r}V${base}Z`
      : `M${x},${base}V${top + hgt - r}Q${x},${top + hgt} ${x + r},${top + hgt}H${x + barW - r}Q${x + barW},${top + hgt} ${x + barW},${top + hgt - r}V${base}Z`;
  };

  const onMove = (e: React.MouseEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    if (px < m.left - 4 || px > w - m.right + 4 || py < m.top - 4 || py > h - m.bottom + 4 || !rows.length) return setHover(null);
    let best = 0;
    let bd = Infinity;
    for (let i = 0; i < rows.length; i++) {
      const d = scatter ? Math.hypot(X(i) - px, Y(num(rows[i][series[0].c]) ?? 0) - py) : Math.abs(X(i) - px);
      if (d < bd) (bd = d), (best = i);
    }
    if (scatter && bd > 24) return setHover(null);
    setHover({ i: best, px, py });
  };

  const exportSvg = (): string => {
    const el = svgRef.current!.cloneNode(true) as SVGSVGElement;
    el.querySelectorAll("[data-hover]").forEach((n) => n.remove());
    el.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    el.setAttribute("font-family", getComputedStyle(document.body).fontFamily);
    el.insertAdjacentHTML("afterbegin", `<rect width="100%" height="100%" fill="${t.surface}"/>`);
    return new XMLSerializer().serializeToString(el);
  };
  const toPng = async (): Promise<Blob> => {
    const img = new Image();
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(exportSvg());
    await img.decode();
    const scale = 2;
    const cv = document.createElement("canvas");
    cv.width = w * scale;
    cv.height = h * scale;
    const ctx = cv.getContext("2d")!;
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, w, h);
    return new Promise((res, rej) => cv.toBlob((b) => (b ? res(b) : rej("couldn't render the PNG")), "image/png"));
  };
  const save = async (fmtx: "png" | "svg") => {
    const path = await dialogs.save(`${title.replace(/[^\w.-]+/g, "-") || "chart"}.${fmtx}`, fmtx);
    if (!path) return;
    try {
      if (fmtx === "svg") await api.writeTextFile(path, exportSvg());
      else {
        const buf = new Uint8Array(await (await toPng()).arrayBuffer());
        let bin = "";
        for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
        await api.writeBase64File(path, btoa(bin));
      }
      onToast(`Saved ${path.split(/[\\/]/).pop()}`, "ok");
    } catch (e) {
      onToast(String(e));
    }
  };
  const copy = async () => {
    try {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": toPng() })]);
      onToast("Chart copied as PNG", "ok");
    } catch (e) {
      onToast(String(e));
    }
  };

  const toggleY = (name: string) => {
    const has = s.y.includes(name);
    if (has && s.y.length === 1) return;
    if (!has && s.y.length >= MAX_SERIES) return onToast(`Up to ${MAX_SERIES} series`);
    set({ y: has ? s.y.filter((y) => y !== name) : [...s.y, name] });
  };

  const hv = hover && rows[hover.i];
  const tipLeft = hover ? (hover.px > w * 0.6 ? hover.px - 12 : hover.px + 12) : 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {!compact && <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2.5 text-2xs">
        <Segmented
          value={s.type}
          onChange={(type) => set({ type: type as ChartType })}
          options={[
            { value: "bar", label: <BarChart3 size={12} aria-label="Bar" /> },
            { value: "line", label: <LineChart size={12} aria-label="Line" /> },
            { value: "area", label: <AreaChart size={12} aria-label="Area" /> },
            { value: "scatter", label: <ScatterChart size={12} aria-label="Scatter" /> },
          ]}
        />
        <span className="ml-1 text-subtle">X</span>
        <select value={s.x} onChange={(e) => set({ x: e.target.value, y: s.y.filter((y) => y !== e.target.value).length ? s.y.filter((y) => y !== e.target.value) : s.y })} className="h-6 max-w-40 rounded-md border border-line bg-elevated px-1 text-2xs text-fg outline-none">
          {names.map((n, i) => (
            <option key={i} value={n} disabled={scatter && !numeric[i]}>
              {n}
            </option>
          ))}
        </select>
        <span className="ml-1 text-subtle">Y</span>
        <div className="flex min-w-0 flex-1 gap-1 overflow-x-auto">
          {names.map((n, i) =>
            numeric[i] && n !== s.x ? (
              <button
                key={i}
                onClick={() => toggleY(n)}
                className={cx(
                  "flex h-6 shrink-0 items-center gap-1.5 rounded-md border px-2",
                  s.y.includes(n) ? "border-line-strong bg-elevated text-fg" : "border-transparent text-muted hover:bg-hover",
                )}
              >
                {s.y.includes(n) && <span className="size-2 rounded-[2px]" style={{ background: t.series[s.y.indexOf(n)] }} />}
                {n}
              </button>
            ) : null,
          )}
        </div>
        {cut && <span className="shrink-0 text-subtle">first {cap.toLocaleString()} rows</span>}
        <Button size="xs" variant="ghost" onClick={copy} title="Copy as PNG">
          <Copy size={11} />
        </Button>
        <Button size="xs" variant="ghost" onClick={() => save("png")} title="Save as PNG">
          <Download size={11} /> PNG
        </Button>
        <Button size="xs" variant="ghost" onClick={() => save("svg")} title="Save as SVG">
          SVG
        </Button>
        {extraActions}
      </div>}

      <div ref={box} className="relative min-h-0 flex-1">
        <svg ref={svgRef} width={w} height={h} viewBox={`0 0 ${w} ${h}`} className="absolute inset-0 select-none" onMouseMove={onMove} onMouseLeave={() => setHover(null)} role="img" aria-label={`${s.type} chart of ${s.y.join(", ")} by ${s.x}`}>
          {/* grid + y axis */}
          {yt.map((v) => (
            <g key={v}>
              <line x1={m.left} x2={w - m.right} y1={Y(v)} y2={Y(v)} stroke={t.line} strokeWidth={1} />
              <text x={m.left - 8} y={Y(v)} dy="0.32em" textAnchor="end" fontSize={10.5} fill={t.subtle} style={{ fontVariantNumeric: "tabular-nums" }}>
                {fmt(v)}
              </text>
            </g>
          ))}
          {xt.map((tk, i) => (
            <text key={i} x={tk.at} y={h - m.bottom + (compact ? 14 : 16)} textAnchor="middle" fontSize={10.5} fill={t.subtle}>
              {tk.text}
            </text>
          ))}
          {!compact && (
            <text x={m.left + iw / 2} y={h - 6} textAnchor="middle" fontSize={10.5} fill={t.muted}>
              {s.x}
            </text>
          )}
          {!legend && !compact && (
            <text x={12} y={m.top + ih / 2} transform={`rotate(-90 12 ${m.top + ih / 2})`} textAnchor="middle" fontSize={10.5} fill={t.muted}>
              {s.y[0]}
            </text>
          )}

          {/* legend */}
          {legend && (
            <g>
              {(() => {
                let x = m.left;
                return series.map((sr) => {
                  const g = (
                    <g key={sr.name} transform={`translate(${x},12)`}>
                      <rect width={10} height={10} rx={2} fill={sr.color} />
                      <text x={15} y={5} dy="0.32em" fontSize={11} fill={t.muted}>
                        {sr.name}
                      </text>
                    </g>
                  );
                  x += 28 + sr.name.length * 6.4;
                  return g;
                });
              })()}
            </g>
          )}

          {/* hover band for bars */}
          {hover && s.type === "bar" && <rect data-hover x={m.left + band * hover.i} y={m.top} width={band} height={ih} fill={t.line} opacity={0.6} />}

          {s.type === "bar" &&
            series.map((sr, k) => (
              <g key={sr.name}>
                {rows.map((r, i) => {
                  const v = num(r[sr.c]);
                  if (v === null) return null;
                  const x = m.left + band * i + (band - groupW) / 2 + k * (barW + 2);
                  return <path key={i} d={bar(x, v)} fill={sr.color} />;
                })}
              </g>
            ))}
          {s.type === "area" && series.map((sr) => <path key={sr.name + "a"} d={area(sr.c)} fill={sr.color} opacity={series.length > 1 ? 0.14 : 0.2} />)}
          {(s.type === "line" || s.type === "area") &&
            series.map((sr) => <path key={sr.name} d={path(sr.c)} fill="none" stroke={sr.color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />)}
          {(s.type === "line" || s.type === "area") &&
            rows.length <= 40 &&
            series.map((sr) =>
              rows.map((r, i) => {
                const v = num(r[sr.c]);
                return v === null ? null : <circle key={sr.name + i} cx={X(i)} cy={Y(v)} r={3} fill={sr.color} stroke={t.surface} strokeWidth={1.5} />;
              }),
            )}
          {scatter &&
            series.map((sr) =>
              rows.map((r, i) => {
                const v = num(r[sr.c]);
                return v === null || num(r[xi]) === null ? null : <circle key={sr.name + i} cx={X(i)} cy={Y(v)} r={4} fill={sr.color} fillOpacity={0.75} stroke={t.surface} strokeWidth={1.5} />;
              }),
            )}
          {/* baseline */}
          <line x1={m.left} x2={w - m.right} y1={base} y2={base} stroke={t.subtle} strokeOpacity={0.5} strokeWidth={1} />

          {/* crosshair */}
          {hover && (s.type === "line" || s.type === "area") && (
            <g data-hover>
              <line x1={X(hover.i)} x2={X(hover.i)} y1={m.top} y2={m.top + ih} stroke={t.subtle} strokeDasharray="3 3" />
              {series.map((sr) => {
                const v = num(rows[hover.i][sr.c]);
                return v === null ? null : <circle key={sr.name} cx={X(hover.i)} cy={Y(v)} r={4.5} fill={sr.color} stroke={t.surface} strokeWidth={2} />;
              })}
            </g>
          )}
        </svg>

        {hv && (
          <div
            className="pointer-events-none absolute z-10 min-w-36 rounded-lg border border-line bg-elevated px-2.5 py-2 text-2xs shadow-pop"
            style={{ left: tipLeft, top: Math.max(4, hover!.py - 20), transform: hover!.px > w * 0.6 ? "translateX(-100%)" : undefined }}
          >
            <div className="mb-1 font-medium text-fg">
              {s.x}: {label(hv[xi])}
            </div>
            {series.map((sr) => (
              <div key={sr.name} className="flex items-center gap-2 leading-5">
                <span className="size-2 rounded-[2px]" style={{ background: sr.color }} />
                <span className="text-muted">{sr.name}</span>
                <span className="ml-auto pl-3 font-medium text-fg tabular-nums">{num(hv[sr.c]) === null ? "—" : (num(hv[sr.c]) as number).toLocaleString()}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
