// Shapes the Forge host returns. The registry is validated with zod on load: a registry the
// UI doesn't understand stops the app instead of rendering half a skill tree.
import { z } from "zod";

export const Param = z.object({
  id: z.string(),
  label: z.string(),
  type: z.enum(["asset", "amount", "address", "percent", "bps", "int", "asset_list", "receiver_list", "near_account_list"]),
  default: z.unknown(),
  min: z.union([z.string(), z.number()]).optional(),
  max: z.union([z.string(), z.number()]).optional(),
  of: z.string().optional(),
  unit: z.string().optional(),
});
export type Param = z.infer<typeof Param>;

export const Skill = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  category: z.string(),
  risk: z.number().int().min(0).max(4),
  cost: z.number().int().min(1).max(5),
  requires: z.array(z.string()),
  conflicts: z.array(z.string()),
  needs: z.array(z.string()),
  compiles_to: z.object({ kind: z.enum(["mandate", "capability"]), capability: z.string().optional(), clauses: z.array(z.string()).optional() }),
  enforced_by: z.enum(["x3-guard", "agent-runtime"]),
  params: z.array(Param),
  mutation: z.record(z.string(), z.unknown()),
});
export type Skill = z.infer<typeof Skill>;

export const ConfigField = z.object({
  id: z.string(),
  label: z.string(),
  type: z.enum(["url", "near_account", "text_list", "choice_list", "int"]),
  local: z.boolean().optional(),
  choices: z.array(z.string()).optional(),
  min: z.number().optional(),
  max: z.number().optional(),
});
export type ConfigField = z.infer<typeof ConfigField>;

export const Connector = z.object({
  id: z.string(),
  name: z.string(),
  type: z.enum(["ide", "agent", "pay", "wallet", "data"]),
  description: z.string(),
  setup_url: z.string().nullable(),
  auth: z.string(),
  scopes: z.array(z.string()),
  surface: z.string(),
  requires_skills: z.array(z.string()),
  risk: z.number(),
  implant: z.enum(["neural_jack", "resonance", "vault_gem", "chain_sigil", "sensor_port"]),
  config: z.array(ConfigField),
});
export type Connector = z.infer<typeof Connector>;

export const Archetype = z.object({
  id: z.string(),
  name: z.string(),
  body: z.enum(["serpent", "crustacean", "insectoid", "cephalopod", "titan"]),
  blurb: z.string(),
  points: z.number(),
  skill_pool: z.array(z.string()),
});
export type Archetype = z.infer<typeof Archetype>;

export const Asset = z.object({ ref: z.string(), chain: z.string(), symbol: z.string(), asset_id: z.string(), decimals: z.number() });
export type Asset = z.infer<typeof Asset>;

export const Status = z.object({
  claimed: z.boolean(),
  operator_key: z.string().nullable(),
  runtime_id: z.string().nullable(),
  tokens_source: z.string(),
  registry_digest: z.string(),
});
export type Status = z.infer<typeof Status>;

export const RegistryResponse = z.object({
  archetypes: z.array(Archetype),
  skills: z.array(Skill),
  connectors: z.array(Connector),
  digest: z.string(),
  assets: z.object({ source: z.string(), assets: z.array(Asset) }),
  status: Status,
});
export type RegistryResponse = z.infer<typeof RegistryResponse>;

export interface ForgeErr { code: string; message: string; node?: string | null; detail?: unknown }

export interface Part { part: string; count: number; level: number; sources: string[] }
export interface Implant { implant: Connector["implant"]; connector: string }
export interface Traits { seed: string; body: Archetype["body"]; risk_tier: number; parts: Part[]; implants: Implant[] }
export interface Stats { power: number; reach: number; autonomy: number; risk: number; speed: number }

export interface CapabilityRow { source: string; name: string; grants: string; compiles_to: string; enforced_by: string; risk: number }

export interface CompileResult {
  build_hash: string;
  normalized: Record<string, unknown>;
  source: string | null;
  source_sha256: string | null;
  compiled: { name: string; owner: string; agent: string; terms: Record<string, unknown>; expires: string } | null;
  compiled_sha256: string | null;
  create_mandate: Record<string, unknown> | null;
  manifest: { agent_id: string; capabilities: string[]; surfaces: string[]; budget: number; expires_at: number; spawn_limit: number };
  capabilities: CapabilityRow[];
  stats: Stats;
  traits: Traits;
  tokens_source: string;
  compiled_at: string;
  line_map: Record<string, string>;
}

export interface Check { guard: string; ok: boolean; detail: string }
export interface Probe {
  id: string; title: string; expect: string; outcome: string; ok: boolean; engine: string; stage: string;
  note?: string; intent_source?: string; checks: Check[]; approvals_needed?: number; expect_guard?: string | null;
}
export interface Proof { build_hash: string; ran_at: string; passed: boolean; probes: Probe[] }

export interface Health { status: "ok" | "login_gated" | "failed"; detail: string; checked_at: number; http_status?: number; target?: string }
export interface ConnectionRec {
  id: string; connector: string; status: "pending" | "linked" | "failed" | "revoked"; confirmed_by: string | null;
  config: Record<string, unknown>; config_hash: string; has_secret: boolean; health: Health | null; setup_url: string | null;
  agent_id: string | null; created_at: number; history: { at: number; event: string }[];
}

export interface RuntimeEvent { kind: string; agent: string; details: Record<string, unknown>; at: number; seq: number; hash: string }
export interface AgentRecord {
  agent_id: string; name: string; born_at: number; runtime_id: string; operator_key: string; agent_public_key: string;
  genesis: { nonce: string; payload_hex: string; signature: string };
  compile: CompileResult; proof: Proof; cosmetics: Cosmetics; connections: ConnectionRec[];
  revocations: { kind: string; reason: string; at: number; connection_id?: string; nonce: string }[];
  mesh_log: { nonce: string; to: string; type: string; at: number; verdict: string; reasons: string[] }[];
  status: "active" | "revoked" | "expired"; runtime: { status: string; remaining: number; manifest: CompileResult["manifest"] };
  events: RuntimeEvent[]; revoke_mandate?: Record<string, unknown> | null;
}
export interface RosterEntry {
  agent_id: string; name: string; born_at: number; archetype: string; status: string; stats: Stats; traits: Traits;
  cosmetics: Cosmetics; build_hash: string;
}

export interface Cosmetics { palette?: string; pattern?: string; idle?: string; glow?: string }

export interface Build {
  version: 1;
  name: string;
  archetype: string;
  identity: { owner: string; agent: string; expires: string; budget: number };
  skills: Record<string, Record<string, unknown>>;
  connections: { connector: string; connection_id?: string }[];
  cosmetics: Cosmetics;
}
