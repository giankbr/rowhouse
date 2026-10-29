import { CheckCircle2, Loader2, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, ENVIRONMENTS, kindInfo, KINDS, newConnection, type ConnectionConfig, type DbKind, type Environment } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Field, Input, Modal, Segmented, Switch, TAG_COLORS, cx } from "./ui";

export function ConnectionDialog({
  open,
  initial,
  onClose,
  onSaved,
  onDeleted,
}: {
  open: boolean;
  initial: ConnectionConfig | null;
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
      setC(initial ?? newConnection());
      setPassword("");
      setSshSecret("");
      setTest({ state: "idle" });
    }
  }, [open, initial]);

  const set = (patch: Partial<ConnectionConfig>) => setC((x) => ({ ...x, ...patch }));
  const setKind = (kind: DbKind) => {
    const d = newConnection(kind);
    set({ kind, host: d.host, port: d.port, user: d.user, ssh: d.host ? c.ssh : null });
  };
  const info = kindInfo(c.kind);
  // Empty password field on an existing profile = keep the stored one.
  const pw = password === "" && !isNew ? undefined : password;
  const sshPw = sshSecret === "" && !isNew ? undefined : sshSecret;
  const ssh = c.ssh;
  const setSsh = (patch: Partial<NonNullable<ConnectionConfig["ssh"]>>) =>
    setC((x) => ({ ...x, ssh: x.ssh ? { ...x.ssh, ...patch } : null }));

  const runTest = async () => {
    setTest({ state: "busy" });
    try {
      const fp = await api.testConnection(c, pw, sshPw);
      if (fp && c.ssh && !c.ssh.hostKey) setSsh({ hostKey: fp }); // pin on save
      setTest({ state: "ok", msg: "Connection successful" });
    } catch (e) {
      setTest({ state: "err", msg: String(e) });
    }
  };

  const save = async () => {
    const saved = await api.saveConnection({ ...c, name: c.name.trim() || defaultName(c) }, pw, sshPw);
    onSaved(saved);
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={520}
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
      <form className="flex flex-col gap-3" onSubmit={(e) => (e.preventDefault(), save())}>
        <div className="grid grid-cols-6 gap-1">
          {KINDS.map((k) => (
            <button
              type="button"
              key={k.kind}
              onClick={() => setKind(k.kind)}
              className={cx(
                "relative flex flex-col items-center gap-1 rounded-lg px-1 pt-2 pb-1.5 text-[10.5px] leading-tight transition-colors",
                c.kind === k.kind ? "bg-accent-soft text-fg ring-1 ring-accent" : "text-muted hover:bg-hover hover:text-fg",
              )}
            >
              <KindBadge kind={k.kind} size={26} />
              <span className="w-full truncate text-center">{k.label}</span>
              {k.beta && <span className="absolute top-1 right-1 size-1.5 rounded-full bg-warning" title="Beta" />}
            </button>
          ))}
        </div>
        <div className="flex items-center justify-between">
          <span className="text-2xs text-subtle">
            {info.beta ? "Beta — not yet verified against a live server" : `${info.label} · default port ${info.port ?? "—"}`}
          </span>
          <div className="flex items-center gap-1">
            {Object.entries(TAG_COLORS).map(([name, hex]) => (
              <button
                type="button"
                key={name}
                title={name}
                onClick={() => set({ color: c.color === name ? null : name })}
                className={cx("size-3.5 rounded-full transition-transform hover:scale-110", c.color === name && "ring-2 ring-offset-1 ring-offset-elevated")}
                style={{ background: hex, ["--tw-ring-color" as string]: hex }}
              />
            ))}
          </div>
        </div>

        <Field label="Environment" hint={c.env === "production" ? "Production starts read-only, gets a red window frame, and asks you to type the table name before DROP / TRUNCATE / unfiltered DELETE." : undefined}>
          <Segmented
            value={(c.env ?? "none") as Environment}
            onChange={(env) => {
              const e = ENVIRONMENTS.find((x) => x.value === env)!;
              // production defaults to read-only; the user can still opt in to writes below
              set({ env, color: c.color ?? e.color, ...(env === "production" ? { readOnly: true } : {}) });
            }}
            options={ENVIRONMENTS.map((e) => ({ value: e.value, label: e.label }))}
          />
        </Field>

        <Field label="Name">
          <Input autoFocus value={c.name} placeholder={defaultName(c)} onChange={(e) => set({ name: e.target.value })} />
        </Field>

        {info.file ? (
          <Field label="Database file" hint={c.kind === "duckdb" ? "Path to a .duckdb file (created if missing)" : "Absolute path to an existing .db / .sqlite file"}>
            <Input
              value={c.file ?? ""}
              placeholder={c.kind === "duckdb" ? "/Users/you/data/warehouse.duckdb" : "/Users/you/data/app.db"}
              onChange={(e) => set({ file: e.target.value })}
            />
          </Field>
        ) : (
          <>
            <div className="grid grid-cols-[1fr_88px] gap-2">
              <Field label="Host">
                <Input value={c.host ?? ""} onChange={(e) => set({ host: e.target.value })} />
              </Field>
              <Field label="Port">
                <Input
                  value={c.port ?? ""}
                  inputMode="numeric"
                  onChange={(e) => set({ port: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })}
                />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <Field label="User">
                <Input value={c.user ?? ""} onChange={(e) => set({ user: e.target.value })} />
              </Field>
              <Field label="Password">
                <Input
                  type="password"
                  value={password}
                  placeholder={isNew ? "" : "•••••• (unchanged)"}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </Field>
            </div>
            <Field label="Database">
              <Input value={c.database ?? ""} placeholder="optional" onChange={(e) => set({ database: e.target.value || null })} />
            </Field>
          </>
        )}

        {!info.file && (
          <div className="flex flex-col gap-2.5 rounded-lg border border-line p-3">
            <Switch
              checked={!!ssh}
              onChange={(v) =>
                set({ ssh: v ? { host: "", port: 22, user: "", auth: "key", keyPath: "~/.ssh/id_ed25519", hostKey: null } : null })
              }
              label="Connect through SSH tunnel"
            />
            {ssh && (
              <>
                <div className="grid grid-cols-[1fr_72px_1fr] gap-2">
                  <Field label="SSH host">
                    <Input value={ssh.host} placeholder="bastion.example.com" onChange={(e) => setSsh({ host: e.target.value, hostKey: null })} />
                  </Field>
                  <Field label="Port">
                    <Input
                      value={ssh.port ?? ""}
                      inputMode="numeric"
                      onChange={(e) => setSsh({ port: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })}
                    />
                  </Field>
                  <Field label="SSH user">
                    <Input value={ssh.user} placeholder="ubuntu" onChange={(e) => setSsh({ user: e.target.value })} />
                  </Field>
                </div>
                <div className="flex items-center gap-3">
                  <Segmented
                    value={ssh.auth}
                    onChange={(auth) => setSsh({ auth })}
                    options={[
                      { value: "key", label: "Private key" },
                      { value: "agent", label: "SSH agent" },
                      { value: "password", label: "Password" },
                    ]}
                  />
                </div>
                {ssh.auth === "agent" ? (
                  <Field label="Agent socket" hint="Leave empty for the system agent ($SSH_AUTH_SOCK / Windows OpenSSH / Pageant). 1Password: ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock">
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
                    <Input
                      type="password"
                      value={sshSecret}
                      placeholder={isNew ? (ssh.auth === "key" ? "if encrypted" : "") : "•••••• (unchanged)"}
                      onChange={(e) => setSshSecret(e.target.value)}
                    />
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
          </div>
        )}

        <div className="flex flex-col gap-2 rounded-lg bg-panel p-3">
          <Switch
            checked={c.readOnly}
            onChange={(v) => set({ readOnly: v })}
            label={c.env === "production" && !c.readOnly ? "Read-only — OFF: writes to production are allowed" : "Read-only — block writes from the app"}
          />
          <Switch checked={c.mcpEnabled} onChange={(v) => set({ mcpEnabled: v })} label="Expose to AI assistants via MCP (read-only)" />
        </div>
        <p className="text-2xs text-subtle">Passwords and SSH secrets are stored in the system keychain, never in plain text.</p>
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

function defaultName(c: ConnectionConfig) {
  if (kindInfo(c.kind).file) return c.file?.split(/[\\/]/).pop() || kindInfo(c.kind).label;
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
