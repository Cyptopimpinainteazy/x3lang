import { useState } from "react";
import { api, errorsOf } from "../api";
import type { CompileResult, ForgeErr, Probe, Proof } from "../schema";
import { useForge } from "../store";
import { Badge, Button, Errors, Panel } from "../ui";

export function ProbeList({ probes }: { probes: Probe[] }) {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <ul className="space-y-2">
      {probes.map((p) => {
        const failed = p.checks.filter((c) => !c.ok);
        return (
          <li key={p.id} className={`rounded border p-3 ${p.ok ? "border-edge" : "danger-edge border-transparent"}`}>
            <button className="flex w-full items-center gap-3 text-left" onClick={() => setOpen(open === p.id ? null : p.id)}>
              <span className={`grid h-6 w-6 shrink-0 place-items-center rounded-full text-xs ${p.ok ? "bg-acid/15 text-acid" : "bg-danger/20 text-danger"}`}>{p.ok ? "✓" : "✕"}</span>
              <span className="flex-1">
                <span className="block text-sm text-slate-100">{p.title}</span>
                <span className="block text-[10px] text-slate-500">{p.engine}</span>
              </span>
              <Badge tone={p.outcome === "allow" ? "acid" : "danger"}>{p.outcome === "allow" ? (p.approvals_needed ? `allowed · needs ${p.approvals_needed} approval` : "allowed") : "blocked"}</Badge>
              <span className="text-[10px] text-slate-500">expected {p.expect === "approval" ? "approval" : p.expect === "allow" ? "allow" : "block"}</span>
            </button>
            {failed.length > 0 && <div className="mt-2 flex flex-wrap gap-1.5 pl-9">{failed.map((c) => <Badge key={c.guard} tone="danger">{c.guard}</Badge>)}</div>}
            {open === p.id && (
              <div className="mt-3 space-y-2 pl-9 text-xs">
                {p.note && <p className="text-gold">{p.note}</p>}
                <table className="w-full">
                  <tbody>
                    {p.checks.map((c, i) => (
                      <tr key={i} className="border-t border-edge"><td className={`py-1 pr-3 font-mono ${c.ok ? "text-acid" : "text-danger"}`}>{c.ok ? "pass" : "FAIL"}</td><td className="pr-3 font-mono text-slate-300">{c.guard}</td><td className="text-slate-400">{c.detail}</td></tr>
                    ))}
                  </tbody>
                </table>
                {p.intent_source && <pre className="code rounded border border-edge bg-void/80 p-2 font-mono text-slate-400">{p.intent_source}</pre>}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

export function ProvingGround() {
  const { build, proof, setProof, errors } = useForge();
  const [busy, setBusy] = useState(false);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const run = async () => {
    setBusy(true);
    setErrs([]);
    try {
      const out = await api.post<{ compile: CompileResult; proof: Proof }>("/api/prove", { build });
      setProof(out.proof);
    } catch (e) {
      setErrs(errorsOf(e));
      setProof(null);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="space-y-4">
      <Panel title="Proving Ground" right={<Button onClick={run} disabled={busy || errors.length > 0} tone="acid">{busy ? "Attacking…" : "Run attacks"}</Button>}>
        <p className="text-sm text-slate-300">
          Real agent intents, compiled by <code>near_intents.compile_program</code> and judged by the x3-guard checks
          (<code>mandate check</code>), plus delegation attempts judged by Agent Law admission. A permitted payment must pass; an unlisted recipient,
          an over-cap payment, a refund hijack, an expired, over-budget or revoked mandate, and every privilege escalation must be blocked. Birth is
          refused unless every probe matches.
        </p>
        <div className="mt-3"><Errors errors={errs} /></div>
      </Panel>
      {proof && (
        <Panel title={proof.passed ? "All probes held" : "Probes failed"} right={<Badge tone={proof.passed ? "acid" : "danger"}>{proof.probes.filter((p) => p.ok).length}/{proof.probes.length}</Badge>}>
          <ProbeList probes={proof.probes} />
        </Panel>
      )}
    </div>
  );
}
