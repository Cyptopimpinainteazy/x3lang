import { GLOW_NAMES, IDLE_NAMES, PALETTE_NAMES, PATTERN_NAMES } from "../creature/genome";
import type { CapabilityRow, Cosmetics } from "../schema";
import { skillMap, useForge } from "../store";
import { Badge, EnforcedBy, Panel, RiskPips, StatBars } from "../ui";

export function CapabilityTable({ rows }: { rows: CapabilityRow[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-xs">
        <thead className="text-[10px] tracking-widest text-slate-500 uppercase">
          <tr><th className="py-1.5 pr-3">From</th><th className="pr-3">Grants</th><th className="pr-3">Compiles to</th><th className="pr-3">Enforced by</th><th>Risk</th></tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.source} className="border-t border-edge align-top">
              <td className="py-2 pr-3 font-semibold text-slate-200">{r.name}</td>
              <td className="pr-3 text-slate-300">{r.grants}</td>
              <td className="pr-3 font-mono text-[11px] text-slate-400">{r.compiles_to}</td>
              <td className="pr-3"><EnforcedBy by={r.enforced_by} /></td>
              <td><RiskPips risk={r.risk} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Choice({ label, options, value, onPick }: { label: string; options: string[]; value?: string; onPick: (v: string) => void }) {
  return (
    <div>
      <div className="mb-1 text-[10px] tracking-widest text-slate-400 uppercase">{label}</div>
      <div className="flex flex-wrap gap-2">
        {options.map((o) => (
          <button key={o} onClick={() => onPick(o)} className={`rounded border px-2.5 py-1 text-xs capitalize ${value === o ? "border-acid text-acid" : "border-edge text-slate-400 hover:text-slate-200"}`}>{o}</button>
        ))}
      </div>
    </div>
  );
}

export function AlienScreen() {
  const { registry, build, setBuild, compile } = useForge();
  const skills = skillMap(registry);
  const setCos = (k: keyof Cosmetics, v: string) => setBuild((b) => ({ ...b, cosmetics: { ...b.cosmetics, [k]: v } }));
  return (
    <div className="space-y-4">
      <Panel title="Cosmetics" right={<Badge>never affects permissions</Badge>}>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Choice label="Skin palette" options={PALETTE_NAMES} value={build.cosmetics.palette} onPick={(v) => setCos("palette", v)} />
          <Choice label="Bioluminescence" options={GLOW_NAMES} value={build.cosmetics.glow} onPick={(v) => setCos("glow", v)} />
          <Choice label="Skin pattern" options={PATTERN_NAMES} value={build.cosmetics.pattern} onPick={(v) => setCos("pattern", v)} />
          <Choice label="Idle" options={IDLE_NAMES} value={build.cosmetics.idle} onPick={(v) => setCos("idle", v)} />
        </div>
        <p className="mt-3 text-xs text-slate-500">Body shape, parts and implants come only from the compiled build. Cosmetics are left out of the build hash, the mandate and the manifest.</p>
      </Panel>
      <Panel title="Anatomy: what grew from what">
        {!compile ? <p className="text-xs text-slate-500">Compile the build to grow the creature.</p> : (
          <ul className="grid grid-cols-1 gap-2 text-xs md:grid-cols-2">
            <li className="rounded border border-edge p-2"><span className="text-acid">Body · {compile.traits.body}</span> <span className="text-slate-400">from the species</span></li>
            {compile.traits.parts.map((p) => (
              <li key={p.part} className="rounded border border-edge p-2">
                <span className="text-acid capitalize">{p.part.replace("_", " ")}</span>{p.count ? ` ×${p.count}` : ""}{p.level ? ` · level ${p.level}` : ""}
                <span className="text-slate-400"> from {p.sources.map((s) => skills[s]?.name ?? s).join(", ")}</span>
              </li>
            ))}
            {compile.traits.implants.map((im) => (
              <li key={im.connector} className="rounded border border-edge p-2"><span className="text-glow">{im.implant.replace("_", " ")}</span>
                <span className="text-slate-400"> from {registry?.connectors.find((c) => c.id === im.connector)?.name}</span></li>
            ))}
            {compile.traits.risk_tier >= 3 && <li className="rounded border border-danger/40 p-2 text-danger">Red veins · risk tier {compile.traits.risk_tier}</li>}
          </ul>
        )}
      </Panel>
      <Panel title="Stats (derived from the compiled build)"><StatBars stats={compile?.stats ?? null} /></Panel>
      <Panel title="Capability table">{compile ? <CapabilityTable rows={compile.capabilities} /> : <p className="text-xs text-slate-500">Nothing compiled yet.</p>}</Panel>
    </div>
  );
}
