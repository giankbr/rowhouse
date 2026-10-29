// Tiny compact UI kit. Sized for dense desktop tools: 24–28px controls, 12–13px text.
import clsx from "clsx";
import { forwardRef, useEffect, useRef, useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from "react";

export const cx = clsx;

type Variant = "default" | "primary" | "ghost" | "danger";
type Size = "xs" | "sm" | "icon";

export const Button = forwardRef<
  HTMLButtonElement,
  ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: Size }
>(function Button({ variant = "default", size = "sm", className, ...props }, ref) {
  return (
    <button
      ref={ref}
      className={cx(
        "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-md font-medium whitespace-nowrap",
        "transition-[background,color,box-shadow] duration-100 disabled:pointer-events-none disabled:opacity-45",
        size === "xs" && "h-6 px-2 text-2xs",
        size === "sm" && "h-7 px-2.5 text-xs",
        size === "icon" && "size-6 text-muted",
        variant === "default" && "border border-line bg-elevated shadow-[0_1px_0_rgb(0_0_0/0.03)] hover:bg-hover",
        variant === "primary" && "bg-accent text-accent-fg hover:brightness-110",
        variant === "ghost" && "hover:bg-hover hover:text-fg",
        variant === "danger" && "text-danger hover:bg-danger/10",
        className,
      )}
      {...props}
    />
  );
});

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(function Input(
  { className, ...props },
  ref,
) {
  return (
    <input
      ref={ref}
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      className={cx(
        "h-7 w-full rounded-md border border-line bg-elevated px-2 text-xs text-fg placeholder:text-subtle",
        "outline-none transition-shadow focus:border-accent focus:ring-2 focus:ring-accent-soft",
        className,
      )}
      {...props}
    />
  );
});

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-2xs font-medium text-muted">{label}</span>
      {children}
      {hint && <span className="text-2xs text-subtle">{hint}</span>}
    </label>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-4 min-w-4 items-center justify-center rounded border border-line bg-panel px-1 font-sans text-[10px] text-muted">
      {children}
    </kbd>
  );
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex items-center gap-2 text-xs text-fg"
    >
      <span
        className={cx(
          "relative h-4 w-7 shrink-0 rounded-full transition-colors duration-150",
          checked ? "bg-accent" : "bg-line-strong",
        )}
      >
        <span
          className={cx(
            "absolute top-0.5 left-0.5 size-3 rounded-full bg-white shadow transition-transform duration-150",
            checked ? "translate-x-3" : "translate-x-0",
          )}
        />
      </span>
      {label}
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: ReactNode }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="inline-flex rounded-md bg-panel p-0.5 ring-1 ring-line">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cx(
            "h-6 rounded-[5px] px-2.5 text-2xs font-medium transition-colors",
            value === o.value ? "bg-elevated text-fg shadow-[0_1px_2px_rgb(0_0_0/0.08)]" : "text-muted hover:text-fg",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Modal({
  open,
  onClose,
  title,
  children,
  footer,
  width = 440,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  if (!open) return null;
  return (
    <div
      className="animate-fade fixed inset-0 z-50 flex items-start justify-center bg-black/25 pt-[12vh] backdrop-blur-[2px]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
    >
      <div className="animate-pop overflow-hidden rounded-xl bg-elevated shadow-pop" style={{ width }}>
        <div className="flex h-10 items-center border-b border-line px-4 text-sm font-semibold">{title}</div>
        <div className="max-h-[68vh] overflow-y-auto p-4">{children}</div>
        {footer && <div className="flex items-center gap-2 border-t border-line bg-panel/60 px-4 py-2.5">{footer}</div>}
      </div>
    </div>
  );
}

export const TAG_COLORS: Record<string, string> = {
  red: "#ef4444",
  orange: "#f97316",
  yellow: "#eab308",
  green: "#22c55e",
  blue: "#3b82f6",
  violet: "#8b5cf6",
  gray: "#a1a1aa",
};

export function Dot({ color, className }: { color?: string | null; className?: string }) {
  return (
    <span
      className={cx("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ background: color ? TAG_COLORS[color] ?? color : "var(--subtle)" }}
    />
  );
}

/** Minimal dropdown menu: a trigger button + a popover list. */
export function Menu({
  label,
  items,
  disabled,
}: {
  label: ReactNode;
  items: { label: ReactNode; hint?: string; onSelect: () => void }[];
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <Button variant="ghost" size="xs" disabled={disabled} onClick={() => setOpen((o) => !o)}>
        {label}
      </Button>
      {open && (
        <div className="animate-pop absolute top-7 right-0 z-30 min-w-44 rounded-lg bg-elevated p-1 shadow-pop">
          {items.map((it, i) => (
            <button
              key={i}
              onClick={() => (setOpen(false), it.onSelect())}
              className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-xs hover:bg-hover"
            >
              {it.label}
              {it.hint && <span className="ml-auto text-2xs text-subtle">{it.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
