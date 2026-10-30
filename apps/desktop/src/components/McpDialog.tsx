import { Check, ChevronRight, Copy, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type McpClient, type McpInfo } from "../lib/api";
import { Button, Modal, cx } from "./ui";

const TOOLS: [string, string][] = [
  ["list_connections", "connections exposed to MCP"],
  ["search_schema", "find tables / columns by name"],
  ["list_tables · describe_table", "schema"],
  ["get_relations · get_ddl", "foreign keys and CREATE statements"],
  ["list_objects · get_object_definition", "functions, triggers, sequences, types"],
  ["sample_rows · run_query", "read-only queries"],
  ["explain_query", "query plans"],
  ["table_stats", "approximate row counts and sizes"],
  ["list_saved_queries · run_saved_query", "your saved queries"],
  ["recent_queries", "query history"],
  ["execute_sql", "writes — each one waits for your approval here"],
];

/** Hook Rowhouse's MCP server into the AI tools on this machine. */
export function McpDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [info, setInfo] = useState<McpInfo | null>(null);
  const [clients, setClients] = useState<McpClient[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [manual, setManual] = useState(false);
  const load = () => api.mcpClients().then(setClients).catch((e) => setError(String(e)));
  useEffect(() => {
    if (!open) return;
    setError(null);
    api.mcpInfo().then(setInfo);
    load();
  }, [open]);
  const snippet = info ? JSON.stringify(info.snippet, null, 2) : "";

  const act = async (c: McpClient, remove: boolean) => {
    setBusy(c.id);
    setError(null);
    try {
      if (remove) await api.mcpRemove(c.id);
      else await api.mcpInstall(c.id);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const sorted = [...(clients ?? [])].sort((a, b) => Number(b.detected) - Number(a.detected));

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={620}
      title="Connect an AI assistant"
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">Only connections with “Expose to AI assistants via MCP” are visible. Restart the client after adding.</span>
          <Button onClick={onClose}>Done</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        <p className="text-muted">
          Rowhouse ships an MCP server. Add it to your AI tools and they can explore schemas and run read-only queries on your saved connections — writes always wait for
          your approval.
        </p>

        <div className="overflow-hidden rounded-lg border border-line">
          {clients === null && <div className="p-4 text-center text-subtle">Looking for AI clients…</div>}
          {sorted.map((c) => (
            <div key={c.id} className={cx("flex items-center gap-3 border-b border-line/70 px-3 py-2 last:border-0", !c.detected && "opacity-60")}>
              <span className={cx("size-1.5 shrink-0 rounded-full", c.status === "installed" ? "bg-success" : c.status === "outdated" ? "bg-warning" : "bg-line-strong")} />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 font-medium">
                  {c.label}
                  {!c.detected && <span className="text-2xs font-normal text-subtle">not found</span>}
                  {c.status === "outdated" && <span className="text-2xs font-normal text-warning">points at another copy</span>}
                  {c.status === "unreadable" && <span className="text-2xs font-normal text-danger">config has comments — add by hand</span>}
                </span>
                <span className="block truncate font-mono text-[10.5px] text-subtle" title={c.configPath}>
                  {c.configPath}
                </span>
              </span>
              {busy === c.id ? (
                <Loader2 size={13} className="animate-spin text-muted" />
              ) : c.status === "installed" ? (
                <>
                  <span className="flex items-center gap-1 text-2xs text-success">
                    <Check size={12} /> Added
                  </span>
                  <Button size="xs" variant="ghost" onClick={() => act(c, true)}>
                    Remove
                  </Button>
                </>
              ) : (
                <Button size="xs" variant={c.detected ? "primary" : "default"} disabled={c.status === "unreadable"} onClick={() => act(c, false)}>
                  {c.status === "outdated" ? "Update" : "Add"}
                </Button>
              )}
            </div>
          ))}
        </div>
        {error && <div className="rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">{error}</div>}

        <div>
          <div className="mb-1 text-2xs font-medium tracking-wide text-subtle uppercase">Tools</div>
          <div className="grid grid-cols-2 gap-x-4 gap-y-0.5 text-2xs">
            {TOOLS.map(([t, d]) => (
              <div key={t} className="flex min-w-0 gap-1.5">
                <span className="shrink-0 font-mono text-fg/85">{t}</span>
                <span className="truncate text-subtle">{d}</span>
              </div>
            ))}
          </div>
        </div>

        <button onClick={() => setManual((m) => !m)} className="flex items-center gap-1 text-2xs text-muted hover:text-fg">
          <ChevronRight size={12} className={cx("transition-transform", manual && "rotate-90")} /> Other clients — add it by hand
        </button>
        {manual && (
          <div className="relative">
            <pre className="selectable overflow-x-auto rounded-lg border border-line bg-panel p-3 font-mono text-2xs leading-relaxed">{snippet}</pre>
            <Button
              size="xs"
              className="absolute top-2 right-2"
              onClick={() => {
                navigator.clipboard.writeText(snippet);
                setCopied(true);
                setTimeout(() => setCopied(false), 1200);
              }}
            >
              {copied ? <Check size={11} /> : <Copy size={11} />} {copied ? "Copied" : "Copy"}
            </Button>
            <p className="mt-1 truncate text-2xs text-subtle">
              Profiles file: <span className="selectable font-mono">{info?.configPath}</span>
            </p>
          </div>
        )}
      </div>
    </Modal>
  );
}
