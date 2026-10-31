import { CheckCircle2, Link2, Loader2, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, dialogs, ENVIRONMENTS, kindInfo, KINDS, newConnection, type ConnectionConfig, type DbKind, type Environment, type TlsMode } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Field, Input, Modal, Segmented, Switch, TAG_COLORS, cx } from "./ui";

export function ConnectionDialog({
  open,
  initial,
  onClose,
  onSaved,
  onDeleted,
  initialKind,
}: {
  open: boolean;
  initial: ConnectionConfig | null;
  /** kind preselected for a new connection */
  initialKind?: DbKind;
  onClose: () => void;
  onSaved: (c: ConnectionConfig) => void;
  onDeleted: (id: string) => void;
}) {
  const [c, setC] = useState<ConnectionConfig>(newConnection());
  const [password, setPassword] = useState<string>("");
  const [sshSecret, setSshSecret] = useState<string>("");
  const [test, setTest] = useState<{ state: "idle" | "busy" | "ok" | "err"; msg?: string }>({ state: "idle" });
  const isNew = !initial;

  useEffect(() => {
    if (open) {
      setC(initial ?? { ...newConnection(initialKind), host: null, port: null, user: null });
      setTabSel("general");
      setKindFilter("");
      setPassword("");
      setSshSecret("");
      setUrl("");
      setUrlError(null);
      setTest({ state: "idle" });
    }
  }, [open, initial]);

  const set = (patch: Partial<ConnectionConfig>) => setC((x) => ({ ...x, ...patch }));
  const info = kindInfo(c.kind);
  const [url, setUrl] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);
  const applyUrl = async (u: string) => {
    setUrl(u);
    if (!u.includes("://")) return setUrlError(null);
    try {
      const r = await api.parseConnectionUrl(u);
      setC((x) => ({ ...r.config, id: x.id, env: x.env, color: x.color, readOnly: x.readOnly, mcpEnabled: x.mcpEnabled }));
      if (r.password) setPassword(r.password);
      setUrlError(null);
    } catch (e) {
      setUrlError(String(e));
    }
  };
  const tls = c.tls ?? { mode: "prefer" as TlsMode };
  const setTls = (p: Partial<NonNullable<ConnectionConfig["tls"]>>) => set({ tls: { ...tls, ...p } });
  // Empty password field on an existing profile = keep the stored one.
  const pw = password === "" && !isNew ? undefined : password;
  const sshPw = sshSecret === "" && !isNew ? undefined : sshSecret;
  const ssh = c.ssh;
  const setSsh = (patch: Partial<NonNullable<ConnectionConfig["ssh"]>>) =>
    setC((x) => ({ ...x, ssh: x.ssh ? { ...x.ssh, ...patch } : null }));

  const runTest = async () => {
    setTest({ state: "busy" });
    try {
      const fp = await api.testConnection(withDefaults(c), pw, sshPw);
      if (fp && c.ssh && !c.ssh.hostKey) setSsh({ hostKey: fp }); // pin on save
      setTest({ state: "ok", msg: "Connection successful" });
    } catch (e) {
      setTest({ state: "err", msg: String(e) });
    }
  };

  const save = async () => {
    const full = withDefaults(c);
    const saved = await api.saveConnection({ ...full, name: c.name.trim() || defaultName(full) }, pw, sshPw);
    onSaved(saved);
  };

  const blank = (k: DbKind) => ({ ...newConnection(k), host: null, port: null, user: null });
  const [tabSel, setTabSel] = useState<"general" | "ssh" | "tls" | "safety">("general");
  const [kindFilter, setKindFilter] = useState("");
  const shownKinds = KINDS.filter((k) => !kindFilter || k.label.toLowerCase().includes(kindFilter.toLowerCase()));
  const hasNet = !info.file && !info.api;
  const dialect = info.dialect;
  const hostPh = dialect === "oracle" ? "db.example.com (or a TNS alias / EZConnect string)" : c.kind === "mongodb" ? "localhost or mongodb+srv://…" : dialect === "cql" ? "node1, node2:9042" : c.kind === "neon" ? "ep-xxx.region.aws.neon.tech" : c.kind === "supabase" ? "db.xxxx.supabase.co" : c.kind === "upstash" ? "xxx.upstash.io" : dialect === "elastic" ? "localhost or https://my-deploy.es.io:443" : "localhost";
  const tabs = [
    { id: "general" as const, label: "General" },
    ...(hasNet ? [{ id: "ssh" as const, label: ssh ? "SSH · on" : "SSH" }, { id: "tls" as const, label: "TLS" }] : []),
    { id: "safety" as const, label: "Safety & AI" },
  ];

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={isNew ? 780 : 600}
      title={isNew ? "New connection" : `Edit · ${initial?.name}`}
      footer={
        <>
          {!isNew && (
            <Button variant="danger" size="sm" onClick={async () => (await api.deleteConnection(c.id), onDeleted(c.id))}>
              Delete
            </Button>
          )}
          <TestStatus {...test} />
          <span className="flex-1" />
          <Button onClick={runTest} disabled={test.state === "busy"}>
            Test
          </Button>
          <Button variant="primary" onClick={save}>
            {isNew ? "Connect" : "Save"}
          </Button>
        </>
      }
    >
      <div className={cx("-mx-4 -my-3 flex", isNew ? "h-[min(520px,62vh)]" : "max-h-[62vh]")}>
        {isNew && (
          <div className="flex w-52 shrink-0 flex-col border-r border-line bg-panel/60">
            <div className="p-2">
              <input
                value={kindFilter}
                onChange={(e) => setKindFilter(e.target.value)}
                placeholder="Search databases"
                spellCheck={false}
                className="h-7 w-full rounded-md border border-line bg-elevated px-2 text-xs outline-none focus:border-accent"
              />
            </div>
            <div className="flex-1 overflow-y-auto px-1.5 pb-2">
              {KIND_GROUPS.map((g) => {
                const list = shownKinds.filter((k) => g.kinds.includes(k.kind));
                if (!list.length) return null;
                return (
                  <div key={g.label} className="mb-1.5">
                    <div className="px-2 pt-1 pb-0.5 text-[10px] font-semibold tracking-wide text-subtle uppercase">{g.label}</div>
                    {list.map((k) => (
                      <button
                        type="button"
                        key={k.kind}
                        onClick={() => setC((x) => ({ ...blank(k.kind), id: x.id, name: x.name, env: x.env, color: x.color, readOnly: x.readOnly, mcpEnabled: x.mcpEnabled, ssh: k.file || k.api ? null : x.ssh, tls: k.api ? null : (newConnection(k.kind).tls ?? x.tls ?? null) }))}
                        className={cx(
                          "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-xs",
                          c.kind === k.kind ? "bg-selected text-fg" : "text-fg/80 hover:bg-hover",
                        )}
                      >
                        <KindBadge kind={k.kind} size={16} />
                        <span className="truncate">{k.label}</span>
                        {k.beta && <span className="ml-auto rounded bg-warning/15 px-1 text-[9px] font-medium text-warning">beta</span>}
                      </button>
                    ))}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        <form className="flex min-w-0 flex-1 flex-col" onSubmit={(e) => (e.preventDefault(), save())}>
          <div className="flex items-center gap-2 border-b border-line px-4 py-2.5">
            <KindBadge kind={c.kind} size={22} />
            <div className="min-w-0 flex-1">
              <input
                autoFocus
                value={c.name}
                placeholder={defaultName(withDefaults(c))}
                onChange={(e) => set({ name: e.target.value })}
                spellCheck={false}
                className="w-full bg-transparent text-sm font-semibold outline-none placeholder:font-normal placeholder:text-subtle"
              />
              <div className="text-2xs text-subtle">
                {info.label}
                {info.beta ? " · beta" : info.api ? " · over HTTPS" : info.port ? ` · port ${info.port}` : ""}
              </div>
            </div>
            <div className="flex items-center gap-1">
              {Object.entries(TAG_COLORS).map(([name, hex]) => (
                <button
                  type="button"
                  key={name}
                  title={name}
                  onClick={() => set({ color: c.color === name ? null : name })}
                  className={cx("size-3 rounded-full transition-transform hover:scale-110", c.color === name && "ring-2 ring-offset-1 ring-offset-elevated")}
                  style={{ background: hex, ["--tw-ring-color" as string]: hex }}
                />
              ))}
            </div>
          </div>

          <div className="flex gap-0.5 border-b border-line px-3">
            {tabs.map((t) => (
              <button
                type="button"
                key={t.id}
                onClick={() => setTabSel(t.id)}
                className={cx("-mb-px border-b-2 px-2.5 py-1.5 text-xs", tabSel === t.id ? "border-accent font-medium text-fg" : "border-transparent text-muted hover:text-fg")}
              >
                {t.label}
              </button>
            ))}
          </div>

          <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-4">
            {tabSel === "general" && (
              <>
                {isNew && (
                  <div className="flex flex-col gap-1">
                    <div className="relative">
                      <Link2 size={12} className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-subtle" />
                      <Input value={url} onChange={(e) => applyUrl(e.target.value)} placeholder="Paste a URL — postgresql://user:pass@host/db" className="pl-7 font-mono" />
                    </div>
                    {urlError && <span className="text-2xs text-danger">{urlError}</span>}
                  </div>
                )}
                {info.file ? (
                  <Field label="Database file" hint={c.kind === "duckdb" ? "Created if missing" : "An existing .db / .sqlite file"}>
                    <Input value={c.file ?? ""} placeholder={c.kind === "duckdb" ? "~/data/warehouse.duckdb" : "~/data/app.db"} onChange={(e) => set({ file: e.target.value })} />
                  </Field>
                ) : c.kind === "turso" ? (
                  <>
                    <Field label="Database URL" hint="From `turso db show --url`; a local sqld works too">
                      <Input value={c.host ?? ""} placeholder="libsql://my-db-me.turso.io" onChange={(e) => set({ host: e.target.value })} />
                    </Field>
                    <Field label="Auth token">
                      <Input type="password" value={password} placeholder={isNew ? "turso db tokens create <db>" : "•••••• (unchanged)"} onChange={(e) => setPassword(e.target.value)} />
                    </Field>
                  </>
                ) : c.kind === "bigquery" ? (
                  <>
                    <Field label="Service-account key file" hint="IAM → Service accounts → Keys → Add key (JSON). Needs BigQuery Job User + Data Viewer (or Editor to change data).">
                      <div className="flex gap-2">
                        <Input value={c.file ?? ""} placeholder="~/keys/my-project-1234.json" className="font-mono" onChange={(e) => set({ file: e.target.value || null })} />
                        <Button type="button" onClick={() => dialogs.openAny("Key file", ["json"]).then((f) => f && set({ file: f }))}>
                          Browse…
                        </Button>
                      </div>
                    </Field>
                    <div className="grid grid-cols-2 gap-2">
                      <Field label="Project ID">
                        <Input value={c.database ?? ""} placeholder="from the key file" onChange={(e) => set({ database: e.target.value || null })} />
                      </Field>
                      <Field label="Location">
                        <Input value={c.user ?? ""} placeholder="auto · US · EU · asia-southeast2" onChange={(e) => set({ user: e.target.value || null })} />
                      </Field>
                    </div>
                    <Field label="Access token" hint="Instead of a key file: paste the output of `gcloud auth print-access-token` (expires after an hour).">
                      <Input type="password" value={password} placeholder={isNew ? "optional" : "•••••• (unchanged)"} onChange={(e) => setPassword(e.target.value)} />
                    </Field>
                  </>
                ) : c.kind === "snowflake" ? (
                  <>
                    <div className="grid grid-cols-2 gap-2">
                      <Field label="Account identifier" hint="From the account URL: <identifier>.snowflakecomputing.com">
                        <Input value={c.host ?? ""} placeholder="myorg-myaccount" onChange={(e) => set({ host: e.target.value || null })} />
                      </Field>
                      <Field label="User">
                        <Input value={c.user ?? ""} placeholder="ANALYST" onChange={(e) => set({ user: e.target.value || null })} />
                      </Field>
                    </div>
                    <Field label="Private key (key-pair auth)" hint="Unencrypted PKCS#8 .p8 whose public key is set on the user (ALTER USER … SET RSA_PUBLIC_KEY).">
                      <div className="flex gap-2">
                        <Input value={c.file ?? ""} placeholder="~/.snowflake/rsa_key.p8" className="font-mono" onChange={(e) => set({ file: e.target.value || null })} />
                        <Button type="button" onClick={() => dialogs.openAny("Private key", ["p8", "pem"]).then((f) => f && set({ file: f }))}>
                          Browse…
                        </Button>
                      </div>
                    </Field>
                    <Field label="Programmatic access token" hint="Instead of a key: a PAT from Snowsight → your profile → Programmatic access tokens.">
                      <Input type="password" value={password} placeholder={isNew ? "optional" : "•••••• (unchanged)"} onChange={(e) => setPassword(e.target.value)} />
                    </Field>
                    <div className="grid grid-cols-3 gap-2">
                      <Field label="Database">
                        <Input value={c.database ?? ""} placeholder="SALES or SALES.PUBLIC" onChange={(e) => set({ database: e.target.value || null })} />
                      </Field>
                      <Field label="Warehouse">
                        <Input value={c.warehouse ?? ""} placeholder="COMPUTE_WH" onChange={(e) => set({ warehouse: e.target.value || null })} />
                      </Field>
                      <Field label="Role">
                        <Input value={c.role ?? ""} placeholder="default role" onChange={(e) => set({ role: e.target.value || null })} />
                      </Field>
                    </div>
                  </>
                ) : c.kind === "d1" ? (
                  <>
                    <div className="grid grid-cols-2 gap-2">
                      <Field label="Account ID">
                        <Input value={c.user ?? ""} placeholder="Cloudflare account ID" onChange={(e) => set({ user: e.target.value })} />
                      </Field>
                      <Field label="Database ID">
                        <Input value={c.database ?? ""} placeholder="D1 database UUID" onChange={(e) => set({ database: e.target.value || null })} />
                      </Field>
                    </div>
                    <Field label="API token" hint="Needs D1 Edit (or Read) permission">
                      <Input type="password" value={password} placeholder={isNew ? "" : "•••••• (unchanged)"} onChange={(e) => setPassword(e.target.value)} />
                    </Field>
                  </>
                ) : (
                  <>
                    <div className="grid grid-cols-[1fr_96px] gap-2">
                      <Field label={c.kind === "mongodb" ? "Host or URI" : dialect === "cql" ? "Contact points" : "Host"}>
                        <Input value={c.host ?? ""} placeholder={hostPh} onChange={(e) => set({ host: e.target.value || null })} />
                      </Field>
                      <Field label="Port">
                        <Input value={c.port ?? ""} placeholder={info.port ? String(info.port) : ""} inputMode="numeric" onChange={(e) => set({ port: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })} />
                      </Field>
                    </div>
                    <div className="grid grid-cols-2 gap-2">
                      <Field label="User" hint={dialect === "elastic" ? "Leave empty to use an API key" : undefined}>
                        <Input value={c.user ?? ""} placeholder={info.user ?? (dialect === "redis" ? "default" : dialect === "elastic" ? (c.kind === "opensearch" ? "admin" : "elastic") : "")} onChange={(e) => set({ user: e.target.value || null })} />
                      </Field>
                      <Field label={dialect === "elastic" ? "Password or API key" : "Password"}>
                        <Input type="password" value={password} placeholder={isNew ? "" : "•••••• (unchanged)"} onChange={(e) => setPassword(e.target.value)} />
                      </Field>
                    </div>
                    {dialect !== "elastic" && <Field label={dialect === "redis" ? "Database number" : dialect === "cql" ? "Keyspace" : dialect === "oracle" ? "Service name" : "Database"}>
                      <Input
                        value={c.database ?? ""}
                        placeholder={dialect === "redis" ? "0" : c.kind === "mongodb" ? "test" : dialect === "oracle" ? "FREEPDB1 · ORCLPDB1 · XEPDB1" : "optional — pick later from the titlebar"}
                        onChange={(e) => set({ database: e.target.value || null })}
                      />
                    </Field>}
                    {dialect === "oracle" && (
                      <Field label="Oracle Instant Client folder" hint="Rowhouse loads Oracle's client library at runtime. Leave empty to find it in ~/Downloads, ~/instantclient*, /opt/oracle… — or download “Basic Light” from oracle.com.">
                        <Input value={c.clientDir ?? ""} placeholder="auto-detect" onChange={(e) => set({ clientDir: e.target.value || null })} />
                      </Field>
                    )}
                  </>
                )}
                <Field label="Environment" hint={c.env === "production" ? "Production starts read-only, gets a red frame, and asks you to type the table name before destructive statements." : undefined}>
                  <Segmented
                    value={(c.env ?? "none") as Environment}
                    onChange={(env) => {
                      const e = ENVIRONMENTS.find((x) => x.value === env)!;
                      set({ env, color: c.color ?? e.color, ...(env === "production" ? { readOnly: true } : {}) });
                    }}
                    options={ENVIRONMENTS.map((e) => ({ value: e.value, label: e.label }))}
                  />
                </Field>
              </>
            )}

            {tabSel === "tls" && hasNet && (
              <>
                <Field label="TLS / SSL">
                  <Segmented
                    value={tls.mode}
                    onChange={(mode) => setTls({ mode })}
                    options={[
                      { value: "disable", label: "Off" },
                      { value: "prefer", label: "Prefer" },
                      { value: "require", label: "Require" },
                      { value: "verify-ca", label: "Verify CA" },
                      { value: "verify-full", label: "Verify full" },
                    ]}
                  />
                </Field>
                <Field label="CA certificate">
                  <Input value={tls.caCert ?? ""} placeholder="system roots" onChange={(e) => setTls({ caCert: e.target.value || null })} />
                </Field>
                <div className="grid grid-cols-2 gap-2">
                  <Field label="Client certificate">
                    <Input value={tls.clientCert ?? ""} placeholder="optional (mutual TLS)" onChange={(e) => setTls({ clientCert: e.target.value || null })} />
                  </Field>
                  <Field label="Client key">
                    <Input value={tls.clientKey ?? ""} placeholder="optional" onChange={(e) => setTls({ clientKey: e.target.value || null })} />
                  </Field>
                </div>
                {info.tls && tls.mode !== "require" && tls.mode !== "verify-full" && tls.mode !== "verify-ca" && (
                  <span className="text-2xs text-warning">{info.label} requires TLS — set it to Require or stronger.</span>
                )}
              </>
            )}

            {tabSel === "ssh" && hasNet && (
              <>
                <Switch
                  checked={!!ssh}
                  onChange={(v) => set({ ssh: v ? { host: "", port: null, user: "", auth: "key", keyPath: "", hostKey: null } : null })}
                  label="Connect through an SSH tunnel"
                />
                {ssh && (
                  <>
                    <div className="grid grid-cols-[1fr_72px_1fr] gap-2">
                      <Field label="SSH host">
                        <Input value={ssh.host} placeholder="bastion.example.com" onChange={(e) => setSsh({ host: e.target.value, hostKey: null })} />
                      </Field>
                      <Field label="Port">
                        <Input value={ssh.port ?? ""} placeholder="22" inputMode="numeric" onChange={(e) => setSsh({ port: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })} />
                      </Field>
                      <Field label="SSH user">
                        <Input value={ssh.user} placeholder="ubuntu" onChange={(e) => setSsh({ user: e.target.value })} />
                      </Field>
                    </div>
                    <Segmented
                      value={ssh.auth}
                      onChange={(auth) => setSsh({ auth })}
                      options={[
                        { value: "key", label: "Private key" },
                        { value: "agent", label: "SSH agent" },
                        { value: "password", label: "Password" },
                      ]}
                    />
                    {ssh.auth === "agent" ? (
                      <Field label="Agent socket" hint="Empty = system agent. 1Password: ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock">
                        <Input value={ssh.agentSocket ?? ""} placeholder="system default" onChange={(e) => setSsh({ agentSocket: e.target.value || null })} />
                      </Field>
                    ) : (
                      <div className={cx("grid gap-2", ssh.auth === "key" ? "grid-cols-2" : "grid-cols-1")}>
                        {ssh.auth === "key" && (
                          <Field label="Key file">
                            <Input value={ssh.keyPath ?? ""} placeholder="~/.ssh/id_ed25519" onChange={(e) => setSsh({ keyPath: e.target.value })} />
                          </Field>
                        )}
                        <Field label={ssh.auth === "key" ? "Passphrase" : "SSH password"}>
                          <Input type="password" value={sshSecret} placeholder={isNew ? (ssh.auth === "key" ? "if encrypted" : "") : "•••••• (unchanged)"} onChange={(e) => setSshSecret(e.target.value)} />
                        </Field>
                      </div>
                    )}
                    {ssh.hostKey && (
                      <div className="flex items-center gap-1.5 text-2xs text-muted">
                        <ShieldCheck size={12} className="shrink-0 text-success" />
                        <span className="truncate font-mono">{ssh.hostKey}</span>
                        <button type="button" className="ml-auto shrink-0 text-subtle hover:text-fg" onClick={() => setSsh({ hostKey: null })}>
                          Forget
                        </button>
                      </div>
                    )}
                  </>
                )}
              </>
            )}

            {tabSel === "safety" && (
              <div className="flex flex-col gap-2.5">
                <Switch
                  checked={c.readOnly}
                  onChange={(v) => set({ readOnly: v })}
                  label={c.env === "production" && !c.readOnly ? "Read-only — OFF: writes to production are allowed" : "Read-only — block writes from the app"}
                />
                <Switch checked={c.mcpEnabled} onChange={(v) => set({ mcpEnabled: v, ...(v ? {} : { mcpWrites: false }) })} label="Expose to AI assistants via MCP (reads)" />
                {c.mcpEnabled && !c.readOnly && (
                  <Switch checked={!!c.mcpWrites} onChange={(v) => set({ mcpWrites: v })} label="Allow AI to request writes — you approve each one" />
                )}
                <p className="text-2xs text-subtle">Passwords and SSH secrets are stored in the system keychain, never in plain text.</p>
              </div>
            )}
          </div>
          <button type="submit" hidden />
        </form>
      </div>
    </Modal>
  );
}

const KIND_GROUPS: { label: string; kinds: DbKind[] }[] = [
  { label: "Relational", kinds: ["postgres", "mysql", "mariadb", "mssql", "oracle", "sqlite", "duckdb"] },
  { label: "Cloud", kinds: ["neon", "supabase", "turso", "d1", "upstash", "cockroach", "redshift", "tidb", "singlestore", "greenplum"] },
  { label: "NoSQL & analytics", kinds: ["mongodb", "redis", "clickhouse", "bigquery", "snowflake", "elasticsearch", "opensearch", "cassandra", "scylla"] },
];

/** Empty host / port / user fall back to the placeholders' defaults. */
export function withDefaults(c: ConnectionConfig): ConnectionConfig {
  const k = kindInfo(c.kind);
  if (k.file || k.api) return c;
  const ssh = c.ssh && c.ssh.auth === "key" && !c.ssh.keyPath?.trim() ? { ...c.ssh, keyPath: "~/.ssh/id_ed25519" } : c.ssh;
  return { ...c, ssh, host: c.host?.trim() || "localhost", port: c.port ?? k.port, user: c.user?.trim() || k.user };
}

function defaultName(c: ConnectionConfig) {
  if (kindInfo(c.kind).file) return c.file?.split(/[\\/]/).pop() || kindInfo(c.kind).label;
  if (c.kind === "turso") return c.host?.replace(/^\w+:\/\//, "").split(".")[0] || "Turso";
  if (c.kind === "d1") return `D1 · ${c.database?.slice(0, 8) || "database"}`;
  if (c.kind === "snowflake") return `Snowflake · ${c.database || c.host || "account"}`;
  if (c.kind === "bigquery") return `BigQuery · ${c.database || c.file?.split(/[\\/]/).pop()?.replace(/\.json$/, "") || "project"}`;
  return `${c.database || kindInfo(c.kind).label}@${c.host || "localhost"}`;
}

function TestStatus({ state, msg }: { state: string; msg?: string }) {
  if (state === "idle") return null;
  return (
    <span
      title={msg}
      className={cx(
        "flex max-w-56 items-center gap-1 truncate text-2xs",
        state === "ok" && "text-success",
        state === "err" && "text-danger",
        state === "busy" && "text-muted",
      )}
    >
      {state === "busy" && <Loader2 size={12} className="animate-spin" />}
      {state === "ok" && <CheckCircle2 size={12} />}
      {state === "err" && <XCircle size={12} />}
      <span className="truncate">{state === "busy" ? "Testing…" : msg}</span>
    </span>
  );
}
