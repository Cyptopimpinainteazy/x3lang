// Rebuild only these pinned public sources; no checkout or unpinned import is consulted.
import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";
import solc from "solc";
import { root, verifySourcePins } from "./pins.mjs";

export function buildArtifacts() {
  const { pins, sources, manifestHash } = verifySourcePins();
  if (solc.version() !== pins.compiler.version) throw new Error("pinned Solidity compiler version mismatch");
  const output = JSON.parse(solc.compile(JSON.stringify({
    language: "Solidity",
    sources,
    settings: {
      optimizer: pins.compiler.optimizer,
      evmVersion: pins.compiler.evmVersion,
      outputSelection: { "*": { "*": ["abi", "evm.bytecode.object", "evm.deployedBytecode.object"] } },
    },
  })));
  const errors = (output.errors ?? []).filter((item) => item.severity === "error");
  if (errors.length) throw new Error(errors.map((item) => item.formattedMessage).join("\n"));
  const contracts = {};
  for (const [file, names] of Object.entries(output.contracts)) {
    for (const [name, value] of Object.entries(names)) {
      if (contracts[name]) throw new Error(`ambiguous fixture contract name: ${name}`);
      contracts[name] = {
        source: file, abi: value.abi,
        bytecode: "0x" + value.evm.bytecode.object,
        runtime_bytecode: "0x" + value.evm.deployedBytecode.object,
      };
    }
  }
  const artifact = {
    schema: "x3.native-liquidation.fixture-artifacts.v1",
    compiler: solc.version(), source_manifest_sha256: manifestHash,
    contracts,
  };
  mkdirSync(path.join(root, "out"), { recursive: true });
  const temporary = path.join(root, `out/artifacts.${process.pid}.tmp`);
  writeFileSync(temporary, JSON.stringify(artifact) + "\n", { flag: "wx" });
  renameSync(temporary, path.join(root, "out/artifacts.json"));
  return artifact;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const output = buildArtifacts();
  console.log(`Built ${Object.keys(output.contracts).length} contracts with ${output.compiler}; source pins verified.`);
}
