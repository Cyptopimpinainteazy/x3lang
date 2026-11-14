import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import QRCode from "qrcode";
import { api, download, errorsOf, randomHex } from "../api";
import { exportGlb, renderCard } from "../card";
import { CreatureView, type CreatureHandle } from "../creature/Creature";
import { genome, implantStateOf } from "../creature/genome";
import { agentKey, exportAgentJwk, operatorKey, sign } from "../keys";
import type { AgentRecord, ConnectionRec, ForgeErr } from "../schema";
import { useForge } from "../store";
import { Badge, Button, Errors, Field, Mono, Panel, StatBars, inputCls, short } from "../ui";
import { CapabilityTable } from "./AlienScreen";
import { SourceView } from "./CompileChamber";
import { confirmedLabel, statusChip } from "./CommsRig";
import { useCommand } from "./Command";
import { ProbeList } from "./ProvingGround";

export function Profile({ agentId }: { agentId: string }) {
  const { registry } = useForge();
  const { isCommander, refresh } = useCommand();
  const [a, setA] = useState<AgentRecord | null>(null);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const [qr, setQr] = useState<string>("");
  const [hasAgentKey, setHasAgentKey] = useState(false);
  const handle = useRef<CreatureHandle>({ canvas: null, group: null });
  const url = `${window.location.origin}/agents/${agentId}`;

  const load = useCallback(() => {
    api.get<AgentRecord>(`/api/agents/${agentId}`).then(setA).catch((e) => setErrs(errorsOf(e)));
  }, [agentId]);
  useEffect(() => {
    load();
    refresh().catch(() => undefined);
    agentKey(agentId).then((k) => setHasAgentKey(!!k)).catch(() => undefined);
    QRCode.toDataURL(url, { margin: 1, width: 180, color: { dark: "#05070b", light: "#e8f6ff" } }).then(setQr);
  }, [agentId, load, refresh, url]);

  const g = useMemo(() => {
    if (!a) return null;
    const states: Record<string, ReturnType<typeof implantStateOf>> = {};
    for (const c of a.connections) states[c.connector] = implantStateOf(c.status, c.health?.status ?? null);
    return genome({ traits: a.compile.traits, cosmetics: a.cosmetics, implantStates: states, revoked: a.status !== "active" });
  }, [a]);

  if (errs.length && !a) return <div className="mx-auto max-w-3xl p-6"><Errors errors={errs} /></div>;
  if (!a || !g) return <div className="p-8 text-center font-display text-xs text-glow">Reading the runtime…</div>;
  const dead = a.status !== "active";
  const connectorName = (id: string) => registry?.connectors.find((c) => c.id === id)?.name ?? id;

  const card = async () => download(`${a.agent_id}-card.png`, await renderCard(a, handle.current.canvas, url), "image/png");
  const glb = async () => {
    if (handle.current.group) download(`${a.agent_id}.glb`, await exportGlb(handle.current.group), "model/gltf-binary");
  };
  const manifest = async () => {
    const out = await api.get<{ manifest: unknown; source: string | null }>(`/api/agents/${a.agent_id}/export`);
    download(`${a.agent_id}.manifest.json`, JSON.stringify(out.manifest, null, 2));
  };

  return (
    <div className={`mx-auto max-w-[1400px] space-y-4 p-4`}>
      {dead && (
        <div className="danger-edge rounded-lg bg-danger/5 px-4 py-3 text-sm text-red-200">
          <span className="font-display text-xs text-danger">Scrapyard</span> · this agent is {a.status}. The Agent Law runtime refuses its actions and the
          Forge mesh refuses its messages. Evidence below is preserved: mandate, Proving Ground results, genesis signature and the runtime's hash-chained events.
        </div>
      )}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_420px]">
        <Panel title={a.name} right={<div className="flex items-center gap-2">
          <Badge tone={dead ? "danger" : "acid"}>{a.status}</Badge>
          <Badge>{(a.compile.normalized as { archetype: string }).archetype}</Badge>
        </div>}>
          <div className={`h-[420px] overflow-hidden rounded border border-edge ${dead ? "scrapyard" : ""}`}>
            <CreatureView genome={g} handle={handle} className="h-full w-full" />
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button onClick={card}>card.png</Button>
            <Button onClick={glb}>creature.glb</Button>
            <Button onClick={manifest}>manifest.json</Button>
            {a.compile.source && <Button onClick={() => download("agent.x3", a.compile.source ?? "", "text/plain")}>agent.x3</Button>}
            {a.compile.create_mandate && <Button onClick={() => download("create_mandate.json", JSON.stringify(a.compile.create_mandate, null, 2))}>create_mandate.json</Button>}
            {hasAgentKey && <Button tone="ghost" title="The agent's private key, for its own runtime. Keep it secret." onClick={async () => {
              const k = await agentKey(a.agent_id);
              if (k) download(`${a.agent_id}.agent-key.jwk.json`, JSON.stringify(await exportAgentJwk(k), null, 2));
            }}>agent key</Button>}
          </div>
        </Panel>
        <div className="space-y-4">
          <Panel title="Identity">
            <div className="flex gap-4">
              {qr && <img src={qr} alt="QR code for this profile page" className="h-32 w-32 rounded" />}
              <div className="min-w-0 space-y-1 text-xs text-slate-400">
                <div>agent <Mono className="text-slate-200">{a.agent_id}</Mono></div>
                <div>born <span className="text-slate-200">{new Date(a.born_at * 1000).toLocaleString()}</span></div>
                <div>build <Mono className="text-slate-300">{short(a.compile.build_hash, 12)}</Mono></div>
                <div>agent key <Mono className="text-slate-300">{short(a.agent_public_key, 10)}</Mono></div>
                <div>runtime budget left <span className="text-slate-200">{a.runtime.remaining}</span></div>
                <div>expires <span className="text-slate-200">{a.compile.compiled?.expires ?? new Date(a.compile.manifest.expires_at * 1000).toISOString()}</span></div>
                <a href={url} className="block truncate text-glow underline">{url}</a>
              </div>
            </div>
          </Panel>
          <Panel title="Stats"><StatBars stats={a.compile.stats} dim={dead} /></Panel>
        </div>
      </div>

      <Panel title="Connections">
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
          {a.connections.map((c) => <ConnectionCard key={c.id} c={c} name={connectorName(c.connector)} agent={a} canRevoke={isCommander && !dead} onDone={load} />)}
        </div>
      </Panel>

      <Panel title="Capability table"><CapabilityTable rows={a.compile.capabilities} /></Panel>

      {a.compile.source && (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          <Panel title="Mandate (.x3)"><SourceView source={a.compile.source} lineMap={a.compile.line_map} /></Panel>
          <Panel title={dead ? "Stop it on chain: revoke_mandate" : "Put it on chain: create_mandate"}>
            <pre className="code rounded border border-edge bg-void/80 p-3 font-mono text-slate-300">{JSON.stringify(dead ? a.revoke_mandate : a.compile.create_mandate, null, 2)}</pre>
            <p className="mt-2 text-xs text-gold">Sent by the owner account to x3-guard. The Forge never signs NEAR transactions.</p>
          </Panel>
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        <Panel title={`Proving Ground at birth · ${a.proof.probes.filter((p) => p.ok).length}/${a.proof.probes.length}`}><ProbeList probes={a.proof.probes} /></Panel>
        <div className="space-y-4">
          {!dead && hasAgentKey && <MeshConsole a={a} onDone={load} />}
          {isCommander && !dead && <RevokeAgent a={a} onDone={load} />}
          <Panel title="Runtime events (hash-chained)">
            <ol className="space-y-2 text-xs">
              {a.events.map((e) => (
                <li key={e.seq} className="rounded border border-edge p-2">
                  <div className="flex justify-between"><span className={e.kind === "revoked" ? "text-danger" : "text-acid"}>{e.kind}</span><span className="text-slate-500">#{e.seq} · {new Date(e.at * 1000).toLocaleString()}</span></div>
                  <Mono className="text-slate-500">{short(e.hash, 16)}</Mono>
                  {typeof e.details.reason === "string" && <div className="text-slate-300">reason: {e.details.reason}</div>}
                </li>
              ))}
            </ol>
          </Panel>
          {a.mesh_log.length > 0 && (
            <Panel title="Mesh log">
              <ul className="space-y-1 text-xs">
                {[...a.mesh_log].reverse().slice(0, 20).map((m) => (
                  <li key={m.nonce} className="flex flex-wrap gap-2"><Badge tone={m.verdict === "delivered" ? "acid" : "danger"}>{m.verdict}</Badge>
                    <span className="text-slate-300">{m.type} → {m.to}</span>{m.reasons.map((r) => <Badge key={r} tone="danger">{r}</Badge>)}</li>
                ))}
              </ul>
            </Panel>
          )}
        </div>
      </div>
    </div>
  );
}

function ConnectionCard({ c, name, agent, canRevoke, onDone }: { c: ConnectionRec; name: string; agent: AgentRecord; canRevoke: boolean; onDone: () => void }) {
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const revoke = async () => {
    setErrs([]);
    try {
      const op = await operatorKey();
      if (!op) throw new Error("No High Command key in this browser.");
      const nonce = randomHex(16);
      const reason = "revoked from profile page";
      const { payload_hex } = await api.post<{ payload_hex: string }>(`/api/agents/${agent.agent_id}/revoke/payload`, { reason, nonce, connection_id: c.id });
      await api.post(`/api/agents/${agent.agent_id}/connections/${c.id}/revoke`, { reason, nonce, signature: await sign(op, payload_hex) });
      onDone();
    } catch (e) {
      setErrs(errorsOf(e));
    }
  };
  return (
    <div className={`rounded-lg border p-3 ${c.status === "revoked" ? "border-danger/50" : "border-edge"}`}>
      <div className="flex items-center justify-between"><span className="text-sm font-semibold text-slate-100">{name}</span>{statusChip(c)}</div>
      <div className="mt-1 text-xs text-slate-400">{confirmedLabel(c)}</div>
      {c.health && <div className="text-xs text-slate-500">health: {c.health.detail}</div>}
      <div className="mt-2 flex items-center gap-3">
        {c.setup_url && <a href={c.setup_url} target="_blank" rel="noopener noreferrer" className="text-xs text-glow underline">open {name} ↗</a>}
        {c.connector === "forge_mesh" && <span className="text-xs text-slate-500">peers: {((c.config.peers as string[]) ?? []).join(", ") || "none"}</span>}
        {canRevoke && c.status !== "revoked" && <button onClick={revoke} className="ml-auto text-[10px] tracking-widest text-danger uppercase">revoke</button>}
      </div>
      <Errors errors={errs} />
    </div>
  );
}

function RevokeAgent({ a, onDone }: { a: AgentRecord; onDone: () => void }) {
  const [reason, setReason] = useState("");
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    setErrs([]);
    try {
      const op = await operatorKey();
      if (!op) throw new Error("No High Command key in this browser.");
      const nonce = randomHex(16);
      const { payload_hex } = await api.post<{ payload_hex: string }>(`/api/agents/${a.agent_id}/revoke/payload`, { reason, nonce });
      await api.post(`/api/agents/${a.agent_id}/revoke`, { reason, nonce, signature: await sign(op, payload_hex) });
      onDone();
    } catch (e) {
      setErrs(errorsOf(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Panel title="Revoke agent">
      <p className="mb-2 text-xs text-slate-400">Signed by your High Command key. The Agent Law runtime marks the identity revoked, so later actions are refused.
        It does not undo effects already authorized, and it does not touch x3-guard: send the <code>revoke_mandate</code> call shown afterwards.</p>
      <div className="flex items-end gap-2">
        <div className="flex-1"><Field label="Reason"><input className={inputCls} value={reason} maxLength={128} onChange={(e) => setReason(e.target.value)} /></Field></div>
        <Button tone="danger" onClick={go} disabled={busy || !reason.trim()}>Revoke</Button>
      </div>
      <Errors errors={errs} />
    </Panel>
  );
}

function MeshConsole({ a, onDone }: { a: AgentRecord; onDone: () => void }) {
  const mesh = a.connections.find((c) => c.connector === "forge_mesh");
  const peers = (mesh?.config.peers as string[] | undefined) ?? [];
  const kinds = (mesh?.config.messages as string[] | undefined) ?? [];
  const [to, setTo] = useState(peers[0] ?? "");
  const [kind, setKind] = useState(kinds[0] ?? "status");
  const [body, setBody] = useState("status: nominal");
  const [last, setLast] = useState<{ verdict: string; reasons: string[] } | null>(null);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  if (!mesh) return null;
  const send = async () => {
    setErrs([]);
    try {
      const key = await agentKey(a.agent_id);
      if (!key) throw new Error("This browser does not hold the agent key.");
      const nonce = randomHex(16);
      const expires_at = Math.floor(Date.now() / 1000) + 60;
      const msg = { from: a.agent_id, to, type: kind, body, nonce, expires_at };
      const { payload_hex } = await api.post<{ payload_hex: string }>("/api/mesh/payload", msg);
      setLast(await api.post("/api/mesh/send", { ...msg, signature: await sign(key, payload_hex) }));
      onDone();
    } catch (e) {
      setErrs(errorsOf(e));
    }
  };
  return (
    <Panel title="Mesh: send as this agent">
      <p className="mb-2 text-xs text-slate-400">Signed with the agent's own key (held in this browser). The Forge delivers it only if the agent is active, holds
        <code> agents.message</code> and the mesh surface, the connection is live, and the peer and message type are on the connection's allowlist.</p>
      <div className="grid grid-cols-1 gap-2 md:grid-cols-3">
        <Field label="To"><input className={inputCls} value={to} onChange={(e) => setTo(e.target.value.trim())} /></Field>
        <Field label="Type"><input className={inputCls} value={kind} onChange={(e) => setKind(e.target.value.trim())} /></Field>
        <Field label="Body"><input className={inputCls} value={body} onChange={(e) => setBody(e.target.value)} /></Field>
      </div>
      <div className="mt-2 flex items-center gap-2">
        <Button onClick={send}>Send</Button>
        {last && <Badge tone={last.verdict === "delivered" ? "acid" : "danger"}>{last.verdict}</Badge>}
        {last?.reasons.map((r) => <Badge key={r} tone="danger">{r}</Badge>)}
      </div>
      <Errors errors={errs} />
    </Panel>
  );
}
