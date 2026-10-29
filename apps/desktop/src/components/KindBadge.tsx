import { kindInfo, type DbKind } from "../lib/api";
import { cx } from "./ui";

/** Round two-letter database badge (TablePlus-style), sized for dense lists. */
export function KindBadge({ kind, size = 16, className }: { kind: DbKind; size?: number; className?: string }) {
  const k = kindInfo(kind);
  return (
    <span
      className={cx("inline-flex shrink-0 items-center justify-center rounded-full font-semibold text-white", className)}
      style={{ width: size, height: size, background: k.color, fontSize: Math.round(size * 0.42), letterSpacing: "-0.02em" }}
      title={k.label}
    >
      {k.abbr}
    </span>
  );
}
