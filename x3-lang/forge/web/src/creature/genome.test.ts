import { describe, expect, it } from "vitest";
import { genome } from "./genome";
import type { Traits } from "../schema";

const traits: Traits = {
  seed: "ca9fa0dfe05c7680eee4da8a925080352f352e1ead1b86e62fe785c58478fb07",
  body: "serpent",
  risk_tier: 3,
  parts: [
    { part: "core", count: 0, level: 4, sources: ["spend"] },
    { part: "eyes", count: 5, level: 0, sources: ["market_watch", "read_balances"] },
    { part: "feelers", count: 4, level: 0, sources: ["quote_hunter"] },
    { part: "tendrils", count: 2, level: 0, sources: ["payees"] },
  ],
  implants: [{ implant: "vault_gem", connector: "x3_general" }],
};

const tagCount = (g: ReturnType<typeof genome>, tag: string) => g.meshes.filter((m) => m.tag === tag).length;

describe("creature genome", () => {
  it("is a pure function of traits and cosmetics", () => {
    const a = genome({ traits, cosmetics: { palette: "acid" } });
    const b = genome({ traits: structuredClone(traits), cosmetics: { palette: "acid" } });
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
  });

  it("changes when the seed (build hash) changes", () => {
    const a = genome({ traits });
    const b = genome({ traits: { ...traits, seed: traits.seed.replace(/^c/, "d") } });
    expect(JSON.stringify(a.meshes)).not.toBe(JSON.stringify(b.meshes));
  });

  it("grows the parts the skills compiled to", () => {
    const g = genome({ traits });
    expect(tagCount(g, "eyes")).toBe(5 * 3); // sclera, iris, pupil
    expect(tagCount(g, "feelers")).toBe(4 * 2);
    expect(tagCount(g, "tendrils")).toBe(2 * 2);
    expect(tagCount(g, "risk_veins")).toBe(3);
    expect(tagCount(g, "implant:x3_general")).toBeGreaterThan(0);
  });

  it("keeps other parts stable when one part is added", () => {
    const a = genome({ traits });
    const b = genome({ traits: { ...traits, parts: [...traits.parts, { part: "spikes", count: 6, level: 0, sources: ["deployer"] }] } });
    const eyes = (g: ReturnType<typeof genome>) => JSON.stringify(g.meshes.filter((m) => m.tag === "eyes"));
    expect(eyes(a)).toBe(eyes(b));
  });

  it("cosmetics change colours, never parts", () => {
    const a = genome({ traits, cosmetics: { palette: "acid" } });
    const b = genome({ traits, cosmetics: { palette: "void", glow: "gold" } });
    expect(JSON.stringify(a.meshes)).toBe(JSON.stringify(b.meshes));
    expect(a.palette.skin).not.toBe(b.palette.skin);
  });

  it("a revoked agent is grey, cracked and has no live implants", () => {
    const g = genome({ traits, revoked: true, implantStates: { x3_general: "live" } });
    expect(g.revoked).toBe(true);
    expect(tagCount(g, "crack")).toBeGreaterThan(0);
    expect(tagCount(g, "risk_veins")).toBe(0);
    expect(g.meshes.filter((m) => m.tag === "implant:x3_general").every((m) => m.role !== "glow")).toBe(true);
  });

  it("a revoked connection darkens only its implant", () => {
    const live = genome({ traits, implantStates: { x3_general: "live" } });
    const dead = genome({ traits, implantStates: { x3_general: "dead" } });
    const roles = (g: ReturnType<typeof genome>) => g.meshes.filter((m) => m.tag === "implant:x3_general").map((m) => m.role);
    expect(roles(live)).not.toEqual(roles(dead));
    expect(tagCount(dead, "implant:x3_general:crack")).toBe(1);
  });

  it("a mesh mind grows a crown and a mind-link conduit", () => {
    const t: Traits = { ...traits, parts: [...traits.parts, { part: "crown", count: 5, level: 0, sources: ["mesh_mind"] }],
      implants: [...traits.implants, { implant: "mind_link", connector: "gpu_mesh" }] };
    const live = genome({ traits: t, implantStates: { gpu_mesh: "live" } });
    expect(tagCount(live, "crown")).toBe(1 + 5); // halo + one node per crown point
    expect(tagCount(live, "implant:gpu_mesh")).toBe(3);
    const dead = genome({ traits: t, implantStates: { gpu_mesh: "dead" } });
    expect(tagCount(dead, "implant:gpu_mesh:crack")).toBe(1);
  });
});
