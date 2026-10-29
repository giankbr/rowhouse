import { CheckCircle2, Loader2, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, newConnection, type ConnectionConfig, type DbKind } from "../lib/api";
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
    set({ kind, host: d.host, port: d.port, user: d.user });
  };
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
      width={480}
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
        <div className="flex items-center justify-between">
          <Segmented
            value={c.kind}
            onChange={setKind}
            options={[
              { value: "postgres", label: "PostgreSQL" },
              { value: "mysql", label: "MySQL" },
              { value: "sqlite", label: "SQLite" },
            ]}
          />
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

        <Field label="Name">
          <Input autoFocus value={c.name} placeholder={defaultName(c)} onChange={(e) => set({ name: e.target.value })} />
        </Field>

        {c.kind === "sqlite" ? (
          <Field label="Database file" hint="Absolute path to an existing .db / .sqlite file">
            <Input value={c.file ?? ""} placeholder="/Users/you/data/app.db" onChange={(e) => set({ file: e.target.value })} />
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

        {c.kind !== "sqlite" && (
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
                      { value: "password", label: "Password" },
                    ]}
                  />
                </div>
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
          <Switch checked={c.readOnly} onChange={(v) => set({ readOnly: v })} label="Read-only — block writes from the app" />
          <Switch checked={c.mcpEnabled} onChange={(v) => set({ mcpEnabled: v })} label="Expose to AI assistants via MCP (read-only)" />
        </div>
        <p className="text-2xs text-subtle">Passwords and SSH secrets are stored in the system keychain, never in plain text.</p>
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

function defaultName(c: ConnectionConfig) {
  if (c.kind === "sqlite") return c.file?.split(/[\\/]/).pop() || "SQLite";
  return `${c.database || c.kind}@${c.host || "localhost"}`;
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
