import { useForge, emptyBuild, exampleBuild, errorsFor } from "../store";
import { Badge, Button, Field, Panel, inputCls } from "../ui";

const BODY_GLYPH: Record<string, string> = { serpent: "∿", crustacean: "⌬", insectoid: "⋔", cephalopod: "☋", titan: "⛬" };

export function Origin() {
  const { registry, build, setBuild, replaceBuild, errors } = useForge();
  if (!registry) return null;
  const errOf = (node: string) => errorsFor(errors, node)[0]?.message;
  const setIdent = (k: keyof typeof build.identity, v: string | number) => setBuild((b) => ({ ...b, identity: { ...b.identity, [k]: v } }));
  return (
    <div className="space-y-4">
      <Panel title="Choose a species" right={
        <div className="flex gap-2">
          <Button tone="ghost" onClick={() => replaceBuild(exampleBuild())} title="Fills the repository's example accounts (examples/near_mandate_agent.x3)">Load example</Button>
          <Button tone="ghost" onClick={() => replaceBuild(emptyBuild(build.archetype))}>Reset</Button>
        </div>
      }>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-5">
          {registry.archetypes.map((a) => {
            const on = a.id === build.archetype;
            return (
              <button key={a.id} onClick={() => setBuild((b) => ({
                ...b, archetype: a.id,
                skills: Object.fromEntries(Object.entries(b.skills).filter(([k]) => a.skill_pool.includes(k))),
              }))} className={`rounded-lg border p-3 text-left transition ${on ? "glow-edge border-transparent bg-glow/5" : "border-edge hover:border-slate-500"}`}>
                <div className="text-3xl text-acid">{BODY_GLYPH[a.body]}</div>
                <div className="mt-2 font-display text-xs text-slate-100">{a.name}</div>
                <div className="text-[10px] tracking-widest text-slate-500 uppercase">{a.body} · {a.points} pts</div>
                <p className="mt-2 text-xs text-slate-400">{a.blurb}</p>
              </button>
            );
          })}
        </div>
      </Panel>
      <Panel title="Identity" right={<Badge tone="gold">compiles to owner / agent / expires</Badge>}>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Field label="Agent name" error={errOf("origin.name")}>
            <input className={inputCls} value={build.name} maxLength={40} placeholder="Vexa" onChange={(e) => setBuild((b) => ({ ...b, name: e.target.value }))} />
          </Field>
          <Field label="Mandate lasts" error={errOf("origin.expires")}>
            <input className={inputCls} value={build.identity.expires} placeholder="30d" onChange={(e) => setIdent("expires", e.target.value.trim())} />
          </Field>
          <Field label="Owner NEAR account (High Command)" error={errOf("origin.owner")}>
            <input className={inputCls} value={build.identity.owner} placeholder="you.near" onChange={(e) => setIdent("owner", e.target.value.trim())} />
          </Field>
          <Field label="Agent NEAR account" error={errOf("origin.agent")}>
            <input className={inputCls} value={build.identity.agent} placeholder="agent.you.near" onChange={(e) => setIdent("agent", e.target.value.trim())} />
          </Field>
          <Field label="Runtime budget (Agent Law units)" error={errOf("origin.budget")}>
            <input className={inputCls} type="number" min={1} value={build.identity.budget}
                   onChange={(e) => setIdent("budget", Number.parseInt(e.target.value || "0", 10))} />
          </Field>
        </div>
        <p className="mt-3 text-xs text-slate-500">
          The agent account is the agent's identity in the Agent Law runtime and the <code>agent</code> of its x3-guard mandate.
          The owner account signs and sends <code>create_mandate</code> itself. The Forge never holds a NEAR key.
        </p>
      </Panel>
    </div>
  );
}
