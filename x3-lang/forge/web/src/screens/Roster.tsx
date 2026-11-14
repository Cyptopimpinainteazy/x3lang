import { useEffect, useState } from "react";
import { api, errorsOf } from "../api";
import { navigate } from "../App";
import { genome } from "../creature/genome";
import type { ForgeErr, RosterEntry, Traits, Cosmetics } from "../schema";
import { Badge, Errors, Panel, StatBars, short } from "../ui";

/** A flat sigil of the creature (front view) so the roster needs no WebGL context per card. */
export function Sigil({ traits, cosmetics, revoked, size = 120 }: { traits: Traits; cosmetics: Cosmetics; revoked: boolean; size?: number }) {
  const g = genome({ traits, cosmetics, revoked });
  const xs = g.meshes.map((m) => m.pos[0]);
  const ys = g.meshes.map((m) => m.pos[1]);
  const minX = Math.min(...xs) - 0.6, maxX = Math.max(...xs) + 0.6, minY = Math.min(...ys) - 0.6, maxY = Math.max(...ys) + 0.6;
  const span = Math.max(maxX - minX, maxY - minY);
  const sorted = [...g.meshes].sort((a, b) => a.pos[2] - b.pos[2]);
  return (
    <svg viewBox={`${minX} ${-maxY} ${span} ${span}`} width={size} height={size} className={revoked ? "scrapyard" : ""}>
      {sorted.map((m, i) => {
        const color = g.palette[m.role];
        if (m.kind === "tube" && m.points) {
          return <polyline key={i} points={m.points.map((p) => `${p[0]},${-p[1]}`).join(" ")} fill="none" stroke={color} strokeWidth={(m.radius ?? 0.05) * 2} strokeLinecap="round" opacity={m.opacity ?? 1} />;
        }
        const rx = Math.abs(m.scale[0]) * (m.kind === "torus" ? 1 : 1), ry = Math.abs(m.scale[1]);
        if (m.kind === "torus" || m.wire) return <ellipse key={i} cx={m.pos[0]} cy={-m.pos[1]} rx={rx} ry={ry * 0.9} fill="none" stroke={color} strokeWidth={0.03} opacity={m.opacity ?? 1} />;
        return <ellipse key={i} cx={m.pos[0]} cy={-m.pos[1]} rx={rx} ry={ry} fill={color} opacity={m.opacity ?? 1} />;
      })}
    </svg>
  );
}

export function Roster() {
  const [agents, setAgents] = useState<RosterEntry[] | null>(null);
  const [errs, setErrs] = useState<ForgeErr[]>([]);
  useEffect(() => {
    api.get<{ agents: RosterEntry[] }>("/api/agents").then((r) => setAgents(r.agents)).catch((e) => setErrs(errorsOf(e)));
  }, []);
  const live = agents?.filter((a) => a.status === "active") ?? [];
  const dead = agents?.filter((a) => a.status !== "active") ?? [];
  const Card = ({ a }: { a: RosterEntry }) => (
    <button onClick={() => navigate(`/agents/${a.agent_id}`)} className={`flex gap-4 rounded-lg border p-3 text-left transition hover:border-glow/60 ${a.status === "active" ? "border-edge bg-panel/70" : "border-danger/40 bg-panel/40"}`}>
      <div className="grid h-[120px] w-[120px] shrink-0 place-items-center rounded bg-void/80"><Sigil traits={a.traits} cosmetics={a.cosmetics} revoked={a.status !== "active"} /></div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-2"><span className="truncate font-semibold text-slate-100">{a.name}</span><Badge tone={a.status === "active" ? "acid" : "danger"}>{a.status}</Badge></div>
        <div className="truncate text-[11px] text-slate-500">{a.agent_id} · {a.archetype} · {short(a.build_hash, 6)}</div>
        <div className="mt-2"><StatBars stats={a.stats} dim={a.status !== "active"} /></div>
      </div>
    </button>
  );
  return (
    <div className="mx-auto max-w-[1300px] space-y-4 p-4">
      <Errors errors={errs} />
      <Panel title={`Roster · ${live.length} active`}>
        {agents && !live.length && <p className="text-sm text-slate-500">No active agents. Forge one.</p>}
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">{live.map((a) => <Card key={a.agent_id} a={a} />)}</div>
      </Panel>
      <Panel title={`Scrapyard · ${dead.length}`}>
        {agents && !dead.length && <p className="text-sm text-slate-500">Nothing revoked or expired.</p>}
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">{dead.map((a) => <Card key={a.agent_id} a={a} />)}</div>
      </Panel>
    </div>
  );
}
