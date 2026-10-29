import { Database, Plus } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { McpDialog } from "./components/McpDialog";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TabBar } from "./components/TabBar";
import { Titlebar } from "./components/Titlebar";
import { Button, Kbd, cx } from "./components/ui";
import { Workspace } from "./components/Workspace";
import { api, type ConnectionConfig, type TableInfo } from "./lib/api";
import { mod, uid, type Tab } from "./lib/types";

export default function App() {
  const [connections, setConnections] = useState<ConnectionConfig[]>([]);
  const [activeConn, setActiveConn] = useState<string | null>(null);
  const [connected, setConnected] = useState<Record<string, boolean>>({});
  const [tables, setTables] = useState<Record<string, TableInfo[]>>({});
  const [loadingTables, setLoadingTables] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [sidebar, setSidebar] = useState(true);
  const [palette, setPalette] = useState(false);
  const [mcpOpen, setMcpOpen] = useState(false);
  const [dialog, setDialog] = useState<{ open: boolean; edit: ConnectionConfig | null }>({ open: false, edit: null });
  const [toast, setToastState] = useState<{ msg: string; kind: "ok" | "err" } | null>(null);
  const setToast = useCallback(
    (msg: string | null, kind: "ok" | "err" = "err") => setToastState(msg ? { msg, kind } : null),
    [],
  );
  const [dark, setDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  useEffect(() => {
    api.listConnections().then(setConnections).catch((e) => setToast(String(e)));
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  const conn = connections.find((c) => c.id === activeConn) ?? null;
  const connTables = (activeConn && tables[activeConn]) || [];
  const visibleTabs = useMemo(() => tabs.filter((t) => t.connId === activeConn), [tabs, activeConn]);
  const tab = visibleTabs.find((t) => t.id === activeTab) ?? null;

  const patchTab = useCallback((id: string, patch: Partial<Tab>) => {
    setTabs((ts) => ts.map((t) => (t.id === id ? { ...t, ...patch } : t)));
  }, []);

  const loadTables = useCallback(async (id: string) => {
    setLoadingTables(true);
    try {
      await api.connect(id);
      setConnected((c) => ({ ...c, [id]: true }));
      setTables((t) => ({ ...t, [id]: [] }));
      const list = await api.listTables(id);
      setTables((t) => ({ ...t, [id]: list }));
    } catch (e) {
      setConnected((c) => ({ ...c, [id]: false }));
      setToast(String(e));
    } finally {
      setLoadingTables(false);
    }
  }, []);

  const runTab = useCallback(
    async (t: Tab, sqlOverride?: string) => {
      patchTab(t.id, { running: true, error: undefined });
      try {
        const output = await api.runQuery(t.connId, sqlOverride ?? t.sql);
        setConnected((c) => ({ ...c, [t.connId]: true }));
        patchTab(t.id, { output, running: false, resultIndex: 0, pending: undefined });
        // DDL may have changed the schema.
        if (/^\s*(create|drop|alter)\b/i.test(sqlOverride ?? t.sql)) loadTables(t.connId);
      } catch (e) {
        patchTab(t.id, { error: String(e), running: false, output: undefined });
      }
    },
    [patchTab, loadTables],
  );

  const selectConnection = useCallback(
    (id: string) => {
      setActiveConn(id);
      if (!tables[id]?.length) loadTables(id);
      setActiveTab((cur) => {
        const mine = tabs.filter((t) => t.connId === id);
        return mine.find((t) => t.id === cur)?.id ?? mine[mine.length - 1]?.id ?? null;
      });
    },
    [tables, tabs, loadTables],
  );

  const newQuery = useCallback(() => {
    if (!activeConn) return;
    const n = tabs.filter((t) => t.kind === "query" && t.connId === activeConn).length + 1;
    const t: Tab = { id: uid(), kind: "query", title: `Query ${n}`, connId: activeConn, sql: "SELECT 1;" };
    setTabs((ts) => [...ts, t]);
    setActiveTab(t.id);
  }, [activeConn, tabs]);

  const openTable = useCallback(
    async (info: TableInfo) => {
      if (!activeConn) return;
      const existing = tabs.find(
        (t) => t.kind === "table" && t.connId === activeConn && t.schema === info.schema && t.table === info.name,
      );
      if (existing) return setActiveTab(existing.id);
      const sql = await api.tableSql(activeConn, info.schema, info.name);
      const t: Tab = {
        id: uid(),
        kind: "table",
        title: info.name,
        connId: activeConn,
        sql,
        schema: info.schema,
        table: info.name,
        view: "data",
      };
      setTabs((ts) => [...ts, t]);
      setActiveTab(t.id);
      runTab(t);
      // Column metadata (primary key) decides whether the grid is editable.
      api
        .describeTable(activeConn, info.schema, info.name)
        .then((columns) => patchTab(t.id, { columns }))
        .catch((e) => setToast(String(e)));
    },
    [activeConn, tabs, runTab, patchTab],
  );

  const closeTab = useCallback(
    (id: string) => {
      setTabs((ts) => {
        const i = ts.findIndex((t) => t.id === id);
        const next = ts.filter((t) => t.id !== id);
        if (id === activeTab) {
          const mine = next.filter((t) => t.connId === activeConn);
          setActiveTab(mine[Math.min(i, mine.length - 1)]?.id ?? null);
        }
        return next;
      });
    },
    [activeTab, activeConn],
  );

  const setView = useCallback(
    async (v: "data" | "structure") => {
      if (!tab) return;
      patchTab(tab.id, { view: v });
      if (!tab.columns && tab.schema && tab.table) {
        try {
          patchTab(tab.id, { columns: await api.describeTable(tab.connId, tab.schema, tab.table) });
        } catch (e) {
          setToast(String(e));
        }
      }
    },
    [tab, patchTab],
  );

  // Global shortcuts
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const m = e.metaKey || e.ctrlKey;
      if (!m) return;
      const k = e.key.toLowerCase();
      if (k === "k" || k === "p") (e.preventDefault(), setPalette((p) => !p));
      else if (k === "t") (e.preventDefault(), newQuery());
      else if (k === "w" && activeTab) (e.preventDefault(), closeTab(activeTab));
      else if (k === "b") (e.preventDefault(), setSidebar((s) => !s));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newQuery, closeTab, activeTab]);

  return (
    <div className="flex h-full flex-col">
      <Titlebar
        conn={conn}
        dark={dark}
        onToggleTheme={() => setDark((d) => !d)}
        onToggleSidebar={() => setSidebar((s) => !s)}
        onPalette={() => setPalette(true)}
        onMcp={() => setMcpOpen(true)}
      />

      <div className="flex min-h-0 flex-1">
        <div className={cx("shrink-0 overflow-hidden border-r border-line transition-[width] duration-150", sidebar ? "w-60" : "w-0 border-r-0")}>
          <div className="h-full w-60">
            <Sidebar
              connections={connections}
              activeId={activeConn}
              tables={connTables}
              loadingTables={loadingTables}
              onSelect={selectConnection}
              onEdit={(c) => setDialog({ open: true, edit: c })}
              onNew={() => setDialog({ open: true, edit: null })}
              onOpenTable={openTable}
              onRefresh={() => activeConn && loadTables(activeConn)}
            />
          </div>
        </div>

        <main className="flex min-w-0 flex-1 flex-col">
          {conn ? (
            <>
              <TabBar tabs={visibleTabs} activeId={tab?.id ?? null} onSelect={setActiveTab} onClose={closeTab} onNew={newQuery} />
              {tab ? (
                <Workspace
                  key={tab.id}
                  tab={tab}
                  conn={conn}
                  tables={connTables}
                  onChange={(p) => patchTab(tab.id, p)}
                  onRun={(s) => runTab(tab, s)}
                  onView={setView}
                  onToast={setToast}
                />
              ) : (
                <Empty>
                  <p className="text-xs text-muted">Pick a table on the left, or start a query.</p>
                  <div className="flex gap-2">
                    <Button variant="primary" onClick={newQuery}>
                      New query
                    </Button>
                  </div>
                  <Shortcuts />
                </Empty>
              )}
            </>
          ) : (
            <Empty>
              <div className="flex size-10 items-center justify-center rounded-xl bg-accent-soft text-accent">
                <Database size={18} />
              </div>
              <div className="text-center">
                <h1 className="text-sm font-semibold">Welcome to Rowhouse</h1>
                <p className="mt-1 text-xs text-muted">Select a connection or create a new one.</p>
              </div>
              <Button variant="primary" onClick={() => setDialog({ open: true, edit: null })}>
                <Plus size={13} /> New connection
              </Button>
              <Shortcuts />
            </Empty>
          )}
        </main>
      </div>

      <StatusBar tab={tab} connected={!!(activeConn && connected[activeConn])} />

      <ConnectionDialog
        open={dialog.open}
        initial={dialog.edit}
        onClose={() => setDialog({ open: false, edit: null })}
        onSaved={(c) => {
          setConnections((cs) => (cs.some((x) => x.id === c.id) ? cs.map((x) => (x.id === c.id ? c : x)) : [...cs, c]));
          setDialog({ open: false, edit: null });
          setTables((t) => ({ ...t, [c.id]: [] }));
          setActiveConn(c.id);
          loadTables(c.id);
        }}
        onDeleted={(id) => {
          setConnections((cs) => cs.filter((x) => x.id !== id));
          setTabs((ts) => ts.filter((t) => t.connId !== id));
          if (activeConn === id) setActiveConn(null);
          setDialog({ open: false, edit: null });
        }}
      />
      <McpDialog open={mcpOpen} onClose={() => setMcpOpen(false)} />
      <CommandPalette
        open={palette}
        onClose={() => setPalette(false)}
        connections={connections}
        tables={connTables}
        actions={{
          newQuery,
          newConnection: () => setDialog({ open: true, edit: null }),
          openTable,
          selectConnection,
          toggleTheme: () => setDark((d) => !d),
          showMcp: () => setMcpOpen(true),
        }}
      />

      {toast && (
        <div
          className={cx(
            "animate-pop fixed right-4 bottom-9 z-50 max-w-sm rounded-lg bg-elevated px-3 py-2 text-xs shadow-pop",
            toast.kind === "ok" ? "text-fg" : "text-danger",
          )}
        >
          {toast.msg}
        </div>
      )}
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-1 flex-col items-center justify-center gap-4">{children}</div>;
}

function Shortcuts() {
  const rows: [string, string[]][] = [
    ["Command palette", [mod, "K"]],
    ["New query", [mod, "T"]],
    ["Run query", [mod, "↵"]],
    ["Toggle sidebar", [mod, "B"]],
  ];
  return (
    <div className="mt-2 grid grid-cols-[auto_auto] gap-x-6 gap-y-1.5 text-2xs text-muted">
      {rows.map(([label, keys]) => (
        <div key={label} className="contents">
          <span>{label}</span>
          <span className="flex gap-0.5 justify-self-end">
            {keys.map((k) => (
              <Kbd key={k}>{k}</Kbd>
            ))}
          </span>
        </div>
      ))}
    </div>
  );
}
