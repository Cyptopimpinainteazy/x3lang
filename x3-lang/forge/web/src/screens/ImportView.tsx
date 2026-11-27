import { useState } from "react";
import { api, errorsOf } from "../api";
import { CreatureView } from "../creature/Creature";
import { genome } from "../creature/genome";
import type { Cosmetics, ForgeErr, Stats, Traits } from "../schema";
import { Badge, Errors, Panel, StatBars } from "../ui";

interface Report { ok: boolean; checks: { check: string; ok: boolean; detail: string }[]; traits?: Traits; stats?: Stats; cosmetics?: Cosmetics }

export function ImportView() {
  const [report, setReport] = useState<Report | null>(null);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  const onFile = async (f: File | undefined) => {
    setErrs([]);
    setReport(null);
    if (!f) return;
    try {
      const manifest = JSON.parse(await f.text());
      setReport(await api.post<Report>("/api/import", { manifest }));
    } catch (e) {
      setErrs(errorsOf(e));
    }
  };
  const g = report?.traits ? genome({ traits: report.traits, cosmetics: report.cosmetics ?? {} }) : null;
  return (
    <div className="mx-auto max-w-[1200px] space-y-4 p-4">
      <Panel title="Import a manifest">
        <p className="mb-3 text-sm text-slate-300">Load an exported <code>manifest.json</code>. The Forge rebuilds the character from the build alone and checks every hash:
          build hash, stats, creature seed, regenerated mandate source, the compiled terms' <code>program_sha256</code>, and the High Command signature over the runtime genesis payload.</p>
        <input type="file" accept="application/json,.json" onChange={(e) => onFile(e.target.files?.[0])} className="text-sm text-slate-300 file:mr-3 file:rounded file:border file:border-glow/60 file:bg-transparent file:px-3 file:py-1.5 file:text-xs file:text-glow" />
        <div className="mt-3"><Errors errors={errs} /></div>
      </Panel>
      {report && (
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_380px]">
          <Panel title="Verification" right={<Badge tone={report.ok ? "acid" : "danger"}>{report.ok ? "identical" : "mismatch"}</Badge>}>
            <ul className="space-y-1.5 text-xs">
              {report.checks.map((c) => (
                <li key={c.check} className="flex gap-2"><span className={c.ok ? "text-acid" : "text-danger"}>{c.ok ? "✓" : "✕"}</span>
                  <span className="w-44 shrink-0 font-mono text-slate-300">{c.check}</span><span className="break-all text-slate-400">{c.detail}</span></li>
              ))}
            </ul>
          </Panel>
          {g && <Panel title="Rebuilt creature"><div className="h-72 overflow-hidden rounded border border-edge"><CreatureView genome={g} className="h-full w-full" /></div>
            <div className="mt-3"><StatBars stats={report.stats ?? null} /></div></Panel>}
        </div>
      )}
    </div>
  );
}
