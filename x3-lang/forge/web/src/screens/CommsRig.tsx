import { useState } from "react";
import { api, errorsOf } from "../api";
import type { ConfigField, ConnectionRec, Connector, ForgeErr } from "../schema";
import { errorsFor, skillMap, useForge } from "../store";
import { Badge, Button, Errors, Field, Panel, RiskPips, inputCls } from "../ui";

const TYPE_LABEL: Record<Connector["type"], string> = { compute: "Compute (GPU mesh)", ide: "IDE", agent: "Agents", pay: "Pay", wallet: "Wallet / account", data: "Data / tools" };
const IMPLANT_LABEL: Record<Connector["implant"], string> = {
  mind_link: "mind link",
  neural_jack: "neural jack", resonance: "resonance nodes", vault_gem: "vault gem", chain_sigil: "chain sigil", sensor_port: "sensor port",
};
// Setup states are one-time and session-bound; they live only in memory.
const STATES: Record<string, string> = {};

export function statusChip(rec?: ConnectionRec) {
  if (!rec) return <Badge>not plugged in</Badge>;
  if (rec.status === "revoked") return <Badge tone="danger">revoked</Badge>;
  if (rec.status === "failed") return <Badge tone="danger">failed</Badge>;
  if (rec.status === "pending") return <Badge tone="gold">needs setup</Badge>;
  const h = rec.health?.status;
  if (h === "ok") return <Badge tone="acid">connected</Badge>;
  if (h === "login_gated") return <Badge tone="acid" title={rec.health?.detail}>connected · login-gated</Badge>;
  return <Badge tone="glow">linked · unchecked</Badge>;
}

export function confirmedLabel(rec: ConnectionRec) {
  return rec.confirmed_by === "page" ? "setup page returned with a matching state"
    : rec.confirmed_by === "owner" ? "owner confirmed setup (page has no return callback yet)"
    : rec.confirmed_by === "owner_key" ? "owner stored its key on the Forge host"
    : rec.confirmed_by === "forge" ? "this Forge" : "not confirmed";
}

export function CommsRig() {
  const { registry, build, errors } = useForge();
  if (!registry) return null;
  const groups = (Object.keys(TYPE_LABEL) as Connector["type"][]).map((t) => [t, registry.connectors.filter((c) => c.type === t)] as const);
  return (
    <div className="space-y-4">
      <Panel title="Comms Rig">
        <p className="text-sm text-slate-300">
          Plug the agent into what it works with. A connection is only a channel: what the agent may do through it is set by
          its skills, and spending is capped by its mandate. Each connector's page opens in a new tab and should send you back here;
          the Forge only links it if the one-time state it handed out comes back for this session.
        </p>
        <div className="mt-3"><Errors errors={errorsFor(errors, "comms")} /></div>
      </Panel>
      {groups.map(([t, list]) => (
        <Panel key={t} title={TYPE_LABEL[t]}>
          <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
            {list.map((c) => <ConnectorCard key={c.id} c={c} plugged={build.connections.find((x) => x.connector === c.id)} />)}
          </div>
        </Panel>
      ))}
    </div>
  );
}

