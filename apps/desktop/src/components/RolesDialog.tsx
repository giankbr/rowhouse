import { KeyRound, Plus, RefreshCw, ShieldCheck, Trash2, UserRound, UsersRound } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, kindInfo, type ConnectionConfig, type RoleGrant, type RoleInfo, type RoleOp, type TableInfo } from "../lib/api";
import { Button, Input, Modal, Switch, cx } from "./ui";

const PRIVS = ["SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER"];
const MY_PRIVS = ["SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "INDEX", "EXECUTE"];
const MS_PRIVS = ["SELECT", "INSERT", "UPDATE", "DELETE", "EXECUTE", "REFERENCES", "ALTER"];

type Pane = { kind: "view" } | { kind: "create" } | { kind: "grant" } | { kind: "password" } | { kind: "drop" };

/** Users & roles with privileges; every change is shown as SQL before it runs. */
export function RolesDialog({
  conn,
  open,
  tables,
  onClose,
  onToast,
}: {
  conn: ConnectionConfig | null;
  open: boolean;
  tables: TableInfo[];
  onClose: () => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const dialect = conn ? kindInfo(conn.kind).dialect : "postgres";
  const [roles, setRoles] = useState<RoleInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [grants, setGrants] = useState<RoleGrant[] | null>(null);
  const [pane, setPane] = useState<Pane>({ kind: "view" });
  const [filter, setFilter] = useState("");

  const load = useCallback(() => {
    if (!conn) return;
    setError(null);
    api
      .listRoles(conn.id)
      .then((r) => {
        setRoles(r);
        setSel((s) => (s && r.some((x) => x.id === s) ? s : (r.find((x) => x.login && !x.superuser) ?? r[0])?.id ?? null));
      })
      .catch((e) => setError(String(e)));
  }, [conn]);
  useEffect(() => {
    if (!open) return;
    setRoles(null);
    setPane({ kind: "view" });
    load();
  }, [open, load]);
  const loadGrants = useCallback(() => {
    if (!conn || !sel) return setGrants(null);
    setGrants(null);
    api
      .roleGrants(conn.id, sel)
      .then(setGrants)
      .catch((e) => (setGrants([]), onToast(String(e))));
  }, [conn, sel, onToast]);
  useEffect(() => {
    if (open) loadGrants();
  }, [open, loadGrants]);

  const role = roles?.find((r) => r.id === sel) ?? null;
  const shown = (roles ?? []).filter((r) => !filter || r.id.toLowerCase().includes(filter.toLowerCase()));

  const apply = async (op: RoleOp, msg: string) => {
    if (!conn) return false;
    try {
      await api.roleAction(conn.id, op, false);
      onToast(msg, "ok");
      setPane({ kind: "view" });
      load();
      loadGrants();
      return true;
    } catch (e) {
      onToast(String(e));
      return false;
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={900}
      title={
        <span className="flex items-center gap-2">
          <UsersRound size={14} className="text-accent" /> Users & roles{conn ? <span className="font-normal text-muted">· {conn.name}</span> : null}
        </span>
      }
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">{conn?.readOnly ? "Read-only connection — changes are disabled" : "Every change shows its SQL before it runs"}</span>
          <Button variant="ghost" size="sm" onClick={() => (load(), loadGrants())}>
            <RefreshCw size={12} /> Refresh
          </Button>
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      {error ? (
        <div className="selectable rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-2xs text-danger">{error}</div>
      ) : (
        <div className="-mx-4 -my-3 flex h-[58vh]">
          <div className="flex w-60 shrink-0 flex-col border-r border-line">
            <div className="flex gap-1 p-2">
              <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter" className="h-6" />
              <Button size="icon" variant="ghost" title={dialect === "mysql" ? "New user" : "New role / user"} disabled={conn?.readOnly} onClick={() => setPane({ kind: "create" })}>
                <Plus size={14} />
              </Button>
            </div>
            <div className="flex-1 overflow-y-auto px-1.5 pb-2">
              {roles === null && <div className="p-4 text-center text-xs text-subtle">Loading…</div>}
              {shown.map((r) => (
                <button
                  key={r.id}
                  onClick={() => (setSel(r.id), setPane({ kind: "view" }))}
                  className={cx("flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-xs", r.id === sel ? "bg-selected text-fg" : "text-fg/85 hover:bg-hover")}
                >
                  {r.login ? <UserRound size={12} className="shrink-0 text-subtle" /> : <UsersRound size={12} className="shrink-0 text-subtle" />}
                  <span className="truncate">{r.name}</span>
                  {r.host && r.host !== "%" && <span className="truncate text-2xs text-subtle">@{r.host}</span>}
                  {r.superuser && <ShieldCheck size={12} className="ml-auto shrink-0 text-warning" />}
                </button>
              ))}
            </div>
          </div>

          <div className="min-w-0 flex-1 overflow-y-auto p-4">
            {pane.kind === "create" ? (
              <CreateForm conn={conn!} dialect={dialect} onCancel={() => setPane({ kind: "view" })} onApply={(op) => apply(op, `Created ${op.op === "create" ? op.name : ""}`)} />
            ) : !role ? (
              <div className="flex h-full items-center justify-center text-xs text-subtle">Pick a user or role</div>
            ) : pane.kind === "grant" ? (
              <GrantForm conn={conn!} dialect={dialect} role={role} tables={tables} onCancel={() => setPane({ kind: "view" })} onApply={(op) => apply(op, "Privileges granted")} />
            ) : pane.kind === "password" ? (
              <PasswordForm conn={conn!} role={role} onCancel={() => setPane({ kind: "view" })} onApply={(op) => apply(op, "Password changed")} />
            ) : pane.kind === "drop" ? (
              <DropForm conn={conn!} role={role} onCancel={() => setPane({ kind: "view" })} onApply={(op) => apply(op, `Dropped ${role.name}`)} />
            ) : (
              <div className="flex flex-col gap-4">
                <div className="flex items-start gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-sm font-semibold">
                      {role.name}
                      {role.host && <span className="font-mono text-2xs font-normal text-muted">@{role.host}</span>}
                    </div>
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      <Badge tone={role.login ? "ok" : "muted"}>{role.login ? "can log in" : "no login"}</Badge>
                      {role.superuser && <Badge tone="warn">superuser</Badge>}
                      {role.attributes.map((a) => (
                        <Badge key={a}>{a}</Badge>
                      ))}
                      {role.memberOf.map((m) => (
                        <Badge key={m} tone="accent">
                          member of {m}
                        </Badge>
                      ))}
                    </div>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button size="xs" disabled={conn?.readOnly} onClick={() => setPane({ kind: "grant" })}>
                      <Plus size={11} /> Grant…
                    </Button>
                    {role.login && (
                      <Button size="xs" variant="ghost" disabled={conn?.readOnly} onClick={() => setPane({ kind: "password" })}>
                        <KeyRound size={11} /> Password
                      </Button>
                    )}
                    <Button size="xs" variant="danger" disabled={conn?.readOnly} onClick={() => setPane({ kind: "drop" })}>
                      <Trash2 size={11} /> Drop
                    </Button>
                  </div>
                </div>

                <div>
                  <h3 className="mb-1.5 text-2xs font-medium tracking-wide text-subtle uppercase">Privileges</h3>
                  {grants === null ? (
                    <div className="text-xs text-subtle">Loading…</div>
                  ) : grants.length === 0 ? (
                    <div className="rounded-lg border border-dashed border-line p-4 text-center text-xs text-subtle">
                      {role.superuser ? "Superusers can do everything — no explicit grants needed." : "No privileges granted yet."}
                    </div>
                  ) : (
                    <table className="w-full table-fixed border-collapse text-xs">
                      <thead className="text-left text-2xs text-muted">
                        <tr className="h-7 border-b border-line">
                          <th className="w-40 font-medium">{dialect === "mysql" ? "Database" : "Schema"}</th>
                          <th className="w-44 font-medium">Object</th>
                          <th className="font-medium">Privileges</th>
                          <th className="w-20" />
                        </tr>
                      </thead>
                      <tbody>
                        {grants.map((g, i) => (
                          <tr key={i} className="group h-8 border-b border-line/60 align-middle">
                            <td className="truncate font-mono text-[11.5px]">{g.schema || "*"}</td>
                            <td className="truncate font-mono text-[11.5px] text-muted">{g.object || (g.schema ? "all" : "*")}</td>
                            <td>
                              <span className="flex flex-wrap gap-1">
                                {g.privileges.map((p) => (
                                  <span key={p} className={cx("rounded px-1.5 py-px text-[10.5px]", /^DENY/.test(p) ? "bg-danger/10 text-danger" : "bg-hover text-fg/80")}>
                                    {p}
                                  </span>
                                ))}
                              </span>
                            </td>
                            <td className="text-right">
                              {g.schema && !g.privileges.some((p) => /^(USAGE|DENY|PROXY)/.test(p)) && (
                                <Button
                                  size="xs"
                                  variant="ghost"
                                  className="opacity-0 group-hover:opacity-100"
                                  disabled={conn?.readOnly}
                                  onClick={() =>
                                    apply(
                                      { op: "revoke", id: role.id, privileges: g.privileges.map((p) => p.replace(/ WITH GRANT OPTION$/, "")), schema: g.schema, table: g.object || null },
                                      "Privileges revoked",
                                    )
                                  }
                                >
                                  Revoke
                                </Button>
                              )}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}

function Badge({ children, tone = "muted" }: { children: React.ReactNode; tone?: "ok" | "warn" | "muted" | "accent" }) {
  return (
    <span
      className={cx(
        "rounded px-1.5 py-px text-[10.5px]",
        tone === "ok" && "bg-success/12 text-success",
        tone === "warn" && "bg-warning/15 text-warning",
        tone === "accent" && "bg-accent-soft text-accent",
        tone === "muted" && "bg-hover text-muted",
      )}
    >
      {children}
    </span>
  );
}

/** Shows the SQL for `op` (passwords masked) and runs it. */
function Preview({ conn, op, onCancel, onApply, label, danger }: { conn: ConnectionConfig; op: RoleOp | null; onCancel: () => void; onApply: (op: RoleOp) => void; label: string; danger?: boolean }) {
  const [sql, setSql] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const key = JSON.stringify(op);
  useEffect(() => {
    if (!op) return (setSql(null), setErr(null));
    const t = setTimeout(
      () =>
        api
          .roleAction(conn.id, op, true)
          .then((s) => (setSql(s), setErr(null)))
          .catch((e) => (setSql(null), setErr(String(e)))),
      120,
    );
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn.id, key]);
  return (
    <>
      {err ? (
        <div className="rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">{err}</div>
      ) : (
        <pre className="selectable min-h-10 overflow-auto rounded-lg border border-line bg-panel p-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">
          {sql?.map((x) => `${x};`).join("\n") ?? " "}
        </pre>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button variant={danger ? "default" : "primary"} className={cx(danger && "border-danger/40 text-danger hover:bg-danger/10")} disabled={!sql || !!err || !op || conn.readOnly} onClick={() => op && onApply(op)}>
          {label}
        </Button>
      </div>
    </>
  );
}

function CreateForm({ conn, dialect, onCancel, onApply }: { conn: ConnectionConfig; dialect: string; onCancel: () => void; onApply: (op: RoleOp) => void }) {
  const [name, setName] = useState("");
  const [host, setHost] = useState("%");
  const [password, setPassword] = useState("");
  const [login, setLogin] = useState(true);
  const [superuser, setSuperuser] = useState(false);
  const [createDb, setCreateDb] = useState(false);
  const op: RoleOp | null = name.trim() ? { op: "create", name, host: dialect === "mysql" ? host : null, password: password || null, login, superuser, createDb } : null;
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold">{dialect === "mysql" ? "New user" : "New role or user"}</h3>
      <div className="flex gap-2">
        <label className="flex flex-1 flex-col gap-1 text-2xs text-muted">
          Name
          <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="app_reader" />
        </label>
        {dialect === "mysql" && (
          <label className="flex w-40 flex-col gap-1 text-2xs text-muted">
            Host
            <Input value={host} onChange={(e) => setHost(e.target.value)} placeholder="%" />
          </label>
        )}
      </div>
      <label className="flex flex-col gap-1 text-2xs text-muted">
        Password
        <Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder={dialect === "postgres" && !login ? "not needed for a group role" : ""} />
      </label>
      <div className="flex flex-wrap gap-x-5 gap-y-2">
        {dialect !== "mysql" && <Switch checked={login} onChange={setLogin} label={<span className="text-xs">Can log in</span>} />}
        <Switch checked={superuser} onChange={setSuperuser} label={<span className="text-xs">{dialect === "mssql" ? "db_owner" : "Superuser"}</span>} />
        {dialect !== "mssql" && <Switch checked={createDb} onChange={setCreateDb} label={<span className="text-xs">Can create databases</span>} />}
      </div>
      <Preview conn={conn} op={op} onCancel={onCancel} onApply={onApply} label="Create" />
    </div>
  );
}

function GrantForm({ conn, dialect, role, tables, onCancel, onApply }: { conn: ConnectionConfig; dialect: string; role: RoleInfo; tables: TableInfo[]; onCancel: () => void; onApply: (op: RoleOp) => void }) {
  const schemas = useMemo(() => [...new Set(tables.map((t) => t.schema))], [tables]);
  const [schema, setSchema] = useState(schemas[0] ?? (dialect === "postgres" ? "public" : conn.database ?? ""));
  const [table, setTable] = useState("");
  const [privs, setPrivs] = useState<string[]>(["SELECT"]);
  const list = dialect === "mysql" ? MY_PRIVS : dialect === "mssql" ? MS_PRIVS : PRIVS;
  const op: RoleOp | null = privs.length && schema ? { op: "grant", id: role.id, privileges: privs, schema, table: table || null } : null;
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold">Grant to {role.name}</h3>
      <div className="flex gap-2">
        <label className="flex w-48 flex-col gap-1 text-2xs text-muted">
          {dialect === "mysql" ? "Database" : "Schema"}
          <select value={schema} onChange={(e) => (setSchema(e.target.value), setTable(""))} className="h-7 rounded-md border border-line bg-elevated px-1.5 text-xs text-fg outline-none">
            {(schemas.length ? schemas : [schema]).map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label className="flex flex-1 flex-col gap-1 text-2xs text-muted">
          Table
          <select value={table} onChange={(e) => setTable(e.target.value)} className="h-7 rounded-md border border-line bg-elevated px-1.5 text-xs text-fg outline-none">
            <option value="">All tables in {schema || "the schema"}</option>
            {tables
              .filter((t) => t.schema === schema)
              .map((t) => (
                <option key={t.name} value={t.name}>
                  {t.name}
                </option>
              ))}
          </select>
        </label>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {list.map((p) => (
          <button
            key={p}
            onClick={() => setPrivs((x) => (x.includes(p) ? x.filter((y) => y !== p) : [...x, p]))}
            className={cx("h-6 rounded-md border px-2 text-2xs font-medium", privs.includes(p) ? "border-accent bg-accent-soft text-accent" : "border-line text-muted hover:bg-hover")}
          >
            {p}
          </button>
        ))}
        <button onClick={() => setPrivs(["SELECT"])} className="h-6 px-2 text-2xs text-subtle hover:text-fg">
          read-only
        </button>
        <button onClick={() => setPrivs(["SELECT", "INSERT", "UPDATE", "DELETE"])} className="h-6 px-2 text-2xs text-subtle hover:text-fg">
          read-write
        </button>
      </div>
      <Preview conn={conn} op={op} onCancel={onCancel} onApply={onApply} label="Grant" />
    </div>
  );
}

function PasswordForm({ conn, role, onCancel, onApply }: { conn: ConnectionConfig; role: RoleInfo; onCancel: () => void; onApply: (op: RoleOp) => void }) {
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const op: RoleOp | null = pw && pw === pw2 ? { op: "password", id: role.id, password: pw } : null;
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold">New password for {role.name}</h3>
      <Input type="password" autoFocus autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="New password" />
      <Input type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} placeholder="Repeat" />
      {pw2 && pw !== pw2 && <span className="text-2xs text-danger">The passwords don't match</span>}
      <Preview conn={conn} op={op} onCancel={onCancel} onApply={onApply} label="Change password" />
    </div>
  );
}

function DropForm({ conn, role, onCancel, onApply }: { conn: ConnectionConfig; role: RoleInfo; onCancel: () => void; onApply: (op: RoleOp) => void }) {
  const [confirm, setConfirm] = useState("");
  return (
    <div className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold text-danger">Drop {role.name}</h3>
      <p className="text-xs text-muted">
        Removes the {role.login ? "user" : "role"}. On PostgreSQL, objects it owns or privileges granted to it must be reassigned or revoked first. Type{" "}
        <span className="font-mono font-medium text-fg">{role.name}</span> to confirm.
      </p>
      <Input autoFocus value={confirm} onChange={(e) => setConfirm(e.target.value)} placeholder={role.name} />
      <Preview conn={conn} op={confirm === role.name ? { op: "drop", id: role.id } : null} onCancel={onCancel} onApply={onApply} label="Drop" danger />
    </div>
  );
}
