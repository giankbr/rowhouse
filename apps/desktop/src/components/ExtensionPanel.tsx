import { AlertCircle, Puzzle } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, kindInfo, type ConnectionConfig, type ExtensionInfo, type TableInfo } from "../lib/api";
import { buildSrcdoc, can, themeSnapshot } from "../lib/extHost";

export interface ExtEvent {
  nonce: string;
  event: string;
  data: unknown;
}

/** One extension panel (a tab): sandboxed iframe + message bridge. */
export function ExtensionPanel({
  ext,
  conn,
  event,
  onToast,
  onOpenQuery,
  onOpenTable,
}: {
  ext: ExtensionInfo | undefined;
  conn: ConnectionConfig | undefined;
  /** something to deliver once the page is ready (a result, a command) */
  event?: ExtEvent;
  onToast: (m: string, k?: "ok" | "err") => void;
  onOpenQuery: (sql: string) => void;
  onOpenTable: (t: TableInfo) => void;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [doc, setDoc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ready = useRef(false);
  const delivered = useRef<string | null>(null);
  const latest = useRef({ ext, conn, event, onToast, onOpenQuery, onOpenTable });
  latest.current = { ext, conn, event, onToast, onOpenQuery, onOpenTable };

  useEffect(() => {
    if (!ext) return;
    ready.current = false;
    delivered.current = null;
    setDoc(null);
    setError(null);
    api.ext
      .entry(ext.manifest.id)
      .then((html) => setDoc(buildSrcdoc(html, ext)))
      .catch((e) => setError(String(e)));
  }, [ext]);

  const post = (msg: Record<string, unknown>) => frame.current?.contentWindow?.postMessage({ rh: 1, ...msg }, "*");
  const deliver = () => {
    const ev = latest.current.event;
    if (!ready.current || !ev || delivered.current === ev.nonce) return;
    delivered.current = ev.nonce;
    post({ event: ev.event, data: ev.data });
  };
  useEffect(deliver, [event]);

  // connection changes and theme changes reach the page as events
  useEffect(() => {
    if (ready.current) post({ event: "connection", data: conn ? { id: conn.id, name: conn.name, kind: conn.kind, dialect: kindInfo(conn.kind).dialect } : null });
  }, [conn?.id]);
  useEffect(() => {
    const obs = new MutationObserver(() => ready.current && post({ event: "theme", data: themeSnapshot() }));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "style", "data-density"] });
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    const onMessage = async (e: MessageEvent) => {
      if (e.source !== frame.current?.contentWindow) return;
      const m = e.data as { rh?: number; id?: number; method?: string; params?: Record<string, unknown> };
      if (!m || m.rh !== 1 || typeof m.id !== "number" || !m.method) return;
      const { ext, conn, onToast, onOpenQuery, onOpenTable } = latest.current;
      const reply = (result: unknown) => post({ reply: m.id, result });
      const fail = (error: unknown) => post({ reply: m.id, error: String(error) });
      if (!ext) return fail("extension unavailable");
      const need = (perm: string) => {
        if (!can(ext, perm)) throw `${ext.manifest.name} doesn't have the “${perm}” permission`;
      };
      const needConn = () => {
        if (!conn) throw "no connection is open";
        return conn;
      };
      const p = m.params ?? {};
      try {
        switch (m.method) {
          case "ready":
            ready.current = true;
            reply(themeSnapshot());
            setTimeout(deliver, 0);
            return;
          case "context":
            return reply({
              extension: { id: ext.manifest.id, version: ext.manifest.version, permissions: ext.granted },
              connection: conn ? { id: conn.id, name: conn.name, kind: conn.kind, dialect: kindInfo(conn.kind).dialect, database: conn.database ?? null, readOnly: conn.readOnly } : null,
              theme: themeSnapshot().mode,
            });
          case "schema.tables":
            need("schema");
            return reply(await api.listTables(needConn().id));
          case "schema.columns":
            need("schema");
            return reply(await api.describeTable(needConn().id, String(p.schema), String(p.table)));
          case "query": {
            if (!can(ext, "query:read") && !can(ext, "query:write")) need("query:read");
            const out = await api.ext.query(ext.manifest.id, needConn().id, String(p.sql ?? ""), typeof p.limit === "number" ? p.limit : undefined);
            return reply(out);
          }
          case "openQuery":
            onOpenQuery(String(p.sql ?? ""));
            return reply(true);
          case "openTable":
            onOpenTable({ schema: String(p.schema), name: String(p.table), kind: "table" });
            return reply(true);
          case "toast":
            onToast(`${ext.manifest.name}: ${String(p.message ?? "")}`.slice(0, 240), p.kind === "err" ? "err" : "ok");
            return reply(true);
          case "clipboard.write":
            need("clipboard");
            await navigator.clipboard.writeText(String(p.text ?? ""));
            return reply(true);
          case "storage.get":
            return reply(readStore(ext.manifest.id)[String(p.key)] ?? null);
          case "storage.set": {
            const s = readStore(ext.manifest.id);
            s[String(p.key)] = p.value;
            const json = JSON.stringify(s);
            if (json.length > 1_000_000) throw "storage is full (1 MB per extension)";
            try {
              localStorage.setItem(`rh-ext:${ext.manifest.id}`, json);
            } catch {
              /* storage unavailable */
            }
            return reply(true);
          }
          default:
            return fail(`unknown method ${m.method}`);
        }
      } catch (err) {
        fail(err);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!ext)
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 text-xs text-subtle">
        <Puzzle size={20} /> This extension isn't installed or enabled anymore.
      </div>
    );
  if (error)
    return (
      <div className="m-3 flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-3 text-xs text-danger">
        <AlertCircle size={14} className="shrink-0" /> {error}
      </div>
    );
  return doc ? (
    <iframe ref={frame} title={ext.manifest.name} srcDoc={doc} sandbox="allow-scripts allow-forms allow-modals" className="h-full w-full flex-1 border-0 bg-bg" />
  ) : (
    <div className="flex h-full items-center justify-center text-xs text-subtle">Loading {ext.manifest.name}…</div>
  );
}

function readStore(id: string): Record<string, unknown> {
  try {
    return JSON.parse(localStorage.getItem(`rh-ext:${id}`) ?? "{}");
  } catch {
    return {};
  }
}
