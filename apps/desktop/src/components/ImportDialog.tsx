import { FileUp, KeyRound, Lock, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, dialogs, kindInfo, type ConnectionConfig, type DetectedImport, type ImportCandidate, type ImportSource } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Modal, cx } from "./ui";

const SOURCE_LABEL: Record<ImportSource, string> = { dbeaver: "DBeaver", tableplus: "TablePlus", navicat: "Navicat" };

export function ImportDialog({
  open,
  onClose,
  onImported,
  initialPath,
}: {
  open: boolean;
  /** go straight to the preview of this file (e.g. picked on the start screen) */
  initialPath?: string | null;
  onClose: () => void;
  onImported: (saved: ConnectionConfig[]) => void;
}) {
  const [detected, setDetected] = useState<DetectedImport[]>([]);
  const [preview, setPreview] = useState<{ source: ImportSource; path: string; candidates: ImportCandidate[] } | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setPreview(null);
    setError(null);
    api.importDetect().then(setDetected).catch(() => setDetected([]));
    if (initialPath) parse(initialPath);
  }, [open, initialPath]); // eslint-disable-line react-hooks/exhaustive-deps

  const parse = async (path: string | null) => {
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.importParse(path);
      setPreview({ ...r, path });
      setSelected(new Set(r.candidates.flatMap((c, i) => (c.skippedReason ? [] : [i]))));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const doImport = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const saved = await api.importSave(preview.candidates.filter((_, i) => selected.has(i)).map((c) => c.config));
      onImported(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (i: number) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={560}
      title={preview ? `Import from ${SOURCE_LABEL[preview.source]}` : "Import connections"}
      footer={
        preview ? (
          <>
            <Button variant="ghost" onClick={() => setPreview(null)}>
              Back
            </Button>
            <span className="flex-1 text-2xs text-subtle">Passwords aren't imported — you'll be asked once per connection.</span>
            <Button variant="primary" onClick={doImport} disabled={busy || selected.size === 0}>
              Import {selected.size}
            </Button>
          </>
        ) : (
          <>
            <span className="flex-1" />
            <Button onClick={onClose}>Cancel</Button>
          </>
        )
      }
    >
      {error && <div className="selectable mb-3 rounded-md border border-danger/30 bg-danger/5 px-2.5 py-1.5 text-2xs text-danger">{error}</div>}

      {!preview ? (
        <div className="flex flex-col gap-3 text-xs">
          {detected.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <span className="text-2xs font-medium text-muted">Found on this computer</span>
              {detected.map((d) => (
                <button
                  key={d.path}
                  onClick={() => parse(d.path)}
                  className="flex items-center gap-3 rounded-lg border border-line px-3 py-2 text-left hover:border-accent hover:bg-accent-soft/40"
                >
                  <span className="font-medium">{SOURCE_LABEL[d.source]}</span>
                  <span className="shrink-0 rounded-full bg-accent/12 px-1.5 text-2xs font-medium text-accent">{d.count}</span>
                  <span className="truncate font-mono text-2xs text-subtle" title={d.path}>{d.path.replace(/^\/Users\/[^/]+/, "~")}</span>
                </button>
              ))}
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <span className="text-2xs font-medium text-muted">Or pick a file</span>
            <div className="grid grid-cols-3 gap-2">
              {(
                [
                  ["DBeaver", "data-sources.json", ["json"]],
                  ["TablePlus", "Connections.plist / export", ["plist", "json", "tableplusconnection"]],
                  ["Navicat", "connections.ncx", ["ncx", "xml"]],
                ] as const
              ).map(([label, hint, ext]) => (
                <button
                  key={label}
                  onClick={async () => parse(await dialogs.openAny(label, [...ext]))}
                  className="flex flex-col items-start gap-1 rounded-lg border border-line px-3 py-2.5 text-left hover:border-accent hover:bg-accent-soft/40"
                >
                  <span className="flex items-center gap-1.5 font-medium">
                    <FileUp size={12} className="text-accent" /> {label}
                  </span>
                  <span className="font-mono text-[10.5px] text-subtle">{hint}</span>
                </button>
              ))}
            </div>
          </div>
          {busy && (
            <span className="flex items-center gap-1.5 text-2xs text-muted">
              <Loader2 size={12} className="animate-spin" /> Reading…
            </span>
          )}
        </div>
      ) : (
        <div className="flex flex-col">
          <div className="mb-2 truncate font-mono text-2xs text-subtle">{preview.path}</div>
          <div className="flex flex-col overflow-hidden rounded-lg border border-line">
            {preview.candidates.map((c, i) => {
              const cfg = c.config;
              const disabled = !!c.skippedReason;
              return (
                <label
                  key={i}
                  className={cx(
                    "flex items-center gap-2.5 border-b border-line/70 px-3 py-2 text-xs last:border-0",
                    disabled ? "opacity-45" : "cursor-pointer hover:bg-hover",
                  )}
                >
                  <input type="checkbox" disabled={disabled} checked={selected.has(i)} onChange={() => toggle(i)} className="accent-[var(--accent)]" />
                  <KindBadge kind={cfg.kind} size={18} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{cfg.name}</span>
                    <span className="block truncate text-2xs text-subtle">
                      {c.skippedReason ??
                        (kindInfo(cfg.kind).file
                          ? cfg.file
                          : `${cfg.user ? cfg.user + "@" : ""}${cfg.host ?? ""}${cfg.port ? ":" + cfg.port : ""}${cfg.database ? "/" + cfg.database : ""}`)}
                    </span>
                  </span>
                  {cfg.ssh && (
                    <span className="flex items-center gap-1 text-2xs text-muted" title={`SSH via ${cfg.ssh.host}`}>
                      <KeyRound size={11} /> SSH
                    </span>
                  )}
                  {cfg.env === "production" && (
                    <span className="flex items-center gap-1 rounded bg-danger/10 px-1.5 py-px text-[10px] font-semibold text-danger">
                      <Lock size={9} /> PROD
                    </span>
                  )}
                </label>
              );
            })}
          </div>
        </div>
      )}
    </Modal>
  );
}
