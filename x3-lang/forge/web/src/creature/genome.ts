// Procedural alien genome. A pure function of the compiled traits (which the host derives from
// the normalized build) plus cosmetics: the same build always grows the same creature, and a
// different mandate grows a different one. Every random draw comes from a per-part stream seeded
// by the build hash, so adding one skill doesn't reshuffle the rest of the body.
import type { Cosmetics, Traits } from "../schema";

export type Vec3 = [number, number, number];
export type Role = "skin" | "skin2" | "plate" | "eye" | "pupil" | "glow" | "danger" | "metal" | "dark" | "shell";
export type ImplantState = "live" | "dim" | "dead";

export interface Mesh {
  kind: "sphere" | "cone" | "tube" | "torus" | "octa" | "ico" | "box" | "cylinder";
  pos: Vec3;
  scale: Vec3;
  rot?: Vec3;
  role: Role;
  emissive?: number; // 0..2 intensity multiplier
  points?: Vec3[]; // tube path
  radius?: number; // tube radius
  wire?: boolean;
  opacity?: number;
  pulse?: number; // phase offset for animated glow, if any
  tag: string; // which trait or implant produced it
}

export interface Palette { skin: string; skin2: string; plate: string; eye: string; pupil: string; glow: string; danger: string; metal: string; dark: string; shell: string }
export interface Genome { body: Traits["body"]; meshes: Mesh[]; palette: Palette; revoked: boolean; idle: string; height: number }

// ---------------------------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------------------------

