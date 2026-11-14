import QRCode from "qrcode";
import * as THREE from "three";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import type { AgentRecord } from "./schema";

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = src;
  });
}

/** The agent's identity card as a PNG: render, name, status, stats, hashes and a QR code to its profile page. */
export async function renderCard(a: AgentRecord, creatureCanvas: HTMLCanvasElement | null, profileUrl: string): Promise<Blob> {
  const W = 900, H = 1260;
  const c = document.createElement("canvas");
  c.width = W;
  c.height = H;
  const ctx = c.getContext("2d")!;
  const revoked = a.status !== "active";
  const accent = revoked ? "#7b8088" : "#38f2ff";
  const bg = ctx.createLinearGradient(0, 0, W, H);
  bg.addColorStop(0, "#071019");
  bg.addColorStop(1, "#0b0614");
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);
  ctx.strokeStyle = accent;
  ctx.lineWidth = 3;
  roundRect(ctx, 18, 18, W - 36, H - 36, 26);
  ctx.stroke();

  ctx.fillStyle = accent;
  ctx.font = "600 22px ui-sans-serif, system-ui";
  ctx.fillText("X3 AGENT FORGE · IDENTITY CARD", 52, 72);
  ctx.fillStyle = revoked ? "#ff3d5a" : "#a8ff3d";
  ctx.textAlign = "right";
  ctx.fillText(a.status.toUpperCase(), W - 52, 72);
  ctx.textAlign = "left";

  if (creatureCanvas) {
    const img = await loadImage(creatureCanvas.toDataURL("image/png"));
    ctx.save();
    roundRect(ctx, 52, 96, W - 104, 520, 18);
    ctx.clip();
    const scale = Math.max((W - 104) / img.width, 520 / img.height);
    const iw = img.width * scale, ih = img.height * scale;
    if (revoked) ctx.filter = "grayscale(1)";
    ctx.drawImage(img, 52 + (W - 104 - iw) / 2, 96 + (520 - ih) / 2, iw, ih);
    ctx.restore();
  }
  ctx.fillStyle = "#f1f5f9";
  ctx.font = "700 54px ui-sans-serif, system-ui";
  ctx.fillText(a.name, 52, 690);
  const n = a.compile.normalized as { archetype: string; identity: { owner: string; agent: string } };
  ctx.fillStyle = "#94a3b8";
  ctx.font = "400 22px ui-sans-serif, system-ui";
  ctx.fillText(`${n.archetype} · ${a.compile.traits.body} · agent ${a.agent_id}`, 52, 728);
  ctx.fillText(`owner ${n.identity.owner} · born ${new Date(a.born_at * 1000).toISOString().slice(0, 10)}`, 52, 760);

  const stats = a.compile.stats;
  const keys = ["power", "reach", "autonomy", "risk", "speed"] as const;
  keys.forEach((k, i) => {
    const y = 812 + i * 44;
    ctx.fillStyle = "#94a3b8";
    ctx.font = "600 18px ui-sans-serif, system-ui";
    ctx.fillText(k.toUpperCase(), 52, y);
    ctx.fillStyle = "#1e293b";
    roundRect(ctx, 200, y - 14, 360, 14, 7);
    ctx.fill();
    ctx.fillStyle = k === "risk" ? "#ff3d5a" : accent;
    roundRect(ctx, 200, y - 14, Math.max(14, (360 * stats[k]) / 100), 14, 7);
    ctx.fill();
    ctx.fillStyle = "#e2e8f0";
    ctx.fillText(String(stats[k]), 576, y);
  });

  const qr = await QRCode.toDataURL(profileUrl, { margin: 1, width: 230, color: { dark: "#05070b", light: "#e8f6ff" } });
  ctx.drawImage(await loadImage(qr), W - 52 - 230, 790, 230, 230);

  ctx.fillStyle = "#64748b";
  ctx.font = "400 16px ui-monospace, monospace";
  const lines = [
    `build ${a.compile.build_hash}`,
    `mandate src ${a.compile.source_sha256 ?? "none (no spending skill)"}`,
    `runtime ${a.runtime_id.slice(0, 32)}…`,
    `operator sig ${a.genesis.signature.slice(0, 48)}…`,
    `expires ${a.compile.compiled?.expires ?? new Date(a.compile.manifest.expires_at * 1000).toISOString()}`,
  ];
  lines.forEach((l, i) => ctx.fillText(l, 52, 1080 + i * 26));
  ctx.fillStyle = accent;
  ctx.font = "400 16px ui-sans-serif, system-ui";
  ctx.fillText(profileUrl, 52, H - 40);
  return new Promise((resolve, reject) => c.toBlob((b) => (b ? resolve(b) : reject(new Error("card render failed"))), "image/png"));
}

export async function exportGlb(group: THREE.Object3D): Promise<ArrayBuffer> {
  const exporter = new GLTFExporter();
  const out = await exporter.parseAsync(group, { binary: true });
  return out as ArrayBuffer;
}
