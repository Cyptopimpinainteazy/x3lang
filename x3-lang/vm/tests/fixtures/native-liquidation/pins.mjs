import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const root = path.dirname(fileURLToPath(import.meta.url));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function verifySourcePins() {
  const bytes = readFileSync(path.join(root, "source-pins.json"));
  const pins = JSON.parse(bytes), sources = {};
  for (const [name, pin] of Object.entries(pins.sources)) {
    const sourcePath = path.resolve(root, name);
    if (!sourcePath.startsWith(root + path.sep)) throw new Error("source pin escapes fixture directory");
    const content = readFileSync(sourcePath);
    if (sha256(content) !== pin.sha256) throw new Error(`source hash mismatch: ${name}`);
    if (name.endsWith(".sol")) sources[name] = { content: content.toString("utf8") };
  }
  return { pins, sources, manifestHash: sha256(bytes) };
}

export function loadArtifacts() {
  const { pins, manifestHash } = verifySourcePins();
  const artifact = JSON.parse(readFileSync(path.join(root, "out/artifacts.json"), "utf8"));
  if (artifact.compiler !== pins.compiler.version || artifact.source_manifest_sha256 !== manifestHash) {
    throw new Error("stale fixture artifacts: run npm run build");
  }
  return artifact;
}