function hash32(text: string): number {
  // FNV-1a
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

export function rng(seed: string, stream: string): () => number {
  let a = hash32(seed + "|" + stream);
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const r2 = (x: number) => Math.round(x * 1000) / 1000;
const v = (x: number, y: number, z: number): Vec3 => [r2(x), r2(y), r2(z)];
const add = (a: Vec3, b: Vec3): Vec3 => v(a[0] + b[0], a[1] + b[1], a[2] + b[2]);
const mul = (a: Vec3, s: number): Vec3 => v(a[0] * s, a[1] * s, a[2] * s);
function norm(a: Vec3): Vec3 {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return v(a[0] / l, a[1] / l, a[2] / l);
}

// ---------------------------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------------------------

const PALETTES: Record<string, { hue: number; sat: number; light: number }> = {
  acid: { hue: 115, sat: 55, light: 38 },
  void: { hue: 265, sat: 40, light: 30 },
  ember: { hue: 18, sat: 60, light: 36 },
  abyss: { hue: 195, sat: 55, light: 30 },
  aurora: { hue: 160, sat: 45, light: 40 },
  bone: { hue: 40, sat: 18, light: 58 },
};
const GLOWS: Record<string, string> = {
  cyan: "#38f2ff", magenta: "#ff3df2", lime: "#a8ff3d", gold: "#ffcc33", violet: "#a77bff", white: "#e8f6ff",
};
export const PALETTE_NAMES = Object.keys(PALETTES);
export const GLOW_NAMES = Object.keys(GLOWS);
export const PATTERN_NAMES = ["smooth", "spots", "stripes"];
export const IDLE_NAMES = ["breathe", "sway", "hover"];

const hsl = (h: number, s: number, l: number) => `hsl(${Math.round(((h % 360) + 360) % 360)}, ${Math.round(s)}%, ${Math.round(l)}%)`;

function palette(seed: string, c: Cosmetics, revoked: boolean): Palette {
  const base = PALETTES[c.palette ?? "acid"] ?? PALETTES.acid;
  const r = rng(seed, "palette");
  const hue = base.hue + (r() - 0.5) * 30;
  if (revoked) {
    return { skin: "#3a3d42", skin2: "#2b2e33", plate: "#4a4d52", eye: "#5b5f66", pupil: "#111214", glow: "#53575e",
             danger: "#4a3a3a", metal: "#45484d", dark: "#0b0c0e", shell: "#55595f" };
  }
  return {
    skin: hsl(hue, base.sat, base.light),
    skin2: hsl(hue + 25, base.sat + 10, base.light + 12),
    plate: hsl(hue - 20, base.sat - 15, base.light - 14),
    eye: "#f4f0d8",
    pupil: "#07080a",
    glow: GLOWS[c.glow ?? "cyan"] ?? GLOWS.cyan,
    danger: "#ff2a3d",
    metal: "#9aa7b4",
    dark: "#0a0c10",
    shell: hsl(hue + 180, 70, 60),
  };
}

// ---------------------------------------------------------------------------------------------
// Body plans: body meshes plus anchors where parts attach
// ---------------------------------------------------------------------------------------------

interface Anchors {
  head: Vec3; headR: number; chest: Vec3; neck: Vec3; spine: Vec3[]; belly: Vec3[]; sides: Vec3[]; back: Vec3;
  ground: number; spineR: number[];
}

function plan(body: Traits["body"], seed: string, meshes: Mesh[]): Anchors {
  const r = rng(seed, "body");
  const tag = "body";
  switch (body) {
    case "serpent": {
      const n = 7 + Math.floor(r() * 4);
      const amp = 0.25 + r() * 0.2;
      const spine: Vec3[] = [];
      const spineR: number[] = [];
      for (let i = 0; i < n; i++) {
        const t = i / (n - 1);
        const x = -2.3 + t * 3.3;
        const y = -0.55 + t * t * 0.9;
        const z = Math.sin(t * Math.PI * 2 + r() * 0.3) * amp;
        const rad = 0.16 + t * 0.32;
        spine.push(v(x, y, z));
        spineR.push(r2(rad));
        meshes.push({ kind: "sphere", pos: v(x, y, z), scale: v(rad * 1.25, rad, rad), role: i % 2 ? "skin" : "skin2", tag });
      }
      const neck = spine[n - 1];
      const headR = 0.52 + r() * 0.12;
      const head = add(neck, v(0.45, 0.38, 0.1));
      meshes.push({ kind: "sphere", pos: head, scale: v(headR * 1.15, headR, headR * 1.05), role: "skin", tag });
      meshes.push({ kind: "sphere", pos: add(head, v(0.32, -0.22, 0.12)), scale: v(headR * 0.6, headR * 0.38, headR * 0.6), role: "skin2", tag: "jaw" });
      return { head, headR, chest: spine[n - 2], neck, spine, belly: spine.slice(1, n - 1).map((p) => add(p, v(0, -0.2, 0))),
               sides: spine.slice(2, n - 1), back: spine[Math.floor(n / 2)], ground: -0.85, spineR };
    }
    case "crustacean": {
      const w = 1.25 + r() * 0.3;
      meshes.push({ kind: "sphere", pos: v(0, 0, 0), scale: v(w, 0.5 + r() * 0.12, 0.9), role: "skin", tag });
      meshes.push({ kind: "sphere", pos: v(0, -0.12, -0.55), scale: v(w * 0.7, 0.35, 0.6), role: "skin2", tag });
      const headR = 0.38 + r() * 0.08;
      const head = v(0, 0.22, 0.82);
      meshes.push({ kind: "sphere", pos: head, scale: v(headR * 1.3, headR, headR), role: "skin", tag });
      for (let i = 0; i < 6; i++) {
        const side = i < 3 ? -1 : 1;
        const zz = -0.45 + (i % 3) * 0.45;
        const hip = v(side * w * 0.8, -0.1, zz);
        const knee = v(side * (w + 0.45), 0.25, zz + (r() - 0.5) * 0.2);
        const foot = v(side * (w + 0.65), -0.85, zz + (r() - 0.5) * 0.3);
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [hip, knee, foot], radius: 0.07, role: "plate", tag: "legs" });
      }
      const spine = [v(-0.6, 0.45, -0.3), v(0, 0.52, 0), v(0.6, 0.45, -0.3), v(0, 0.45, -0.6)];
      return { head, headR, chest: v(0, 0.05, 0.85), neck: v(0, 0.2, 0.62), spine, spineR: spine.map(() => 0.45),
               belly: [v(-0.5, -0.45, 0.2), v(0.5, -0.45, 0.2), v(0, -0.45, -0.3)], sides: [v(-w, 0.05, 0.1), v(w, 0.05, 0.1)],
               back: v(0, 0.5, -0.25), ground: -0.85 };
    }
    case "insectoid": {
      const headR = 0.32 + r() * 0.06;
      const head = v(0, 1.0, 0.7);
      const thorax = v(0, 0.55, 0.05);
      const abdomen = v(0, 0.42, -0.95);
      meshes.push({ kind: "sphere", pos: head, scale: v(headR * 1.25, headR, headR), role: "skin", tag });
      meshes.push({ kind: "sphere", pos: thorax, scale: v(0.38, 0.42, 0.5), role: "skin2", tag });
      meshes.push({ kind: "sphere", pos: abdomen, scale: v(0.55, 0.48, 0.95 + r() * 0.2), role: "skin", tag });
      for (let i = 0; i < 6; i++) {
        const side = i % 2 ? 1 : -1;
        const zz = 0.25 - Math.floor(i / 2) * 0.28;
        const hip = v(side * 0.3, 0.45, zz);
        const knee = v(side * 0.95, 0.75, zz + 0.15);
        const foot = v(side * 1.2, -0.85, zz + (r() - 0.3) * 0.5);
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [hip, knee, foot], radius: 0.04, role: "plate", tag: "legs" });
      }
      meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [add(head, v(0, -0.35, -0.2)), thorax], radius: 0.13, role: "skin2", tag });
      const spine = [thorax, add(abdomen, v(0, 0.3, 0.5)), add(abdomen, v(0, 0.4, 0)), add(abdomen, v(0, 0.35, -0.5))];
      return { head, headR, chest: add(thorax, v(0, 0, 0.42)), neck: add(head, v(0, -0.3, -0.3)), spine, spineR: [0.4, 0.5, 0.5, 0.45],
               belly: [add(abdomen, v(0, -0.4, 0.2)), add(abdomen, v(0, -0.42, -0.3))], sides: [add(abdomen, v(-0.5, 0.1, 0)), add(abdomen, v(0.5, 0.1, 0))],
               back: add(thorax, v(0, 0.35, -0.1)), ground: -0.85 };
    }
    case "cephalopod": {
      const mantle = v(0, 0.85, -0.15);
      meshes.push({ kind: "sphere", pos: mantle, scale: v(0.85, 1.05 + r() * 0.2, 0.85), role: "skin", tag });
      const headR = 0.55;
      const head = v(0, 0.2, 0.25);
      meshes.push({ kind: "sphere", pos: head, scale: v(0.62, 0.5, 0.6), role: "skin2", tag });
      const n = 8;
      for (let i = 0; i < n; i++) {
        const a = (i / n) * Math.PI * 2 + r() * 0.2;
        const curl = 0.4 + r() * 0.5;
        const pts: Vec3[] = [];
        for (let k = 0; k < 5; k++) {
          const t = k / 4;
          const rad = 0.35 + t * (0.6 + curl * 0.4);
          pts.push(v(Math.cos(a + t * curl) * rad, -0.05 - t * 0.85 + Math.sin(t * 3 + a) * 0.08, 0.2 + Math.sin(a + t * curl) * rad));
        }
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: pts, radius: 0.07, role: i % 2 ? "skin" : "skin2", tag: "tentacles" });
      }
      const spine = [add(mantle, v(0, 0.95, -0.1)), add(mantle, v(0, 0.6, -0.55)), add(mantle, v(0, 0.1, -0.75))];
      return { head: add(head, v(0, 0.25, 0.1)), headR, chest: add(head, v(0, -0.1, 0.55)), neck: add(head, v(0, -0.1, 0)), spine,
               spineR: [0.4, 0.55, 0.55], belly: [v(-0.3, -0.25, 0.2), v(0.3, -0.25, 0.2), v(0, -0.25, -0.2)],
               sides: [add(mantle, v(-0.82, 0, 0)), add(mantle, v(0.82, 0, 0))], back: add(mantle, v(0, 0.3, -0.7)), ground: -0.9 };
    }
    case "titan":
    default: {
      const torso = v(0, 0.35, 0);
      const bulk = 0.95 + r() * 0.2;
      meshes.push({ kind: "sphere", pos: torso, scale: v(bulk, 1.05, 0.65), role: "skin", tag });
      meshes.push({ kind: "sphere", pos: v(0, -0.35, 0), scale: v(bulk * 0.75, 0.45, 0.55), role: "skin2", tag });
      const headR = 0.34;
      const head = v(0, 1.55, 0.18);
      meshes.push({ kind: "sphere", pos: head, scale: v(headR * 1.1, headR, headR), role: "skin2", tag });
      for (const side of [-1, 1]) {
        const shoulder = v(side * bulk * 0.95, 0.85, 0);
        meshes.push({ kind: "sphere", pos: shoulder, scale: v(0.32, 0.32, 0.32), role: "plate", tag });
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [shoulder, v(side * (bulk + 0.4), 0.2, 0.25), v(side * (bulk + 0.35), -0.35, 0.45)], radius: 0.16, role: "skin", tag: "arms" });
        meshes.push({ kind: "sphere", pos: v(side * (bulk + 0.35), -0.42, 0.48), scale: v(0.2, 0.2, 0.2), role: "plate", tag: "arms" });
        meshes.push({ kind: "cylinder", pos: v(side * 0.42, -0.72, 0), scale: v(0.24, 0.55, 0.24), role: "plate", tag: "legs" });
      }
      const spine = [v(0, 1.05, -0.45), v(0, 0.6, -0.6), v(0, 0.1, -0.6)];
      return { head, headR, chest: v(0, 0.6, 0.62), neck: v(0, 1.25, 0.05), spine, spineR: [0.6, 0.65, 0.6],
               belly: [v(-0.35, -0.65, 0.35), v(0.35, -0.65, 0.35)], sides: [v(-bulk, 0.4, 0), v(bulk, 0.4, 0)], back: v(0, 0.7, -0.65), ground: -1.27 };
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Parts and implants
// ---------------------------------------------------------------------------------------------

