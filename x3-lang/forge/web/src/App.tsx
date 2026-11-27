import { useEffect, useMemo, useState } from "react";
import { api, errorsOf } from "./api";
import { CreatureView } from "./creature/Creature";
import { genome, implantStateOf } from "./creature/genome";
import { RegistryResponse, type CompileResult, type ConnectionRec } from "./schema";
import { STEPS, buildKey, useForge } from "./store";
import { Badge, Errors, Panel, StatBars, short } from "./ui";
import { Origin } from "./screens/Origin";
import { SkillTree } from "./screens/SkillTree";
import { CommsRig } from "./screens/CommsRig";
import { AlienScreen } from "./screens/AlienScreen";
import { CompileChamber } from "./screens/CompileChamber";
import { ProvingGround } from "./screens/ProvingGround";
import { Birth } from "./screens/Birth";
import { Profile } from "./screens/Profile";
import { Roster } from "./screens/Roster";
import { ImportView } from "./screens/ImportView";
import { CommandChip } from "./screens/Command";

export function navigate(path: string) {
  window.history.pushState({}, "", path);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

function usePath() {
  const [path, setPath] = useState(window.location.pathname);
  useEffect(() => {
    const on = () => setPath(window.location.pathname);
    window.addEventListener("popstate", on);
    return () => window.removeEventListener("popstate", on);
  }, []);
  return path;
}

export function App() {
  const path = usePath();
  const { registry, setRegistry } = useForge();
  const [fatal, setFatal] = useState<string | null>(null);

  useEffect(() => {
    api.get<unknown>("/api/registry")
      .then((raw) => {
        const parsed = RegistryResponse.safeParse(raw);
        if (!parsed.success) throw new Error(`registry from host failed schema validation: ${parsed.error.message}`);
        setRegistry(parsed.data);
      })
      .catch((e) => setFatal(errorsOf(e).map((x) => x.message).join("; ")));
  }, [setRegistry]);

  if (fatal) return <div className="grid h-full place-items-center p-8 text-danger">Forge host unavailable: {fatal}</div>;
  if (!registry) return <div className="grid h-full place-items-center font-display text-xs text-glow">Warming the vats…</div>;

  const agentMatch = path.match(/^\/agents\/([a-z0-9._-]{2,64})\/?$/);
  return (
    <div className="grid-bg min-h-full">
      <Header path={path} />
      {agentMatch ? <Profile agentId={agentMatch[1]} /> : path.startsWith("/roster") ? <Roster /> : path.startsWith("/import") ? <ImportView /> : <Forge />}
    </div>
  );
}

function Header({ path }: { path: string }) {
  const links = [["/", "Forge"], ["/roster", "Roster"], ["/import", "Import"]] as const;
  return (
    <header className="sticky top-0 z-20 flex items-center gap-6 border-b border-edge bg-void/85 px-5 py-3 backdrop-blur">
      <a href="/" onClick={(e) => { e.preventDefault(); navigate("/"); }} className="flex items-center gap-2">
        <span className="grid h-7 w-7 place-items-center rounded-full border border-acid/70 text-acid shadow-[0_0_14px_rgba(168,255,61,.35)]">✶</span>
        <span className="font-display text-sm text-slate-100">X3 Agent Forge</span>
      </a>
      <nav className="flex gap-1">
        {links.map(([href, label]) => {
          const on = href === "/" ? path === "/" : path.startsWith(href);
          return (
            <a key={href} href={href} onClick={(e) => { e.preventDefault(); navigate(href); }}
               className={`rounded px-3 py-1 text-xs tracking-widest uppercase ${on ? "bg-glow/10 text-glow" : "text-slate-400 hover:text-slate-200"}`}>{label}</a>
          );
        })}
      </nav>
      <div className="ml-auto"><CommandChip /></div>
    </header>
  );
}

function useLiveCompile() {
  const { build, setCompile, setCompiling } = useForge();
  const key = buildKey(build);
  useEffect(() => {
    const ctrl = new AbortController();
    setCompiling(true); // errors on screen belong to the previous build until this returns
    const t = setTimeout(async () => {
      try {
        const res = await api.post<CompileResult>("/api/compile", { build }, ctrl.signal);
        setCompile(res, [], key);
      } catch (e) {
        const errs = errorsOf(e);
        if (errs.length) setCompile(null, errs, null);
      } finally {
        if (!ctrl.signal.aborted) setCompiling(false);
      }
    }, 350);
    return () => {
      clearTimeout(t);
      ctrl.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

function useConnectionSync() {
  const { build, setConnection, setStep } = useForge();
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    const q = new URLSearchParams(window.location.search);
    const linked = q.get("linked");
    const err = q.get("link_error");
    if (linked || err) {
      window.history.replaceState({}, "", "/" + window.location.hash);
      setStep("comms");
      if (err) setNotice(`Setup return refused: ${err}`);
      if (linked) setNotice("Setup page returned and its state matched: connection linked.");
    }
    for (const c of build.connections) {
      if (c.connection_id) api.get<ConnectionRec>(`/api/connect/${c.connection_id}`).then(setConnection).catch(() => undefined);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return [notice, setNotice] as const;
}

function Forge() {
  useLiveCompile();
  const [notice, setNotice] = useConnectionSync();
  const { step, setStep, compile, compileFor, errors, compiling, build, connections } = useForge();
  const stale = compile !== null && compileFor !== buildKey(build);
  const implantStates = useMemo(() => {
    const out: Record<string, ReturnType<typeof implantStateOf>> = {};
    for (const c of build.connections) {
      const rec = c.connection_id ? connections[c.connection_id] : undefined;
      out[c.connector] = rec ? implantStateOf(rec.status, rec.health?.status ?? null) : "dim";
    }
    return out;
  }, [build.connections, connections]);
  const g = useMemo(() => (compile ? genome({ traits: compile.traits, cosmetics: build.cosmetics, implantStates }) : null),
    [compile, build.cosmetics, implantStates]);
  const idx = STEPS.findIndex((s) => s.id === step);
  const Screen = { origin: Origin, skills: SkillTree, comms: CommsRig, alien: AlienScreen, compile: CompileChamber, proving: ProvingGround, birth: Birth }[step];

  return (
    <div className="mx-auto grid max-w-[1500px] grid-cols-1 gap-4 p-4 lg:grid-cols-[210px_minmax(0,1fr)_380px]">
      <nav className="space-y-1 lg:sticky lg:top-20 lg:self-start">
        {STEPS.map((s, i) => (
          <button key={s.id} onClick={() => setStep(s.id)}
                  className={`flex w-full items-center gap-3 rounded border px-3 py-2 text-left transition ${s.id === step ? "glow-edge border-transparent bg-glow/5" : "border-edge bg-panel/50 hover:border-slate-500"}`}>
            <span className={`grid h-6 w-6 place-items-center rounded-full border font-mono text-[10px] ${i <= idx ? "border-glow text-glow" : "border-slate-600 text-slate-500"}`}>{i + 1}</span>
            <span>
              <span className="block text-xs font-semibold tracking-wider text-slate-100 uppercase">{s.label}</span>
              <span className="block text-[10px] text-slate-500">{s.hint}</span>
            </span>
          </button>
        ))}
      </nav>
      <main className="min-w-0 space-y-4">
        {notice && (
          <div className="flex items-center justify-between rounded border border-glow/40 bg-glow/5 px-3 py-2 text-xs text-glow">
            {notice}<button className="text-slate-400" onClick={() => setNotice(null)}>✕</button>
          </div>
        )}
        <Screen />
        <div className="flex justify-between">
          <button disabled={idx === 0} onClick={() => setStep(STEPS[idx - 1].id)} className="text-xs tracking-widest text-slate-400 uppercase disabled:opacity-30">← {STEPS[idx - 1]?.label ?? ""}</button>
          <button disabled={idx === STEPS.length - 1} onClick={() => setStep(STEPS[idx + 1].id)} className="text-xs tracking-widest text-glow uppercase disabled:opacity-30">{STEPS[idx + 1]?.label ?? ""} →</button>
        </div>
      </main>
      <aside className="space-y-4 lg:sticky lg:top-20 lg:self-start">
        <Panel title={build.name || "Unnamed specimen"} right={
          compiling ? <Badge tone="glow">compiling</Badge> : errors.length ? <Badge tone="danger">won't compile</Badge> : stale ? <Badge>stale</Badge> : compile ? <Badge tone="acid">compiled</Badge> : null
        }>
          <div className={`relative h-72 overflow-hidden rounded border border-edge ${errors.length ? "opacity-50" : ""}`}>
            {g ? <CreatureView genome={g} className="h-full w-full" /> : <div className="grid h-full place-items-center text-xs text-slate-500">The creature grows from a compiled build.</div>}
          </div>
          <div className="mt-3 flex items-center justify-between text-[10px] text-slate-500">
            <span>seed <span className="font-mono text-slate-400">{short(compile?.build_hash, 8)}</span></span>
            <span>{compile?.traits.body ?? ""}</span>
          </div>
          <div className="mt-3"><StatBars stats={compile?.stats ?? null} dim={!!errors.length} /></div>
        </Panel>
        {errors.length > 0 && <Panel title={compiling ? "Compile errors · rechecking" : "Compile errors"}><div className={compiling ? "opacity-50" : ""}><Errors errors={errors} /></div></Panel>}
      </aside>
    </div>
  );
}