function ConnectorCard({ c, plugged }: { c: Connector; plugged?: { connector: string; connection_id?: string } }) {
  const { registry, build, setBuild, connections, setConnection, errors } = useForge();
  const skills = skillMap(registry);
  const rec = plugged?.connection_id ? connections[plugged.connection_id] : undefined;
  const [config, setConfig] = useState<Record<string, unknown>>({});
  const [secret, setSecret] = useState("");
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const [busy, setBusy] = useState(false);
  const [openUrl, setOpenUrl] = useState<string | null>(null);
  const missing = c.requires_skills.filter((s) => !build.skills[s]);
  const nodeErrs = errorsFor(errors, `connector.${c.id}`);

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setErrs([]);
    try {
      await f();
    } catch (e) {
      setErrs(errorsOf(e));
    } finally {
      setBusy(false);
    }
  };

  const start = () => run(async () => {
    const out = await api.post<{ connection: ConnectionRec; open_url: string | null; state: string | null }>(
      "/api/connect/start", { connector: c.id, config, agent: build.identity.agent || null });
    setConnection(out.connection);
    if (out.state) STATES[out.connection.id] = out.state;
    setOpenUrl(out.open_url);
    setBuild((b) => ({ ...b, connections: [...b.connections.filter((x) => x.connector !== c.id), { connector: c.id, connection_id: out.connection.id }] }));
    if (c.setup_url === "forge:self") {
      setConnection(await api.post<ConnectionRec>("/api/connect/health", { connection: out.connection.id }));
    }
  });
  const confirm = () => run(async () => {
    if (!rec) return;
    const state = STATES[rec.id];
    if (!state) throw new Error("This setup state was lost (page reloaded). Start setup again.");
    setConnection(await api.post<ConnectionRec>("/api/connect/confirm", { connection: rec.id, state }));
    delete STATES[rec.id];
  });
  const health = () => run(async () => {
    if (rec) setConnection(await api.post<ConnectionRec>("/api/connect/health", { connection: rec.id }));
  });
  const storeSecret = () => run(async () => {
    if (!rec) return;
    setConnection(await api.post<ConnectionRec>("/api/connect/secret", { connection: rec.id, secret }));
    setSecret("");
  });
  const unplug = () => setBuild((b) => ({ ...b, connections: b.connections.filter((x) => x.connector !== c.id) }));

  return (
    <div className={`rounded-lg border p-3 ${nodeErrs.length ? "danger-edge border-transparent" : plugged ? "glow-edge border-transparent" : "border-edge"} ${missing.length && !plugged ? "opacity-55" : ""}`}>
      <div className="flex items-start justify-between gap-2">
        <div>
          <div className="text-sm font-semibold text-slate-100">{c.name}</div>
          <div className="mt-0.5 flex flex-wrap items-center gap-2 text-[10px] text-slate-500">
            <RiskPips risk={c.risk} /><span>surface {c.surface}</span><span>· implant: {IMPLANT_LABEL[c.implant]}</span><span>· auth: {c.auth.replace("_", " ")}</span>
          </div>
        </div>
        {statusChip(rec)}
      </div>
      <p className="mt-2 text-xs text-slate-400">{c.description}</p>
      {missing.length > 0 && !plugged && <p className="mt-2 text-xs text-gold">Needs the {missing.map((m) => skills[m]?.name ?? m).join(", ")} skill.</p>}

      {!plugged && missing.length === 0 && (
        <div className="mt-3 space-y-2">
          {c.config.map((f) => <ConfigInput key={f.id} f={f} value={config[f.id]} onChange={(v) => setConfig((x) => ({ ...x, [f.id]: v }))} />)}
          <Button onClick={start} disabled={busy}>{c.setup_url === "forge:self" ? "Plug in" : "Plug in & start setup"}</Button>
        </div>
      )}

      {plugged && rec && (
        <div className="mt-3 space-y-2 text-xs">
          {rec.status === "pending" && (
            <div className="flex flex-wrap items-center gap-2">
              {(openUrl || rec.setup_url) && c.setup_url !== null && (
                <a className="rounded border border-glow/60 px-3 py-1.5 text-xs font-semibold tracking-wider text-glow uppercase hover:bg-glow/10"
                   href={openUrl ?? rec.setup_url ?? "#"} target="_blank" rel="noopener noreferrer">Open setup page ↗</a>
              )}
              {c.setup_url !== null && <Button tone="ghost" onClick={confirm} disabled={busy} title="Use when the page has no return callback yet">I finished setup</Button>}
            </div>
          )}
          {c.auth === "api_key" && rec.status !== "revoked" && (
            <div className="flex items-end gap-2">
              <div className="flex-1"><Field label={rec.has_secret ? "Key stored server-side · replace" : "API key (kept on the Forge host only)"}>
                <input className={inputCls} type="password" autoComplete="off" value={secret} onChange={(e) => setSecret(e.target.value)} />
              </Field></div>
              <Button onClick={storeSecret} disabled={busy || secret.length < 8}>Store</Button>
            </div>
          )}
          {rec.status !== "pending" && <div className="text-slate-400">Linked: {confirmedLabel(rec)}.</div>}
          {rec.health && <div className={rec.health.status === "failed" ? "text-danger" : "text-slate-400"}>Health: {rec.health.detail}</div>}
          <div className="flex flex-wrap gap-2">
            {rec.status !== "pending" && rec.status !== "revoked" && <Button tone="ghost" onClick={health} disabled={busy}>Check health</Button>}
            {rec.setup_url && <a href={rec.setup_url} target="_blank" rel="noopener noreferrer" className="self-center text-[11px] text-glow underline">open page</a>}
            <button onClick={unplug} className="ml-auto text-[10px] tracking-widest text-danger uppercase">unplug</button>
          </div>
        </div>
      )}
      {plugged && !rec && <div className="mt-3 flex gap-2 text-xs text-slate-400">Loading connection… <button onClick={unplug} className="text-danger">unplug</button></div>}
      <div className="mt-2"><Errors errors={[...errs, ...nodeErrs]} /></div>
    </div>
  );
}

function ConfigInput({ f, value, onChange }: { f: ConfigField; value: unknown; onChange: (v: unknown) => void }) {
  if (f.type === "choice_list") {
    const list = (Array.isArray(value) ? value : []) as string[];
    return (
      <Field label={f.label} group>
        <div className="flex flex-wrap gap-2">
          {f.choices?.map((ch) => (
            <button key={ch} onClick={() => onChange(list.includes(ch) ? list.filter((x) => x !== ch) : [...list, ch])}
                    className={`rounded border px-2 py-1 text-xs ${list.includes(ch) ? "border-glow text-glow" : "border-edge text-slate-400"}`}>{ch}</button>
          ))}
        </div>
      </Field>
    );
  }
  if (f.type === "text_list") {
    return <Field label={`${f.label} (comma separated)`}><input className={inputCls} onChange={(e) => onChange(e.target.value.split(/[\s,]+/).filter(Boolean))} /></Field>;
  }
  if (f.type === "int") {
    return <Field label={f.label}><input className={inputCls} type="number" min={f.min} max={f.max} onChange={(e) => onChange(Number.parseInt(e.target.value || "0", 10))} /></Field>;
  }
  return <Field label={f.label}><input className={inputCls} placeholder={f.type === "url" ? (f.local ? "http://localhost:…" : f.allow_local ? "http://127.0.0.1:8790 or https://gpu.x3star.net" : "https://…") : "account.near"} onChange={(e) => onChange(e.target.value.trim())} /></Field>;
}