function onHead(a: Anchors, dir: Vec3, out = 1.0): Vec3 {
  return add(a.head, mul(norm(dir), a.headR * out));
}

function growPart(part: string, count: number, level: number, a: Anchors, seed: string, meshes: Mesh[]) {
  const r = rng(seed, `part:${part}`);
  const tag = part;
  switch (part) {
    case "eyes": {
      const n = Math.min(count, 12);
      for (let i = 0; i < n; i++) {
        const ring = i === 0 ? 0 : 1 + Math.floor((i - 1) / 5);
        const ang = ((i - 1) % 5) / 5 * Math.PI * 2 + ring * 0.6 + r() * 0.2;
        const spread = ring * 0.38;
        const dir: Vec3 = n === 1 ? v(0.25, 0.15, 1) : v(Math.cos(ang) * spread + 0.15, Math.sin(ang) * spread + 0.2, 1);
        const p = onHead(a, dir, 0.92);
        const s = a.headR * (i === 0 ? 0.3 : 0.2 - ring * 0.03) * (0.85 + r() * 0.3);
        meshes.push({ kind: "sphere", pos: p, scale: v(s, s, s), role: "eye", tag });
        meshes.push({ kind: "sphere", pos: add(p, mul(norm(dir), s * 0.62)), scale: v(s * 0.5, s * 0.5, s * 0.5), role: "glow", emissive: 1.4, tag, pulse: r() * 6 });
        meshes.push({ kind: "sphere", pos: add(p, mul(norm(dir), s * 0.82)), scale: v(s * 0.24, s * 0.24, s * 0.24), role: "pupil", tag });
      }
      return;
    }
    case "brain_lobes": {
      for (let i = 0; i < Math.min(count, 6); i++) {
        const ang = (i / Math.max(count, 1)) * Math.PI - Math.PI / 2;
        const p = onHead(a, v(Math.sin(ang) * 0.7, 1, -0.3 + Math.cos(ang) * 0.3), 0.75);
        const s = a.headR * (0.45 + r() * 0.15);
        meshes.push({ kind: "sphere", pos: p, scale: v(s, s * 0.85, s), role: "skin2", tag });
        meshes.push({ kind: "sphere", pos: add(p, v(0, s * 0.3, 0)), scale: v(s * 0.5, s * 0.4, s * 0.5), role: "glow", emissive: 0.6, opacity: 0.55, tag, pulse: i });
      }
      return;
    }
    case "feelers": {
      for (let i = 0; i < Math.min(count, 8); i++) {
        const side = i % 2 ? 1 : -1;
        const k = Math.floor(i / 2);
        const base = onHead(a, v(side * 0.55, -0.35 - k * 0.12, 0.8), 0.9);
        const len = 0.6 + r() * 0.5;
        const pts = [base, add(base, v(side * 0.15, -0.15, len * 0.5)), add(base, v(side * (0.3 + r() * 0.2), -0.45 - r() * 0.2, len))];
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: pts, radius: 0.025, role: "skin2", tag });
        meshes.push({ kind: "sphere", pos: pts[2], scale: v(0.045, 0.045, 0.045), role: "glow", emissive: 1.2, tag, pulse: i });
      }
      return;
    }
    case "antennae": {
      for (let i = 0; i < Math.min(count, 10); i++) {
        const side = i % 2 ? 1 : -1;
        const k = Math.floor(i / 2);
        const base = onHead(a, v(side * (0.25 + k * 0.15), 1, -0.2 - k * 0.1), 0.95);
        const h = 0.7 + r() * 0.6 - k * 0.08;
        const tip = add(base, v(side * (0.25 + r() * 0.3 + k * 0.12), h, -0.15 - r() * 0.2));
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [base, add(base, v(side * 0.05, h * 0.6, 0.05)), tip], radius: 0.02, role: "plate", tag });
        meshes.push({ kind: "sphere", pos: tip, scale: v(0.07, 0.07, 0.07), role: "glow", emissive: 1.8, tag, pulse: i * 0.7 });
      }
      return;
    }
    case "core": {
      const s = 0.1 + 0.05 * Math.min(level, 8);
      meshes.push({ kind: "sphere", pos: a.chest, scale: v(s, s, s), role: "glow", emissive: 2, tag, pulse: 0 });
      meshes.push({ kind: "torus", pos: a.chest, scale: v(s * 1.7, s * 1.7, s * 1.7), rot: v(Math.PI / 2, 0, 0), role: "metal", tag });
      meshes.push({ kind: "sphere", pos: a.chest, scale: v(s * 1.5, s * 1.5, s * 1.5), role: "glow", emissive: 0.6, opacity: 0.25, tag, pulse: 0 });
      return;
    }
    case "tendrils": {
      for (let i = 0; i < Math.min(count, 10); i++) {
        const base = a.belly[i % a.belly.length];
        const off = v((r() - 0.5) * 0.4, 0, (r() - 0.5) * 0.4);
        const start = add(base, off);
        const end = v(start[0] + (r() - 0.5) * 0.6, a.ground + 0.05 + r() * 0.2, start[2] + 0.3 + r() * 0.4);
        const mid = v((start[0] + end[0]) / 2 + (r() - 0.5) * 0.3, (start[1] + end[1]) / 2, (start[2] + end[2]) / 2 + 0.2);
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [start, mid, end], radius: 0.035, role: "skin2", tag });
        meshes.push({ kind: "sphere", pos: end, scale: v(0.05, 0.05, 0.05), role: "glow", emissive: 1, tag, pulse: i });
      }
      return;
    }
    case "shield": {
      // A faint energy shell around the whole body: centred on the body, sized to its extent.
      const pts = [...a.spine, a.head];
      const c = mul(pts.reduce((acc, p) => add(acc, p), v(0, 0, 0)), 1 / pts.length);
      const ext = (i: 0 | 1 | 2) => Math.max(...pts.map((p) => Math.abs(p[i] - c[i])));
      meshes.push({ kind: "ico", pos: c, scale: v(ext(0) + 0.75, ext(1) + 0.8, ext(2) + 0.75), role: "shell", wire: true, opacity: 0.16, emissive: 0.6, tag, pulse: 1 });
      return;
    }
    case "egg_sacs": {
      for (let i = 0; i < Math.min(count, 9); i++) {
        const p = add(a.back, v((r() - 0.5) * 0.7, 0.15 + r() * 0.25, -0.2 - r() * 0.4));
        const s = 0.11 + r() * 0.07;
        meshes.push({ kind: "sphere", pos: p, scale: v(s, s, s), role: "shell", opacity: 0.6, emissive: 0.5, tag, pulse: i });
      }
      return;
    }
    case "carapace": {
      const n = 2 + level * 3;
      for (let i = 0; i < n; i++) {
        const spinePt = a.spine[i % a.spine.length];
        const rr = a.spineR[i % a.spineR.length] ?? 0.4;
        const p = add(spinePt, v((r() - 0.5) * rr, rr * 0.8, (r() - 0.5) * rr));
        const s = 0.18 + r() * 0.14 + level * 0.03;
        meshes.push({ kind: "octa", pos: p, scale: v(s * 1.5, s * 0.5, s * 1.2), rot: v(r() * 0.4, r() * 3, r() * 0.4), role: "plate", tag });
      }
      return;
    }
    case "sensor_fins": {
      for (let i = 0; i < Math.min(count, 6); i++) {
        const base = a.sides[i % a.sides.length];
        const side = base[0] >= 0 ? 1 : -1;
        meshes.push({ kind: "cone", pos: add(base, v(side * 0.2, 0.15 + Math.floor(i / 2) * 0.12, 0)), scale: v(0.08, 0.5 + r() * 0.2, 0.3), rot: v(0, 0, -side * 1.1), role: "skin2", tag });
        meshes.push({ kind: "sphere", pos: add(base, v(side * 0.55, 0.35, 0)), scale: v(0.04, 0.04, 0.04), role: "glow", emissive: 1.4, tag, pulse: i });
      }
      return;
    }
    case "claws": {
      for (let i = 0; i < Math.min(count, 4); i++) {
        const side = i % 2 ? 1 : -1;
        const root = add(a.chest, v(side * 0.45, -0.1 - Math.floor(i / 2) * 0.2, 0.1));
        const wrist = add(root, v(side * 0.35, 0.1, 0.55));
        meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [root, add(root, v(side * 0.3, 0.25, 0.25)), wrist], radius: 0.07, role: "plate", tag });
        meshes.push({ kind: "cone", pos: add(wrist, v(0, 0.08, 0.18)), scale: v(0.08, 0.32, 0.08), rot: v(1.3, 0, 0), role: "plate", tag });
        meshes.push({ kind: "cone", pos: add(wrist, v(0, -0.08, 0.18)), scale: v(0.07, 0.28, 0.07), rot: v(1.85, 0, 0), role: "plate", tag });
      }
      return;
    }
    case "gills": {
      for (let i = 0; i < Math.min(count, 6); i++) {
        const p = add(a.neck, v(0, -i * 0.07, -i * 0.03));
        meshes.push({ kind: "torus", pos: p, scale: v(0.3 - i * 0.02, 0.3 - i * 0.02, 0.3), rot: v(Math.PI / 2 + 0.3, 0, 0), role: "glow", emissive: 0.5, opacity: 0.7, tag, pulse: i });
      }
      return;
    }
    case "crown": {
      // A halo of thought-nodes floating above the head: one per crown point the build earned.
      const n = Math.min(count, 9);
      const top = add(a.head, v(0, a.headR * 1.25, 0));
      const rad = a.headR * (0.7 + 0.04 * n);
      meshes.push({ kind: "torus", pos: top, scale: v(rad, rad, rad * 0.6), rot: v(Math.PI / 2, 0, 0), role: "glow", emissive: 0.7, opacity: 0.6, tag, pulse: 0 });
      for (let i = 0; i < n; i++) {
        const ang = (i / n) * Math.PI * 2 + r() * 0.15;
        const p = add(top, v(Math.cos(ang) * rad, 0.04 + r() * 0.06, Math.sin(ang) * rad));
        meshes.push({ kind: "ico", pos: p, scale: v(0.06, 0.06, 0.06), role: "glow", emissive: 1.5, tag, pulse: i * 0.8 });
      }
      return;
    }
    case "spikes": {
      for (let i = 0; i < Math.min(count, 12); i++) {
        const spinePt = a.spine[i % a.spine.length];
        const rr = a.spineR[i % a.spineR.length] ?? 0.4;
        const p = add(spinePt, v((r() - 0.5) * rr * 0.6, rr * 0.95, (r() - 0.5) * 0.3));
        meshes.push({ kind: "cone", pos: p, scale: v(0.07, 0.32 + r() * 0.25, 0.07), rot: v(-0.3, 0, (r() - 0.5) * 0.5), role: "danger", emissive: 0.4, tag });
      }
      return;
    }
  }
}

