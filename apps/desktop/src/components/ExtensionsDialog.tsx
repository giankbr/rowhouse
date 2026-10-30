import { BookOpen, FolderOpen, Package, Puzzle, ShieldAlert, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, dialogs, type ExtensionInfo } from "../lib/api";
import { permissionText } from "../lib/extHost";
import { Button, Modal, Switch, cx } from "./ui";

/** Installed extensions: install from a folder or .rhext, review permissions, enable, open. */
export function ExtensionsDialog({
  open,
  onClose,
  onChanged,
  onOpenPanel,
  onToast,
}: {
  open: boolean;
  onClose: () => void;
  onChanged: (list: ExtensionInfo[]) => void;
  onOpenPanel: (ext: ExtensionInfo, panel: string) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [list, setList] = useState<ExtensionInfo[] | null>(null);
  const [review, setReview] = useState<ExtensionInfo | null>(null);
  const [remove, setRemove] = useState<string | null>(null);

  const load = () =>
    api.ext
      .list()
      .then((l) => (setList(l), onChanged(l)))
      .catch((e) => onToast(String(e)));
  useEffect(() => {
    if (open) (setReview(null), setRemove(null), load());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const install = async (kind: "folder" | "file") => {
    const path = kind === "folder" ? await dialogs.folder() : await dialogs.openAny("Rowhouse extension", ["rhext", "zip"]);
    if (!path) return;
    try {
      const x = await api.ext.install(path);
      await load();
      setReview(x);
    } catch (e) {
      onToast(String(e));
    }
  };
  const enable = async (x: ExtensionInfo, on: boolean) => {
    try {
      await api.ext.setEnabled(x.manifest.id, on);
      setReview(null);
      await load();
      if (on) onToast(`${x.manifest.name} enabled`, "ok");
    } catch (e) {
      onToast(String(e));
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={680}
      title={
        <span className="flex items-center gap-2">
          <Puzzle size={14} className="text-accent" /> Extensions
        </span>
      }
      footer={
        <>
          <a className="flex items-center gap-1 text-2xs text-muted hover:text-fg" href="https://github.com/giankbr/rowhouse/blob/master/docs/EXTENSIONS.md" target="_blank" rel="noreferrer">
            <BookOpen size={12} /> Build your own
          </a>
          <span className="flex-1" />
          <Button variant="ghost" onClick={() => install("file")}>
            <Package size={12} /> Install .rhext…
          </Button>
          <Button variant="ghost" onClick={() => install("folder")}>
            <FolderOpen size={12} /> Install from folder…
          </Button>
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      {review ? (
        <div className="flex flex-col gap-3">
          <div className="flex items-start gap-3">
            <ShieldAlert size={18} className="mt-0.5 shrink-0 text-warning" />
            <div>
              <div className="text-sm font-semibold">
                Enable {review.manifest.name} <span className="font-normal text-muted">{review.manifest.version}</span>?
              </div>
              <p className="mt-0.5 text-xs text-muted">{review.manifest.author ? `By ${review.manifest.author}. ` : ""}It runs in a sandbox and can only do what you allow here:</p>
            </div>
          </div>
          <ul className="flex flex-col gap-1 rounded-lg border border-line bg-panel p-3 text-xs">
            {review.manifest.permissions.length === 0 && <li className="text-muted">No special access — it can only show its own panel.</li>}
            {review.manifest.permissions.map((p) => (
              <li key={p} className={cx("flex gap-2", (p === "query:write" || p.startsWith("network:")) && "text-warning")}>
                <span>•</span> {permissionText(p)}
              </li>
            ))}
          </ul>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setReview(null)}>
              Not now
            </Button>
            <Button variant="primary" onClick={() => enable(review, true)}>
              Enable
            </Button>
          </div>
        </div>
      ) : list === null ? (
        <div className="p-6 text-center text-xs text-subtle">Loading…</div>
      ) : list.length === 0 ? (
        <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-line p-8 text-center text-xs text-muted">
          <Puzzle size={22} className="text-subtle" />
          No extensions yet. Extensions are small web apps that run in a sandboxed panel and use the <code className="font-mono">window.rowhouse</code> API — table stats, exporters, dashboards, internal tools.
        </div>
      ) : (
        <div className="-mx-4 -my-3 divide-y divide-line">
          {list.map((x) => (
            <div key={x.manifest.id} className="flex items-start gap-3 px-4 py-3">
              <div className={cx("mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg", x.enabled ? "bg-accent-soft text-accent" : "bg-hover text-subtle")}>
                <Puzzle size={15} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2 text-xs">
                  <span className="font-semibold">{x.manifest.name}</span>
                  <span className="text-2xs text-subtle">
                    {x.manifest.version}
                    {x.manifest.author && ` · ${x.manifest.author}`}
                  </span>
                </div>
                {x.manifest.description && <p className="mt-0.5 text-xs text-muted">{x.manifest.description}</p>}
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {x.manifest.permissions.map((p) => (
                    <span key={p} title={permissionText(p)} className="rounded bg-hover px-1.5 py-px font-mono text-[10px] text-muted">
                      {p}
                    </span>
                  ))}
                  {x.enabled &&
                    x.manifest.contributes.panels.map((p) => (
                      <button key={p.id} onClick={() => (onOpenPanel(x, p.id), onClose())} className="rounded bg-accent-soft px-1.5 py-px text-[10.5px] font-medium text-accent hover:brightness-110">
                        Open {p.title}
                      </button>
                    ))}
                </div>
                {remove === x.manifest.id && (
                  <div className="mt-2 flex items-center gap-2 text-2xs">
                    <span className="text-danger">Remove {x.manifest.name} and its files?</span>
                    <Button
                      size="xs"
                      variant="danger"
                      onClick={() =>
                        api.ext
                          .uninstall(x.manifest.id)
                          .then(load)
                          .catch((e) => onToast(String(e)))
                      }
                    >
                      Remove
                    </Button>
                    <Button size="xs" variant="ghost" onClick={() => setRemove(null)}>
                      Keep
                    </Button>
                  </div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                <Switch checked={x.enabled} onChange={(on) => (on ? setReview(x) : enable(x, false))} label={<span className="sr-only">Enabled</span>} />
                <Button size="icon" variant="ghost" title="Uninstall" onClick={() => setRemove(x.manifest.id)}>
                  <Trash2 size={12} />
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}
