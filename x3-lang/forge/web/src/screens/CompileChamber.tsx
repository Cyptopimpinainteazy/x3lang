import { useState } from "react";
import { download } from "../api";
import { buildKey, useForge } from "../store";
import { Badge, Errors, Mono, Panel } from "../ui";

export function SourceView({ source, lineMap, errorNodes = [] }: { source: string; lineMap: Record<string, string>; errorNodes?: string[] }) {
  const [hover, setHover] = useState<string | null>(null);
  return (
    <pre className="code rounded border border-edge bg-void/80 py-2 font-mono">
      {source.trimEnd().split("\n").map((line, i) => {
        const node = lineMap[String(i + 1)];
        const bad = node && errorNodes.some((n) => n === node || n.startsWith(node) || node.startsWith(n));
        return (
          <div key={i} onMouseEnter={() => setHover(node ?? null)} onMouseLeave={() => setHover(null)}
               className={`flex px-3 ${bad ? "bg-danger/15" : hover && node === hover ? "bg-glow/10" : ""}`}>
            <span className="mr-4 w-6 shrink-0 text-right text-slate-600 select-none">{i + 1}</span>
            <span className={line.trim().startsWith("//") ? "text-slate-500" : "text-slate-200"}>{line}</span>
            {node && <span className="ml-auto pl-4 text-[10px] text-slate-600">{node}</span>}
          </div>
        );
      })}
    </pre>
  );
}

export function CompileChamber() {
  const { compile, compileFor, build, errors } = useForge();
  const stale = compile && compileFor !== buildKey(build);
  return (
    <div className="space-y-4">
      {errors.length > 0 && <Panel title="The compiler refused this build"><Errors errors={errors} /></Panel>}
      {!compile ? <Panel title="Compile Chamber"><p className="text-sm text-slate-400">Nothing has compiled yet. Fill in Origin and learn some skills.</p></Panel> : (
        <>
          <Panel title="Generated mandate (.x3)" right={<div className="flex items-center gap-2">
            {stale && <Badge>showing last good compile</Badge>}
            {compile.source && <button className="text-[10px] tracking-widest text-glow uppercase" onClick={() => download("agent.x3", compile.source ?? "", "text/plain")}>download agent.x3</button>}
          </div>}>
            {compile.source ? (
              <>
                <SourceView source={compile.source} lineMap={compile.line_map} errorNodes={errors.map((e) => e.node ?? "")} />
                <div className="mt-3 grid grid-cols-1 gap-1 text-xs text-slate-400 md:grid-cols-2">
                  <div>source sha256 <Mono className="text-slate-300">{compile.source_sha256}</Mono></div>
                  <div>compiled terms sha256 <Mono className="text-slate-300">{compile.compiled_sha256}</Mono></div>
                  <div>build hash (creature seed) <Mono className="text-slate-300">{compile.build_hash}</Mono></div>
                  <div>token list <span className="text-slate-300">{compile.tokens_source}</span></div>
                </div>
                <p className="mt-3 text-xs text-slate-500">Compiled by the repository's mandate compiler (<code>near_mandate.compile_mandate</code>), the same lowering as
                  <code> near_intents.py mandate args</code>. <code>terms.program_sha256</code> binds the source above.</p>
              </>
            ) : <p className="text-sm text-slate-400">No Armory Access skill, so there is no mandate. x3-guard will refuse every payment this agent tries.</p>}
          </Panel>
          {compile.compiled && (
            <Panel title="x3-guard create_mandate (for the owner to send)" right={<button className="text-[10px] tracking-widest text-glow uppercase" onClick={() => download("create_mandate.json", JSON.stringify(compile.create_mandate, null, 2))}>download</button>}>
              <pre className="code rounded border border-edge bg-void/80 p-3 font-mono text-slate-300">{JSON.stringify(compile.create_mandate, null, 2)}</pre>
              <p className="mt-2 text-xs text-gold">The Forge does not sign or send this. The owner account sends it to the x3-guard contract, which then enforces every term.</p>
            </Panel>
          )}
          <Panel title="Agent Law runtime manifest">
            <pre className="code rounded border border-edge bg-void/80 p-3 font-mono text-slate-300">{JSON.stringify(compile.manifest, null, 2)}</pre>
            <p className="mt-2 text-xs text-slate-500">Capabilities come from skills, surfaces from connections. A child agent can never hold more than this (checked by <code>agent_constitution.check_admission</code>).</p>
          </Panel>
        </>
      )}
    </div>
  );
}