function veins(tier: number, a: Anchors, seed: string, meshes: Mesh[]) {
  if (tier < 3) return;
  const r = rng(seed, "veins");
  const n = (tier - 2) * 3;
  for (let i = 0; i < n; i++) {
    const p0 = a.spine[i % a.spine.length];
    const rr = a.spineR[i % a.spineR.length] ?? 0.4;
    const ang = r() * Math.PI * 2;
    const pts: Vec3[] = [0, 1, 2].map((k) => add(p0, v(Math.cos(ang + k * 0.6) * rr * 1.02, Math.sin(ang + k * 0.6) * rr * 0.95, (k - 1) * rr * 0.6)));
    meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: pts, radius: 0.018, role: "danger", emissive: 1.6, tag: "risk_veins", pulse: i });
  }
}

function implant(kind: string, index: number, state: ImplantState, a: Anchors, tag: string, meshes: Mesh[]) {
  const em = state === "live" ? 1.8 : state === "dim" ? 0.35 : 0;
  const role: Role = state === "dead" ? "dark" : "glow";
  const o = index * 0.18;
  switch (kind) {
    case "neural_jack": {
      const p = onHead(a, v(0.2 + o, 0.4, -1), 1.0);
      meshes.push({ kind: "cylinder", pos: p, scale: v(0.07, 0.12, 0.07), rot: v(1.2, 0, 0), role: "metal", tag });
      meshes.push({ kind: "sphere", pos: add(p, v(0, 0, -0.07)), scale: v(0.05, 0.05, 0.05), role, emissive: em, tag, pulse: 2 });
      meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [add(p, v(0, 0, -0.06)), add(p, v(0.05, -0.35, -0.35)), a.back], radius: 0.022, role: state === "dead" ? "dark" : "metal", tag });
      if (state === "dead") meshes.push({ kind: "box", pos: add(p, v(0, 0.05, -0.02)), scale: v(0.12, 0.012, 0.02), rot: v(0, 0, 0.7), role: "pupil", tag: tag + ":crack" });
      return;
    }
    case "resonance": {
      for (const side of [-1, 1]) {
        const p = add(a.back, v(side * (0.45 + o), 0.35, 0.05));
        meshes.push({ kind: "sphere", pos: p, scale: v(0.085, 0.085, 0.085), role, emissive: em, tag, pulse: side > 0 ? 0 : Math.PI });
        meshes.push({ kind: "torus", pos: p, scale: v(0.13, 0.13, 0.13), rot: v(Math.PI / 2, 0, 0), role: "metal", tag });
      }
      return;
    }
    case "vault_gem": {
      const p = add(a.chest, v((index % 2 ? 1 : -1) * Math.ceil(index / 2) * 0.22, 0.18, 0.06));
      meshes.push({ kind: "octa", pos: p, scale: v(0.11, 0.16, 0.11), role: state === "dead" ? "dark" : "shell", emissive: em, tag, pulse: 3 });
      if (state === "dead") meshes.push({ kind: "box", pos: p, scale: v(0.2, 0.012, 0.02), rot: v(0, 0, -0.6), role: "pupil", tag: tag + ":crack" });
      return;
    }
    case "chain_sigil": {
      const side = a.sides[index % a.sides.length];
      const s = side[0] >= 0 ? 1 : -1;
      meshes.push({ kind: "torus", pos: add(side, v(s * 0.12, 0.1 + o, 0.12)), scale: v(0.16, 0.16, 0.16), rot: v(0, Math.PI / 2, 0), role, emissive: em, tag, pulse: 1 });
      return;
    }
    case "mind_link": {
      // A conduit from the back of the skull up to the crown: lit while the GPU mesh answers.
      const p = onHead(a, v(-0.2 - o, 0.75, -0.7), 1.0);
      const up = add(a.head, v(-0.1 - o, a.headR * 1.25, -0.15));
      meshes.push({ kind: "cylinder", pos: p, scale: v(0.06, 0.1, 0.06), rot: v(0.9, 0, 0), role: "metal", tag });
      meshes.push({ kind: "tube", pos: v(0, 0, 0), scale: v(1, 1, 1), points: [p, add(p, v(0, 0.18, -0.1)), up], radius: 0.018, role, emissive: em, tag, pulse: 4 });
      meshes.push({ kind: "ico", pos: up, scale: v(0.075, 0.075, 0.075), role, emissive: em, tag, pulse: 4 });
      if (state === "dead") meshes.push({ kind: "box", pos: add(p, v(0, 0.05, 0)), scale: v(0.1, 0.012, 0.02), rot: v(0, 0, 0.7), role: "pupil", tag: tag + ":crack" });
      return;
    }
    case "sensor_port":
    default: {
      const p = add(a.spine[index % a.spine.length], v(0, (a.spineR[index % a.spineR.length] ?? 0.4) + 0.08, 0));
      meshes.push({ kind: "cylinder", pos: p, scale: v(0.06, 0.16, 0.06), role: "metal", tag });
      meshes.push({ kind: "sphere", pos: add(p, v(0, 0.1, 0)), scale: v(0.05, 0.05, 0.05), role, emissive: em, tag, pulse: index });
    }
  }
}

