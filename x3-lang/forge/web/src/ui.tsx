import type { ReactNode } from "react";
import type { ForgeErr, Stats } from "./schema";

export function Panel({ title, right, children, className = "" }: { title?: ReactNode; right?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`rounded-lg border border-edge bg-panel/80 backdrop-blur ${className}`}>
      {title && (
        <header className="flex items-center justify-between gap-3 border-b border-edge px-4 py-2.5">
          <h2 className="font-display text-xs text-glow">{title}</h2>
          {right}
        </header>
      )}
      <div className="p-4">{children}</div>
    </section>
  );
}

export function Button({ children, onClick, disabled, tone = "glow", type = "button", title }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; tone?: "glow" | "danger" | "ghost" | "acid"; type?: "button" | "submit"; title?: string;
}) {
  const tones = {
    glow: "border-glow/60 text-glow hover:bg-glow/10",
    acid: "border-acid/60 text-acid hover:bg-acid/10",
    danger: "border-danger/70 text-danger hover:bg-danger/10",
    ghost: "border-edge text-slate-300 hover:bg-white/5",
  };
  return (
    <button type={type} title={title} disabled={disabled} onClick={onClick}
            className={`rounded border px-3 py-1.5 text-xs font-semibold tracking-wider uppercase transition disabled:cursor-not-allowed disabled:opacity-35 ${tones[tone]}`}>
      {children}
    </button>
  );
}

export function Badge({ children, tone = "slate", title }: { children: ReactNode; tone?: "slate" | "glow" | "gold" | "acid" | "danger" | "violet"; title?: string }) {
  const tones = {
    slate: "border-slate-600 text-slate-300",
    glow: "border-glow/60 text-glow",
    gold: "border-gold/60 text-gold",
    acid: "border-acid/60 text-acid",
    danger: "border-danger/70 text-danger",
    violet: "border-violet-400/60 text-violet-300",
  };
  return <span title={title} className={`inline-flex items-center rounded border px-1.5 py-0.5 text-[10px] font-semibold tracking-wider uppercase ${tones[tone]}`}>{children}</span>;
}

export function EnforcedBy({ by }: { by: string }) {
  if (by === "forge-gate")
    return <Badge tone="violet" title="Checked by the Forge inference gate on every signed request before it reaches the GPU mesh">forge gate</Badge>;
  return by === "x3-guard"
    ? <Badge tone="gold" title="Enforced on NEAR by the x3-guard contract once the owner sends create_mandate">x3-guard · on chain</Badge>
    : <Badge tone="glow" title="Recorded in the Agent Law runtime and checked on delegation; no production adapter enforces it yet">agent runtime</Badge>;
}

export function RiskPips({ risk }: { risk: number }) {
  return (
    <span className="inline-flex gap-0.5" title={`risk tier ${risk} of 4`}>
      {[0, 1, 2, 3].map((i) => (
        <span key={i} className={`h-1.5 w-2.5 rounded-sm ${i < risk ? (risk >= 3 ? "bg-danger" : "bg-gold") : "bg-slate-700"}`} />
      ))}
    </span>
  );
}

export function Errors({ errors, empty }: { errors: ForgeErr[]; empty?: ReactNode }) {
  if (!errors.length) return <>{empty ?? null}</>;
  return (
    <ul className="space-y-1.5">
      {errors.map((e, i) => (
        <li key={i} className="rounded border border-danger/40 bg-danger/5 px-3 py-2 text-xs text-red-200">
          <span className="font-mono text-[10px] text-danger">{e.code}</span>
          {e.node && <span className="ml-2 font-mono text-[10px] text-slate-400">{e.node}</span>}
          <div className="mt-0.5">{e.message}</div>
        </li>
      ))}
    </ul>
  );
}

const STAT_LABEL: Record<keyof Stats, string> = { power: "Power", reach: "Reach", autonomy: "Autonomy", risk: "Risk", speed: "Speed" };
export function StatBars({ stats, dim }: { stats: Stats | null; dim?: boolean }) {
  return (
    <div className={`space-y-2 ${dim ? "opacity-40" : ""}`}>
      {(Object.keys(STAT_LABEL) as (keyof Stats)[]).map((k) => (
        <div key={k}>
          <div className="flex justify-between text-[10px] tracking-widest text-slate-400 uppercase">
            <span>{STAT_LABEL[k]}</span><span className="font-mono">{stats ? stats[k] : "—"}</span>
          </div>
          <div className="mt-0.5 h-1.5 rounded bg-slate-800">
            <div className={`h-1.5 rounded ${k === "risk" ? "bg-gradient-to-r from-gold to-danger" : "bg-gradient-to-r from-glow to-acid"}`}
                 style={{ width: `${stats ? stats[k] : 0}%` }} />
          </div>
        </div>
      ))}
    </div>
  );
}

export function Mono({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <span className={`font-mono text-[11px] break-all ${className}`}>{children}</span>;
}

export function Field({ label, children, error, group }: { label: string; children: ReactNode; error?: string; group?: boolean }) {
  // `group` for sets of buttons: a <label> around buttons would name the first button after the label.
  const inner = (
    <>
      <span className="mb-1 block text-[10px] tracking-widest text-slate-400 uppercase">{label}</span>
      {children}
      {error && <span className="mt-1 block text-[11px] text-danger">{error}</span>}
    </>
  );
  return group ? <div role="group" aria-label={label} className="block">{inner}</div> : <label className="block">{inner}</label>;
}

export const inputCls = "w-full rounded border border-edge bg-void/70 px-2.5 py-1.5 text-sm text-slate-100 outline-none focus:border-glow/70";

export function short(h: string | null | undefined, n = 10) {
  if (!h) return "—";
  return h.length > 2 * n ? `${h.slice(0, n)}…${h.slice(-4)}` : h;
}
