import { useCallback, useEffect, useMemo, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { ConfirmDestructive } from "./components/ConfirmDestructive";
import { ApprovalCenter } from "./components/ApprovalDialog";
import { UpdateBanner } from "./components/UpdateBanner";
import { CompareView } from "./components/CompareView";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { ErdView } from "./components/ErdView";
import { ImportDialog } from "./components/ImportDialog";
import { Launcher, touchRecent } from "./components/Launcher";
import { HistoryPanel } from "./components/HistoryPanel";
import { McpDialog } from "./components/McpDialog";
import { SchedulesDialog } from "./components/SchedulesDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TabBar } from "./components/TabBar";
import { Titlebar } from "./components/Titlebar";
import { Button, Kbd, TAG_COLORS, cx } from "./components/ui";
import { Workspace } from "./components/Workspace";
import { api, dialogs, isTauri, newConnection, ENVIRONMENTS, type ConnectionConfig, type Destructive, type Snippet, type TableInfo } from "./lib/api";
import { kindInfo } from "./lib/api";
import { mod, uid, type Tab } from "./lib/types";

export default function App() {
  const [connections, setConnections] = useState<ConnectionConfig[]>([]);
  const [activeConn, setActiveConn] = useState<string | null>(null);
  const [connected, setConnected] = useState<Record<string, boolean>>({});
  const [tables, setTables] = useState<Record<string, TableInfo[]>>({});
  const [loadingTables, setLoadingTables] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  // per split pane: last selected tab (falls back to the pane's last tab)
  const [activeByPane, setActiveByPane] = useState<[string | null, string | null]>([null, null]);
  const [focusedPane, setFocusedPane] = useState<0 | 1>(0);
  const [draggingTab, setDraggingTab] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ tab: Tab; sql: string; items: Destructive[] } | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [sidebar, setSidebar] = useState(true);
  const [palette, setPalette] = useState(false);
  const [mcpOpen, setMcpOpen] = useState(false);
  const [schedules, setSchedules] = useState<{ draft: { connectionId: string; sql: string; name?: string } | null } | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [aiEnabled, setAiEnabled] = useState(false);
  const [inlineAi, setInlineAi] = useState(false);
  const [dialog, setDialog] = useState<{ open: boolean; edit: ConnectionConfig | null }>({ open: false, edit: null });
  const [toast, setToastState] = useState<{ msg: string; kind: "ok" | "err" } | null>(null);
  const setToast = useCallback(
    (msg: string | null, kind: "ok" | "err" = "err") => setToastState(msg ? { msg, kind } : null),
    [],
  );
  const [version, setVersion] = useState("0.1.0");
  useEffect(() => {
    if (isTauri) import("@tauri-apps/api/app").then((m) => m.getVersion()).then(setVersion).catch(() => {});
  }, []);
  const [dark, setDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  useEffect(() => {
    api.listConnections().then(setConnections).catch((e) => setToast(String(e)));
    api.ai
      .getSettings()
      .then((r) => (setAiEnabled(r.settings.enabled && !!r.settings.model), setInlineAi(r.settings.enabled && !!r.settings.inlineCompletions)))
      .catch(() => {});
  }, [setToast]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  const conn = connections.find((c) => c.id === activeConn) ?? null;
  const connTables = (activeConn && tables[activeConn]) || [];
  const visibleTabs = useMemo(() => tabs.filter((t) => t.connId === activeConn), [tabs, activeConn]);
  const paneTabs = useCallback((p: 0 | 1) => visibleTabs.filter((t) => (t.pane ?? 0) === p), [visibleTabs]);
  const split = paneTabs(1).length > 0;
  const focused: 0 | 1 = split ? focusedPane : 0;
  const activeIn = useCallback(
    (p: 0 | 1) => {
      const list = paneTabs(p);
      return list.find((t) => t.id === activeByPane[p]) ?? list[list.length - 1] ?? null;
    },
    [paneTabs, activeByPane],
  );
  const tab = activeIn(focused);
  const activeTab = tab?.id ?? null;

  const selectTab = useCallback(
    (id: string) => {
      const t = tabs.find((x) => x.id === id);
      if (!t) return;
      const p = (t.pane ?? 0) as 0 | 1;
      setActiveByPane((a) => (p === 0 ? [id, a[1]] : [a[0], id]));
      setFocusedPane(p);
    },
    [tabs],
  );

  /** Add a tab to the focused pane and select it. */
  const addTab = useCallback(
    (t: Tab) => {
      const p = focused;
      setTabs((ts) => [...ts, { ...t, pane: p }]);
      setActiveByPane((a) => (p === 0 ? [t.id, a[1]] : [a[0], t.id]));
      setFocusedPane(p);
    },
    [focused],
  );

  /** Drag & drop: move a tab into `pane`, before `beforeId` or at the end. */
  const moveTab = useCallback((id: string, pane: 0 | 1, beforeId: string | null) => {
    setTabs((ts) => {
      const t = ts.find((x) => x.id === id);
      if (!t) return ts;
      const rest = ts.filter((x) => x.id !== id);
      const moved = { ...t, pane };
      let at = beforeId ? rest.findIndex((x) => x.id === beforeId) : -1;
      if (at < 0) {
        const lastInPane = rest.reduce((acc, x, i) => (x.connId === t.connId && (x.pane ?? 0) === pane ? i : acc), -1);
        at = lastInPane + 1 || rest.length;
      }
      return [...rest.slice(0, at), moved, ...rest.slice(at)];
    });
    setActiveByPane((a) => (pane === 0 ? [id, a[1]] : [a[0], id]));
    setFocusedPane(pane);
  }, []);

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

  const execTab = useCallback(
    async (t: Tab, sqlOverride?: string) => {
      patchTab(t.id, { running: true, error: undefined, showPlan: false });
      if (t.kind === "table" && t.schema && t.table && !sqlOverride) {
        // table tabs browse server-side: filters, sort and paging become SQL
        try {
          const page = await api.tablePage(t.connId, {
            schema: t.schema,
            table: t.table,
            filters: t.filters ?? [],
            sort: t.sort ?? null,
            limit: t.pageSize ?? 300,
            offset: t.offset ?? 0,
            count: true,
          });
          setConnected((c) => ({ ...c, [t.connId]: true }));
          patchTab(t.id, {
            output: { results: [page.result], elapsed_ms: page.elapsed_ms },
            total: page.total,
            sql: page.sql,
            running: false,
            resultIndex: 0,
            pending: undefined,
          });
        } catch (e) {
          patchTab(t.id, { error: String(e), running: false, output: undefined });
        }
        return;
      }
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

  /** Run, but stop for a typed confirmation on DROP / TRUNCATE / unfiltered DELETE & UPDATE. */
  const runTab = useCallback(
    async (t: Tab, sqlOverride?: string) => {
      const sql = sqlOverride ?? t.sql;
      if (t.kind === "query") {
        const items = await api.checkDestructive(sql).catch(() => [] as Destructive[]);
        if (items.length) {
          // close editor popups (autocomplete) before the modal takes over
          (document.activeElement as HTMLElement | null)?.blur();
          return setConfirm({ tab: t, sql, items });
        }
      }
      execTab(t, sqlOverride);
    },
    [execTab],
  );

  const selectConnection = useCallback(
    (id: string) => {
      touchRecent(id);
      setActiveConn(id);
      if (!tables[id]?.length) loadTables(id);
    },
    [tables, loadTables],
  );

  const newQuery = useCallback(
    (sql?: string) => {
      if (!activeConn) return;
      if (sql === undefined) {
        const k = connections.find((c) => c.id === activeConn)?.kind;
        sql = k && kindInfo(k).dialect === "redis" ? "PING" : k === "mongodb" ? "show collections" : "SELECT 1;";
      }
      const n = tabs.filter((t) => t.kind === "query" && t.connId === activeConn).length + 1;
      addTab({ id: uid(), kind: "query", title: `Query ${n}`, connId: activeConn, sql });
    },
    [activeConn, tabs, addTab, connections],
  );

  const openErd = useCallback(() => {
    if (!activeConn) return;
    const existing = tabs.find((t) => t.kind === "erd" && t.connId === activeConn);
    if (existing) return selectTab(existing.id);
    addTab({ id: uid(), kind: "erd", title: "Diagram", connId: activeConn, sql: "" });
  }, [activeConn, tabs, addTab, selectTab]);

  const pickHistory = useCallback(
    (sql: string, snippet?: Snippet) => {
      if (snippet) {
        if (!activeConn) return;
        const open = tabs.find((t) => t.snippetId === snippet.id && t.connId === activeConn);
        if (open) return selectTab(open.id);
        return addTab({ id: uid(), kind: "query", title: snippet.name, connId: activeConn, sql, snippetId: snippet.id });
      }
      if (tab?.kind === "query") patchTab(tab.id, { sql });
      else newQuery(sql);
    },
    [tab, patchTab, newQuery, activeConn, tabs, selectTab, addTab],
  );

  const openDatabaseFile = useCallback(async () => {
    const path = await dialogs.openAny("Database file", ["db", "sqlite", "sqlite3", "duckdb", "ddb"]);
    if (!path) return;
    const kind = /\.(duckdb|ddb)$/i.test(path) ? "duckdb" : "sqlite";
    const name = path.split(/[\\/]/).pop() ?? "database";
    const existing = connections.find((c) => c.file === path);
    if (existing) return selectConnection(existing.id);
    try {
      const saved = await api.saveConnection({ ...newConnection(kind), name, file: path });
      setConnections((cs) => [...cs, saved]);
      selectConnection(saved.id);
    } catch (e) {
      setToast(String(e));
    }
  }, [connections, selectConnection, setToast]);

  const openCompare = useCallback(() => {
    if (!activeConn) return;
    const existing = tabs.find((t) => t.kind === "compare" && t.connId === activeConn);
    if (existing) return selectTab(existing.id);
    addTab({ id: uid(), kind: "compare", title: "Compare", connId: activeConn, sql: "" });
  }, [activeConn, tabs, addTab, selectTab]);

  /** Open SQL in a new query tab on (possibly another) connection. */
  const openSqlOn = useCallback(
    (connId: string, sql: string) => {
      setActiveConn(connId);
      if (!tables[connId]?.length) loadTables(connId);
      const t: Tab = { id: uid(), kind: "query", title: "Migration", connId, sql, pane: focused };
      setTabs((ts) => [...ts, t]);
      setActiveByPane((a) => (focused === 0 ? [t.id, a[1]] : [a[0], t.id]));
    },
    [tables, loadTables, focused],
  );

  const openTable = useCallback(
    async (info: TableInfo) => {
      if (!activeConn) return;
      const existing = tabs.find(
        (t) => t.kind === "table" && t.connId === activeConn && t.schema === info.schema && t.table === info.name,
      );
      if (existing) return selectTab(existing.id);
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
      addTab(t);
      runTab(t);
      // Column metadata (primary key) decides whether the grid is editable.
      api
        .describeTable(activeConn, info.schema, info.name)
        .then((columns) => patchTab(t.id, { columns }))
        .catch((e) => setToast(String(e)));
    },
    [activeConn, tabs, runTab, patchTab, addTab, selectTab, setToast],
  );

  const closeTab = useCallback((id: string) => {
    // the pane falls back to its last remaining tab; an empty right pane closes the split
    setTabs((ts) => ts.filter((t) => t.id !== id));
  }, []);

  const splitRight = useCallback(() => {
    const left = paneTabs(0);
    const cur = activeIn(0);
    if (cur && left.length > 1) moveTab(cur.id, 1, null);
    else if (activeConn) {
      const n = tabs.filter((t) => t.kind === "query" && t.connId === activeConn).length + 1;
      const t: Tab = { id: uid(), kind: "query", title: `Query ${n}`, connId: activeConn, sql: "SELECT 1;", pane: 1 };
      setTabs((ts) => [...ts, t]);
      setActiveByPane((a) => [a[0], t.id]);
      setFocusedPane(1);
    }
  }, [paneTabs, activeIn, moveTab, activeConn, tabs]);

  const setView = useCallback(
    async (t: Tab, v: "data" | "structure") => {
      patchTab(t.id, { view: v });
      if (!t.columns && t.schema && t.table) {
        try {
          patchTab(t.id, { columns: await api.describeTable(t.connId, t.schema, t.table) });
        } catch (e) {
          setToast(String(e));
        }
      }
    },
    [patchTab, setToast],
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
      else if (k === "y") (e.preventDefault(), setHistoryOpen((h) => !h));
      else if (k === "\\") (e.preventDefault(), splitRight());
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newQuery, closeTab, activeTab, splitRight]);

  const schemas = useMemo(() => [...new Set(connTables.map((t) => t.schema))], [connTables]);
  const env = conn?.env && conn.env !== "none" ? ENVIRONMENTS.find((e) => e.value === conn.env) : undefined;
  const frame =
    conn && (conn.env === "production" || conn.env === "staging")
      ? (TAG_COLORS[conn.color ?? env?.color ?? "red"] ?? conn.color ?? "#ef4444")
      : null;

  const renderPane = (p: 0 | 1) => {
    const t = activeIn(p);
    const isFocused = focused === p;
    return (
      <div
        className={cx("@container flex min-w-0 flex-1 flex-col", p === 1 && "border-l border-line")}
        onMouseDownCapture={() => p !== focusedPane && setFocusedPane(p)}
      >
        <TabBar
          tabs={paneTabs(p)}
          activeId={t?.id ?? null}
          focused={isFocused && split}
          onSelect={selectTab}
          onClose={closeTab}
          onNew={() => (setFocusedPane(p), newQuery())}
          onDropTab={(id, before) => moveTab(id, p, before)}
          onSplit={!split && p === 0 ? splitRight : undefined}
          onDragState={setDraggingTab}
        />
        {t ? (
          t.kind === "compare" ? (
            <CompareView key={t.id} connections={connections} currentId={t.connId} onOpenSql={openSqlOn} onToast={setToast} />
          ) : t.kind === "erd" ? (
            <ErdView
              key={t.id}
              connId={t.connId}
              schemas={schemas}
              onToast={setToast}
              onOpenTable={(schema, name) => {
                const info = connTables.find((x) => x.schema === schema && x.name === name);
                if (info) openTable(info);
              }}
            />
          ) : (
            <Workspace
              key={t.id}
              tab={t}
              conn={conn ?? undefined}
              tables={connTables}
              active={isFocused}
              onChange={(patch) => patchTab(t.id, patch)}
              onRun={(sql) => runTab(t, sql)}
              onBrowse={(patch) => {
                const next = { ...t, ...patch };
                patchTab(t.id, patch);
                execTab(next);
              }}
              onView={(v) => setView(t, v)}
              onToast={setToast}
              onHistory={() => setHistoryOpen((h) => !h)}
              onSchedule={(sql) => setSchedules({ draft: { connectionId: t.connId, sql, name: t.snippetId ? t.title : "" } })}
              aiEnabled={aiEnabled}
              inlineAi={inlineAi}
              onOpenSettings={() => setSettingsOpen(true)}
            />
          )
        ) : (
          <Empty>
            <p className="text-xs text-muted">Pick a table on the left, start a query, or open the diagram.</p>
            <div className="flex gap-2">
              <Button variant="primary" onClick={() => newQuery()}>
                New query
              </Button>
              <Button onClick={openErd}>Diagram</Button>
            </div>
            <Shortcuts />
          </Empty>
        )}
      </div>
    );
  };

  return (
    <div className="flex h-full flex-col">
      {frame && (
        <div
          className="pointer-events-none fixed inset-0 z-[60] rounded-[inherit]"
          style={{ boxShadow: `inset 0 0 0 2px ${frame}` }}
          title={`${env?.label} connection`}
        />
      )}
      <Titlebar
        conn={conn}
        dark={dark}
        onToggleTheme={() => setDark((d) => !d)}
        onToggleSidebar={() => setSidebar((s) => !s)}
        onPalette={() => setPalette(true)}
        onMcp={() => setMcpOpen(true)}
        onSettings={() => setSettingsOpen(true)}
        onHome={() => setActiveConn(null)}
      />

      <div className="flex min-h-0 flex-1">
        {conn && (
        <div className={cx("shrink-0 overflow-hidden border-r border-line transition-[width] duration-150", sidebar && conn ? "w-60" : "w-0 border-r-0")}>
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
              onImport={() => setImportOpen(true)}
              onErd={openErd}
              onCompare={openCompare}
            />
          </div>
        </div>
        )}

        <main className="relative flex min-w-0 flex-1">
          {conn ? (
            <>
              {renderPane(0)}
              {split && renderPane(1)}
              {draggingTab && !split && paneTabs(0).length > 1 && (
                <div
                  className="animate-fade absolute inset-y-0 right-0 z-20 flex w-1/3 items-center justify-center border-2 border-dashed border-accent bg-accent-soft/60 text-xs font-medium text-accent"
                  onDragOver={(e) => e.preventDefault()}
                  onDrop={(e) => {
                    const id = e.dataTransfer.getData("application/x-rowhouse-tab");
                    setDraggingTab(null);
                    if (id) moveTab(id, 1, null);
                  }}
                >
                  Drop to split right
                </div>
              )}
            </>
          ) : (
            <Launcher
              connections={connections}
              version={version}
              onOpen={selectConnection}
              onEdit={(c) => setDialog({ open: true, edit: c })}
              onNew={() => setDialog({ open: true, edit: null })}
              onImport={() => setImportOpen(true)}
              onOpenFile={openDatabaseFile}
              onMcp={() => setMcpOpen(true)}
              onSettings={() => setSettingsOpen(true)}
            />
          )}
        </main>
        <HistoryPanel connId={activeConn} open={historyOpen && !!conn} onClose={() => setHistoryOpen(false)} onPick={pickHistory} />
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
      <SchedulesDialog open={!!schedules} draft={schedules?.draft} onClose={() => setSchedules(null)} connections={connections} />
      <ApprovalCenter connections={connections} />
      <UpdateBanner />
      <ImportDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={(saved) => {
          setConnections((cs) => [...cs, ...saved]);
          setImportOpen(false);
          setToast(`Imported ${saved.length} connection${saved.length === 1 ? "" : "s"} — enter passwords on first connect`, "ok");
        }}
      />
      <ConfirmDestructive
        items={confirm?.items ?? null}
        conn={conn}
        onCancel={() => setConfirm(null)}
        onConfirm={() => {
          if (confirm) execTab(confirm.tab, confirm.sql);
          setConfirm(null);
        }}
      />
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onSaved={(s) => (setAiEnabled(s.enabled && !!s.model), setInlineAi(s.enabled && !!s.inlineCompletions))}
      />
      <CommandPalette
        open={palette}
        onClose={() => setPalette(false)}
        connections={connections}
        tables={connTables}
        actions={{
          newQuery: () => newQuery(),
          openErd,
          openCompare,
          importConnections: () => setImportOpen(true),
          splitRight,
          showHistory: () => setHistoryOpen(true),
          showSettings: () => setSettingsOpen(true),
          newConnection: () => setDialog({ open: true, edit: null }),
          openTable,
          selectConnection,
          toggleTheme: () => setDark((d) => !d),
          showMcp: () => setMcpOpen(true),
          showSchedules: () => setSchedules({ draft: null }),
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
    ["Query history", [mod, "Y"]],
    ["Ask AI", [mod, "I"]],
    ["Split right", [mod, "\\"]],
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
