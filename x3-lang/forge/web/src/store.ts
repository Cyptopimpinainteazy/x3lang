import { create } from "zustand";
import type { Build, CompileResult, ConnectionRec, ForgeErr, Proof, RegistryResponse, Skill } from "./schema";

export type Step = "origin" | "skills" | "comms" | "alien" | "compile" | "proving" | "birth";
export const STEPS: { id: Step; label: string; hint: string }[] = [
  { id: "origin", label: "Origin", hint: "Species and identity" },
  { id: "skills", label: "Skill Tree", hint: "What it may do" },
  { id: "comms", label: "Comms Rig", hint: "What it plugs into" },
  { id: "alien", label: "Alien", hint: "Stats and cosmetics" },
  { id: "compile", label: "Compile Chamber", hint: "Real .x3 compile" },
  { id: "proving", label: "Proving Ground", hint: "Attack it" },
  { id: "birth", label: "Birth", hint: "Sign it into being" },
];

const KEY = "x3-agent-forge/build/v1";

export function emptyBuild(archetype = "trader"): Build {
  return {
    version: 1,
    name: "",
    archetype,
    identity: { owner: "", agent: "", expires: "30d", budget: 1000 },
    skills: {},
    connections: [],
    cosmetics: { palette: "acid", pattern: "smooth", idle: "breathe", glow: "cyan" },
  };
}

// The repository's own example accounts (examples/near_mandate_agent.x3). Replace them with
// accounts you control before you send the mandate on chain.
export function exampleBuild(): Build {
  return {
    ...emptyBuild("trader"),
    name: "Vexa",
    identity: { owner: "x3-owner.near", agent: "vexa.x3-owner.near", expires: "30d", budget: 1000 },
    skills: {
      read_balances: {}, market_watch: {}, quote_hunter: {}, spend: {}, payees: {},
      cosign: { approvers: ["x3-owner.near"] }, hive_voice: {},
    },
  };
}

function load(): Build {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const b = JSON.parse(raw) as Build;
      if (b && b.version === 1 && typeof b.skills === "object") return b;
    }
  } catch {
    /* storage unavailable: start fresh */
  }
  return emptyBuild();
}

function save(b: Build) {
  try {
    localStorage.setItem(KEY, JSON.stringify(b));
  } catch {
    /* ignore */
  }
}

interface State {
  registry: RegistryResponse | null;
  build: Build;
  step: Step;
  compile: CompileResult | null;
  compileFor: string | null;
  errors: ForgeErr[];
  compiling: boolean;
  proof: Proof | null;
  connections: Record<string, ConnectionRec>;
  selectedSkill: string | null;
  setRegistry: (r: RegistryResponse) => void;
  setStep: (s: Step) => void;
  setBuild: (f: (b: Build) => Build) => void;
  replaceBuild: (b: Build) => void;
  setCompile: (c: CompileResult | null, errors: ForgeErr[], key: string | null) => void;
  setCompiling: (v: boolean) => void;
  setProof: (p: Proof | null) => void;
  setConnection: (rec: ConnectionRec) => void;
  selectSkill: (id: string | null) => void;
}

export const useForge = create<State>((set) => ({
  registry: null,
  build: load(),
  step: "origin",
  compile: null,
  compileFor: null,
  errors: [],
  compiling: false,
  proof: null,
  connections: {},
  selectedSkill: null,
  setRegistry: (registry) => set({ registry }),
  setStep: (step) => set({ step }),
  setBuild: (f) =>
    set((s) => {
      const build = f(structuredClone(s.build));
      save(build);
      return { build, proof: null };
    }),
  replaceBuild: (build) => {
    save(build);
    set({ build, proof: null, compile: null });
  },
  setCompile: (compile, errors, compileFor) =>
    set((s) => ({ compile: compile ?? s.compile, errors, compileFor: compile ? compileFor : s.compileFor })),
  setCompiling: (compiling) => set({ compiling }),
  setProof: (proof) => set({ proof }),
  setConnection: (rec) => set((s) => ({ connections: { ...s.connections, [rec.id]: rec } })),
  selectSkill: (selectedSkill) => set({ selectedSkill }),
}));

/** The build without cosmetics: what the host hashes. Used to know when a compile is stale. */
export function buildKey(b: Build): string {
  const { cosmetics: _c, ...rest } = b;
  void _c;
  return JSON.stringify(rest);
}

export function skillMap(r: RegistryResponse | null): Record<string, Skill> {
  return Object.fromEntries((r?.skills ?? []).map((s) => [s.id, s]));
}

export function errorsFor(errors: ForgeErr[], prefix: string): ForgeErr[] {
  return errors.filter((e) => e.node === prefix || (e.node ?? "").startsWith(prefix + "."));
}