function pattern(kind: string, a: Anchors, seed: string, meshes: Mesh[]) {
  const r = rng(seed, `pattern:${kind}`);
  if (kind === "spots") {
    for (let i = 0; i < 14; i++) {
      const sp = a.spine[i % a.spine.length];
      const rr = a.spineR[i % a.spineR.length] ?? 0.4;
      const ang = r() * Math.PI * 2;
      meshes.push({ kind: "sphere", pos: add(sp, v(Math.cos(ang) * rr * 0.97, Math.abs(Math.sin(ang)) * rr * 0.9, (r() - 0.5) * rr)), scale: v(0.06, 0.03, 0.06), role: "skin2", tag: "pattern" });
    }
  } else if (kind === "stripes") {
    a.spine.forEach((sp, i) => {
      const rr = a.spineR[i] ?? 0.4;
      meshes.push({ kind: "torus", pos: sp, scale: v(rr * 1.01, rr * 1.01, rr * 0.6), rot: v(0, Math.PI / 2, 0), role: "plate", tag: "pattern" });
    });
  }
}

function cracks(a: Anchors, seed: string, meshes: Mesh[]) {
  const r = rng(seed, "cracks");
  for (let i = 0; i < 10; i++) {
    const sp = a.spine[i % a.spine.length];
    const rr = a.spineR[i % a.spineR.length] ?? 0.4;
    meshes.push({ kind: "box", pos: add(sp, v((r() - 0.5) * rr, rr * 0.9, (r() - 0.5) * rr)), scale: v(0.3 + r() * 0.3, 0.015, 0.03), rot: v(r(), r() * 3, r()), role: "pupil", tag: "crack" });
  }
  meshes.push({ kind: "box", pos: add(a.head, v(0.1, a.headR * 0.6, 0.2)), scale: v(0.4, 0.02, 0.03), rot: v(0.3, 0.5, 0.8), role: "pupil", tag: "crack" });
}

