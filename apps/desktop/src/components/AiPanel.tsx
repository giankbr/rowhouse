import { Check, Loader2, Play, Sparkles, X } from "lucide-react";
import { useMemo } from "react";
import type { AiAnswer } from "../lib/api";
import { lineDiff } from "../lib/diff";
import { mod } from "../lib/types";
import { Button, Kbd, cx } from "./ui";

export type AiState =
  | { mode: "ask"; prompt: string; loading?: boolean; answer?: AiAnswer; error?: string; streamed?: string }
  | { mode: "fix" | "explain"; loading?: boolean; answer?: AiAnswer; error?: string; streamed?: string };

/** Inline AI strip under the editor: prompt input, then a diff proposal to accept or discard. */
export function AiPanel({
  state,
  currentSql,
  onChange,
  onSubmit,
  onAccept,
  onClose,
}: {
  state: AiState;
  currentSql: string;
  onChange: (s: AiState) => void;
  onSubmit: () => void;
  onAccept: (run: boolean) => void;
  onClose: () => void;
}) {
  const diff = useMemo(() => (state.answer?.sql ? lineDiff(currentSql, state.answer.sql) : []), [currentSql, state.answer]);
  const title = state.mode === "ask" ? "Ask AI" : state.mode === "fix" ? "Fix with AI" : "Explain";

  return (
    <div
      className="animate-fade shrink-0 border-b border-line bg-accent-soft/40"
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && state.answer?.sql) (e.preventDefault(), onAccept(true));
      }}
    >
      <div className="flex h-9 items-center gap-2 px-2">
        <Sparkles size={13} className="shrink-0 text-accent" />
        {state.mode === "ask" ? (
          <input
            autoFocus
            value={state.prompt}
            onChange={(e) => onChange({ ...state, prompt: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && !e.metaKey && !e.ctrlKey && state.prompt.trim() && onSubmit()}
            placeholder="Describe the query… e.g. top 10 customers by revenue this month"
            spellCheck={false}
            className="h-7 flex-1 bg-transparent text-xs outline-none placeholder:text-subtle focus-visible:outline-none"
          />
        ) : (
          <span className="flex-1 text-xs font-medium">{title}</span>
        )}
        {state.loading && <Loader2 size={13} className="animate-spin text-accent" />}
        {state.mode === "ask" && !state.loading && (
          <span className="hidden items-center gap-0.5 text-2xs text-subtle sm:flex">
            <Kbd>↵</Kbd> generate
          </span>
        )}
        <Button variant="ghost" size="icon" onClick={onClose} title="Close (Esc)">
          <X size={13} />
        </Button>
      </div>

      {state.error && (
        <div className="selectable mx-2 mb-2 rounded-md border border-danger/30 bg-danger/5 px-2.5 py-1.5 text-2xs text-danger">{state.error}</div>
      )}

      {state.loading && state.streamed && (
        <pre className="selectable mx-2 mb-2 max-h-56 overflow-auto rounded-lg border border-line bg-elevated px-3 py-2 font-mono text-[11.5px] leading-[1.55] whitespace-pre-wrap text-muted">
          {state.streamed.replace(/```[a-zA-Z]*\n?/g, "")}
          <span className="ml-0.5 inline-block h-3 w-1.5 animate-pulse bg-accent align-middle" />
        </pre>
      )}

      {state.answer && (
        <div className="mx-2 mb-2 overflow-hidden rounded-lg border border-line bg-elevated">
          {state.answer.sql && (
            <pre className="selectable max-h-56 overflow-auto py-1.5 font-mono text-[11.5px] leading-[1.55]">
              {diff.map((l, i) => (
                <div
                  key={i}
                  className={cx(
                    "px-3 whitespace-pre-wrap",
                    l.kind === "add" && "bg-success/12 text-fg",
                    l.kind === "del" && "bg-danger/10 text-danger line-through decoration-danger/40",
                    l.kind === "same" && "text-muted",
                  )}
                >
                  <span className="mr-2 inline-block w-2 text-subtle select-none">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
                  {l.text || " "}
                </div>
              ))}
            </pre>
          )}
          {state.answer.explanation && (
            <p className={cx("selectable px-3 py-2 text-xs text-muted", state.answer.sql && "border-t border-line")}>{state.answer.explanation}</p>
          )}
          {state.answer.sql && (
            <div className="flex items-center gap-1.5 border-t border-line bg-panel/60 px-2 py-1.5">
              <Button variant="primary" size="xs" onClick={() => onAccept(true)}>
                <Play size={11} fill="currentColor" /> Accept & run
              </Button>
              <Button size="xs" onClick={() => onAccept(false)}>
                <Check size={11} /> Accept
              </Button>
              <Button variant="ghost" size="xs" onClick={onClose}>
                Discard
              </Button>
              <span className="ml-auto flex items-center gap-0.5 text-2xs text-subtle">
                <Kbd>{mod}</Kbd>
                <Kbd>↵</Kbd> accept & run
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
