import { useState } from "react";
import { api, errorsOf } from "../api";
import { navigate } from "../App";
import { newAgentKey, operatorKey, publicHex, saveAgentKey, sign } from "../keys";
import type { AgentRecord, CompileResult, ForgeErr, Proof } from "../schema";
import { buildKey, useForge } from "../store";
import { Badge, Button, Errors, Mono, Panel, short } from "../ui";
import { ClaimPanel, CommandSummary, useCommand } from "./Command";
import { ProbeList } from "./ProvingGround";

interface Prepared { birth_id: string; payload_hex: string; compile: CompileResult; proof: Proof; expires_in: number; key: CryptoKeyPair; agent: string }

export function Birth() {
  const { build, compile, compileFor, errors, connections, setProof } = useForge();
  const { isCommander, status } = useCommand();
  const [prep, setPrep] = useState<Prepared | null>(null);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const [busy, setBusy] = useState(false);
  const fresh = compile && compileFor === buildKey(build) && errors.length === 0;
  const conns = build.connections.map((c) => ({ c, rec: c.connection_id ? connections[c.connection_id] : undefined }));
  const connOk = conns.length > 0 && conns.every(({ rec }) => rec && rec.status === "linked" && (rec.health?.status === "ok" || rec.health?.status === "login_gated"));
  const checks = [
    { ok: !!status?.claimed, label: "High Command claimed" },
    { ok: isCommander, label: "This browser holds the High Command key" },
    { ok: !!fresh, label: "Build compiles with no errors" },
    { ok: connOk, label: "Every connection is linked and its health check passed" },
  ];

  const prepare = async () => {
    setBusy(true);
    setErrs([]);
    try {
      const key = await newAgentKey();
      const out = await api.post<Omit<Prepared, "key" | "agent">>("/api/birth/prepare", { build, agent_public_key: await publicHex(key) });
      setProof(out.proof);
      setPrep({ ...out, key, agent: out.compile.manifest.agent_id });
    } catch (e) {
      setErrs(errorsOf(e));
    } finally {
      setBusy(false);
    }
  };
  const commit = async () => {
    if (!prep) return;
    setBusy(true);
    setErrs([]);
    try {
      const op = await operatorKey();
      if (!op) throw new Error("No High Command key in this browser.");
      const signature = await sign(op, prep.payload_hex);
      const rec = await api.post<AgentRecord>("/api/birth/commit", { birth_id: prep.birth_id, signature });
      await saveAgentKey(rec.agent_id, prep.key);
      navigate(`/agents/${rec.agent_id}`);
    } catch (e) {
      setErrs(errorsOf(e));
    } finally {
      setBusy(false);
    }
  };

  let payloadView: unknown = null;
  try {
    payloadView = prep ? JSON.parse(new TextDecoder().decode(new Uint8Array(prep.payload_hex.match(/../g)!.map((h) => parseInt(h, 16))))) : null;
  } catch {
    payloadView = null;
  }

  return (
    <div className="space-y-4">
      <ClaimPanel />
      <Panel title="Birth chamber" right={<CommandSummary />}>
        <ul className="space-y-1.5">
          {checks.map((c) => (
            <li key={c.label} className="flex items-center gap-2 text-sm">
              <span className={c.ok ? "text-acid" : "text-slate-600"}>{c.ok ? "◉" : "○"}</span>
              <span className={c.ok ? "text-slate-200" : "text-slate-500"}>{c.label}</span>
            </li>
          ))}
        </ul>
        {conns.some(({ rec }) => !rec || rec.status !== "linked") && (
          <p className="mt-2 text-xs text-gold">Finish setup and health checks in the Comms Rig: {conns.filter(({ rec }) => !rec || rec.status !== "linked").map(({ c }) => c.connector).join(", ")}</p>
        )}
        <div className="mt-4 flex gap-3">
          <Button onClick={prepare} disabled={busy || !checks.every((c) => c.ok)} tone="acid">{busy && !prep ? "Running Proving Ground…" : "Prepare birth"}</Button>
          {prep && <Button onClick={commit} disabled={busy} tone="glow">{busy ? "Signing…" : "Sign with High Command & birth"}</Button>}
        </div>
        <p className="mt-3 text-xs text-slate-500">
          Prepare: the host recompiles the build itself, reruns every Proving Ground probe, checks every connection, and returns the Agent Law
          runtime's exact <code>genesis_payload</code>. A new agent key is made in this browser; only its public half is sent. Sign: your High Command
          key signs that payload and the runtime records the birth. Its nonce carries the build hash, so the signature covers the mandate, skills and connections.
        </p>
        <div className="mt-3"><Errors errors={errs} /></div>
      </Panel>
      {prep && (
        <>
          <Panel title="What you are signing" right={<Badge tone="gold">expires in {Math.round(prep.expires_in / 60)} min</Badge>}>
            <div className="mb-2 grid grid-cols-1 gap-1 text-xs text-slate-400 md:grid-cols-2">
              <div>agent <Mono className="text-slate-200">{prep.agent}</Mono></div>
              <div>build hash <Mono className="text-slate-200">{short(prep.compile.build_hash, 12)}</Mono></div>
            </div>
            <pre className="code max-h-80 rounded border border-edge bg-void/80 p-3 font-mono text-slate-300">{JSON.stringify(payloadView, null, 2)}</pre>
          </Panel>
          <Panel title="Proving Ground (rerun by the host)"><ProbeList probes={prep.proof.probes} /></Panel>
        </>
      )}
    </div>
  );
}