export interface GenomeInput {
  traits: Traits;
  cosmetics?: Cosmetics;
  implantStates?: Record<string, ImplantState>; // connector id → state
  revoked?: boolean;
}

export function genome({ traits, cosmetics = {}, implantStates = {}, revoked = false }: GenomeInput): Genome {
  const meshes: Mesh[] = [];
  const seed = traits.seed;
  const a = plan(traits.body, seed, meshes);
  for (const p of [...traits.parts].sort((x, y) => x.part.localeCompare(y.part))) {
    growPart(p.part, p.count, p.level, a, seed, meshes);
  }
  veins(revoked ? 0 : traits.risk_tier, a, seed, meshes);
  const counts: Record<string, number> = {};
  for (const im of [...traits.implants].sort((x, y) => x.connector.localeCompare(y.connector))) {
    const idx = counts[im.implant] ?? 0;
    counts[im.implant] = idx + 1;
    const state: ImplantState = revoked ? "dead" : implantStates[im.connector] ?? "dim";
    implant(im.implant, idx, state, a, `implant:${im.connector}`, meshes);
  }
  pattern(cosmetics.pattern ?? "smooth", a, seed, meshes);
  if (revoked) cracks(a, seed, meshes);
  const ys = meshes.map((m) => m.pos[1]);
  return { body: traits.body, meshes, palette: palette(seed, cosmetics, revoked), revoked, idle: cosmetics.idle ?? "breathe",
           height: r2(Math.max(...ys) - Math.min(...ys, a.ground)) };
}

export function implantStateOf(status: string | undefined, health?: string | null): ImplantState {
  if (status === "revoked" || status === "failed") return "dead";
  if (status === "linked" && (health === "ok" || health === "login_gated" || health === undefined)) return "live";
  return "dim";
}
