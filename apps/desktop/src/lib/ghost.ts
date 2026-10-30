// Inline AI suggestions ("ghost text") for CodeMirror.
// After the user pauses typing, `fetch(before, after)` is asked for a
// continuation; it is shown greyed out at the cursor. Tab accepts, Esc or any
// other edit dismisses. Requests that come back after the doc changed are dropped.
import { Prec, StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, keymap, ViewPlugin, WidgetType, type ViewUpdate } from "@codemirror/view";

type Ghost = { pos: number; text: string } | null;

const setGhost = StateEffect.define<Ghost>();

class GhostWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }
  eq(other: GhostWidget) {
    return other.text === this.text;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-ghost";
    el.textContent = this.text;
    return el;
  }
}

const ghostField = StateField.define<Ghost>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setGhost)) return e.value;
    // any edit or cursor move invalidates the suggestion
    return tr.docChanged || tr.selection ? null : value;
  },
  provide: (f) =>
    EditorView.decorations.from(f, (g) =>
      g && g.text ? Decoration.set([Decoration.widget({ widget: new GhostWidget(g.text), side: 1 }).range(g.pos)]) : Decoration.none,
    ),
});

const acceptGhost = (view: EditorView) => {
  const g = view.state.field(ghostField, false);
  if (!g?.text) return false;
  view.dispatch({
    changes: { from: g.pos, insert: g.text },
    selection: { anchor: g.pos + g.text.length },
    effects: setGhost.of(null),
    userEvent: "input.complete",
  });
  return true;
};

const dismissGhost = (view: EditorView) => {
  if (!view.state.field(ghostField, false)) return false;
  view.dispatch({ effects: setGhost.of(null) });
  return true;
};

export function ghostText(fetch: (before: string, after: string) => Promise<string>, delayMs = 600): Extension {
  const plugin = ViewPlugin.fromClass(
    class {
      timer: ReturnType<typeof setTimeout> | null = null;
      seq = 0;
      constructor(readonly view: EditorView) {}
      update(u: ViewUpdate) {
        if (!u.docChanged || u.transactions.some((t) => t.isUserEvent("input.complete"))) return;
        if (this.timer) clearTimeout(this.timer);
        const seq = ++this.seq;
        this.timer = setTimeout(async () => {
          const { state } = this.view;
          const sel = state.selection.main;
          if (!sel.empty) return;
          const pos = sel.head;
          const line = state.doc.lineAt(pos);
          // only at the end of a line, and only when there's something to go on
          if (state.sliceDoc(pos, line.to).trim() !== "" || state.sliceDoc(0, pos).trim().length < 3) return;
          try {
            const text = await fetch(state.sliceDoc(0, pos), state.sliceDoc(pos));
            if (seq !== this.seq || this.view.state.selection.main.head !== pos || !text) return;
            this.view.dispatch({ effects: setGhost.of({ pos, text }) });
          } catch {
            /* suggestions are best-effort */
          }
        }, delayMs);
      }
      destroy() {
        if (this.timer) clearTimeout(this.timer);
      }
    },
  );
  return [
    ghostField,
    plugin,
    Prec.highest(keymap.of([{ key: "Tab", run: acceptGhost }, { key: "Escape", run: dismissGhost }])),
    EditorView.baseTheme({ ".cm-ghost": { color: "var(--subtle)", fontStyle: "italic", opacity: "0.85" } }),
  ];
}
