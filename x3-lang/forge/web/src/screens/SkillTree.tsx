import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Param, Skill } from "../schema";
import { errorsFor, skillMap, useForge } from "../store";
import { Badge, EnforcedBy, Errors, Field, Panel, RiskPips, inputCls } from "../ui";

function depthOf(id: string, skills: Record<string, Skill>, memo: Record<string, number> = {}): number {
  if (memo[id] !== undefined) return memo[id];
  const reqs = skills[id]?.requires ?? [];
  memo[id] = reqs.length ? 1 + Math.max(...reqs.map((r) => depthOf(r, skills, memo))) : 0;
  return memo[id];
}

export function SkillTree() {
  const { registry, build, setBuild, errors, selectedSkill, selectSkill } = useForge();
  const skills = useMemo(() => skillMap(registry), [registry]);
  const arch = registry?.archetypes.find((a) => a.id === build.archetype);
  const pool = arch?.skill_pool ?? [];
  const chosen = new Set(Object.keys(build.skills));
  const spent = [...chosen].reduce((n, s) => n + (skills[s]?.cost ?? 0), 0);
  const columns = useMemo(() => {
    const cols: string[][] = [];
    for (const id of pool) {
      const d = depthOf(id, skills);
      (cols[d] ??= []).push(id);
    }
    return cols.map((c) => c ?? []);
  }, [pool, skills]);

  const boxRef = useRef<HTMLDivElement>(null);
  const nodeRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const [lines, setLines] = useState<{ x1: number; y1: number; x2: number; y2: number; on: boolean }[]>([]);
  useLayoutEffect(() => {
    const box = boxRef.current?.getBoundingClientRect();
    if (!box) return;
    const out: typeof lines = [];
    for (const id of pool) {
      for (const req of skills[id]?.requires ?? []) {
        const a = nodeRefs.current[req]?.getBoundingClientRect();
        const b = nodeRefs.current[id]?.getBoundingClientRect();
        if (!a || !b) continue;
        out.push({ x1: a.right - box.left, y1: a.top + a.height / 2 - box.top, x2: b.left - box.left, y2: b.top + b.height / 2 - box.top,
                   on: chosen.has(req) && chosen.has(id) });
      }
    }
    setLines(out);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pool.join(","), [...chosen].join(","), registry]);

  if (!registry || !arch) return null;

  const lockReason = (s: Skill): string | null => {
    if (chosen.has(s.id)) return null;
    const missing = s.requires.filter((r) => !chosen.has(r));
    if (missing.length) return `needs ${missing.map((m) => skills[m].name).join(", ")}`;
    if (spent + s.cost > arch.points) return "not enough points";
    const clash = s.conflicts.find((c) => chosen.has(c));
    if (clash) return `conflicts with ${skills[clash].name}`;
    return null;
  };

  const toggle = (s: Skill) => {
    if (chosen.has(s.id)) {
      // Removing a skill removes everything that requires it.
      const drop = new Set([s.id]);
      let grew = true;
      while (grew) {
        grew = false;
        for (const id of chosen) {
          if (!drop.has(id) && skills[id].requires.some((r) => drop.has(r))) {
            drop.add(id);
            grew = true;
          }
        }
      }
      setBuild((b) => ({ ...b, skills: Object.fromEntries(Object.entries(b.skills).filter(([k]) => !drop.has(k))) }));
      if (selectedSkill && drop.has(selectedSkill)) selectSkill(null);
      return;
    }
    if (lockReason(s)) return;
    setBuild((b) => ({ ...b, skills: { ...b.skills, [s.id]: {} } }));
    selectSkill(s.id);
  };

  const sel = selectedSkill ? skills[selectedSkill] : null;
  return (
    <div className="space-y-4">
      <Panel title={`${arch.name} skill tree`} right={<span className="font-mono text-xs text-slate-300">{spent} / {arch.points} pts</span>}>
        <div ref={boxRef} className="relative overflow-x-auto">
          <svg className="pointer-events-none absolute inset-0 h-full w-full">
            {lines.map((l, i) => {
              const mx = (l.x1 + l.x2) / 2;
              return <path key={i} d={`M${l.x1},${l.y1} C${mx},${l.y1} ${mx},${l.y2} ${l.x2},${l.y2}`} fill="none"
                           stroke={l.on ? "#38f2ff" : "#2a3647"} strokeWidth={l.on ? 2 : 1.2} strokeDasharray={l.on ? "" : "4 4"} />;
            })}
          </svg>
          <div className="relative flex min-w-max gap-10">
            {columns.map((col, i) => (
              <div key={i} className="flex flex-col justify-center gap-3">
                {col.map((id) => {
                  const s = skills[id];
                  const on = chosen.has(id);
                  const lock = lockReason(s);
                  const errs = errorsFor(errors, `skill.${id}`);
                  return (
                    <button key={id} ref={(el) => { nodeRefs.current[id] = el; }} onClick={() => (on ? selectSkill(id) : toggle(s))}
                            onDoubleClick={() => on && toggle(s)}
                            className={`w-52 rounded-lg border p-2.5 text-left transition ${errs.length ? "danger-edge border-transparent" : on ? "glow-edge border-transparent bg-glow/10" : lock ? "border-edge opacity-45" : "border-edge hover:border-slate-400"} ${selectedSkill === id ? "ring-1 ring-acid" : ""}`}>
                      <div className="flex items-center justify-between">
                        <span className="text-sm font-semibold text-slate-100">{s.name}</span>
                        <span className="font-mono text-[10px] text-slate-400">{s.cost}pt</span>
                      </div>
                      <div className="mt-1 flex items-center justify-between">
                        <RiskPips risk={s.risk} />
                        <span className="text-[10px] text-slate-500">{s.compiles_to.kind === "mandate" ? "mandate" : s.compiles_to.capability}</span>
                      </div>
                      {lock && <div className="mt-1 text-[10px] text-gold">{lock}</div>}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </div>
        <p className="mt-4 text-xs text-slate-500">Click a skill to learn it, click again to tune it, double-click to drop it (and anything that needs it).
          There is no skill that lets an agent change its own mandate, and the registry refuses one.</p>
      </Panel>
      {sel && chosen.has(sel.id) && <SkillEditor skill={sel} onRemove={() => toggle(sel)} />}
    </div>
  );
}

function listText(v: unknown): string {
  return Array.isArray(v) ? v.join(", ") : "";
}

function SkillEditor({ skill, onRemove }: { skill: Skill; onRemove: () => void }) {
  const { registry, build, setBuild, errors } = useForge();
  const values = build.skills[skill.id] ?? {};
  const set = (pid: string, value: unknown) => setBuild((b) => ({ ...b, skills: { ...b.skills, [skill.id]: { ...b.skills[skill.id], [pid]: value } } }));
  const assets = registry?.assets.assets ?? [];
  const spendAsset = (build.skills.spend?.asset as string | undefined) ?? "Solana.USDC";
  const unit = (p: Param) => (p.of === "asset" ? ((values.asset as string) ?? "Solana.USDC") : p.of === "spend.asset" ? spendAsset : "").split(".")[1] ?? "";
  const errs = errorsFor(errors, `skill.${skill.id}`);
  return (
    <Panel title={skill.name} right={<div className="flex items-center gap-2"><EnforcedBy by={skill.enforced_by} /><button onClick={onRemove} className="text-[10px] tracking-widest text-danger uppercase">drop</button></div>}>
      <p className="mb-3 text-sm text-slate-300">{skill.description}</p>
      {skill.compiles_to.kind === "mandate" && (
        <div className="mb-3 text-xs text-slate-400">Compiles to mandate clauses: {skill.compiles_to.clauses?.map((c) => <Badge key={c} tone="gold">{c}</Badge>)}</div>
      )}
      {skill.params.length === 0 && <p className="text-xs text-slate-500">No parameters. Compiles to runtime capability <code>{skill.compiles_to.capability}</code>.</p>}
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        {skill.params.map((p) => {
          const v = values[p.id] ?? p.default;
          const err = errorsFor(errors, `skill.${skill.id}.${p.id}`)[0]?.message;
          const label = `${p.label}${unit(p) && (p.type === "amount") ? ` (${unit(p)})` : ""}`;
          switch (p.type) {
            case "asset":
              return <Field key={p.id} label={label} error={err}><select className={inputCls} value={String(v)} onChange={(e) => set(p.id, e.target.value)}>
                {assets.map((a) => <option key={a.ref} value={a.ref}>{a.ref} ({a.decimals} dp)</option>)}
              </select></Field>;
            case "asset_list":
              return <Field key={p.id} label={label} error={err} group><div className="flex flex-wrap gap-2">
                {assets.map((a) => {
                  const list = (Array.isArray(v) ? v : []) as string[];
                  const on = list.includes(a.ref);
                  return <button key={a.ref} onClick={() => set(p.id, on ? list.filter((x) => x !== a.ref) : [...list, a.ref])}
                                 className={`rounded border px-2 py-1 text-xs ${on ? "border-glow text-glow" : "border-edge text-slate-400"}`}>{a.ref}</button>;
                })}
              </div></Field>;
            case "receiver_list":
            case "near_account_list":
              return <Field key={p.id} label={`${label} (comma separated)`} error={err}>
                <textarea className={inputCls} rows={2} defaultValue={listText(v)} key={listText(v)}
                          onBlur={(e) => set(p.id, e.target.value.split(/[\s,]+/).map((x) => x.trim()).filter(Boolean))} />
              </Field>;
            case "int":
            case "bps":
              return <Field key={p.id} label={`${label}${p.type === "bps" ? " " : ""}`} error={err}>
                <input className={inputCls} type="number" min={Number(p.min)} max={Number(p.max)} value={Number(v)} onChange={(e) => set(p.id, Number.parseInt(e.target.value || "0", 10))} />
              </Field>;
            default:
              return <Field key={p.id} label={label} error={err}>
                <input className={inputCls} value={String(v)} onChange={(e) => set(p.id, e.target.value.trim())} />
              </Field>;
          }
        })}
      </div>
      {errs.filter((e) => e.node === `skill.${skill.id}`).length > 0 && <div className="mt-3"><Errors errors={errs.filter((e) => e.node === `skill.${skill.id}`)} /></div>}
    </Panel>
  );
}
