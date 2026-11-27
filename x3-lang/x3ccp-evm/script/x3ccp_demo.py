"""X3CCP end-to-end demo: deploy, install an X3Lang policy, then show one allowed and two refused
withdrawals mined on chain, each checked against the predictor.

    # Local anvil chain (no keys, no public network). Safe to run anywhere:
    python3 x3-lang/x3ccp-evm/script/x3ccp_demo.py --local

    # Base Sepolia (chain 84532). Sends REAL testnet transactions from Foundry keystore accounts.
    # Nothing is sent without both flags; cast prompts for each keystore password itself.
    python3 x3-lang/x3ccp-evm/script/x3ccp_demo.py --base-sepolia \\
        --rpc-url https://sepolia.base.org \\
        --owner-account x3-owner --agent-account x3-agent \\
        --recipient 0x... --broadcast --i-approve-testnet-transactions

Keys never pass through this script: it calls `forge create` / `cast send --account <name>`,
and Foundry reads its own encrypted keystore. A refused call is sent with a fixed gas limit so it
is mined and visible on the explorer as a failed transaction; its revert reason is replayed with
eth_call against the parent block (public RPCs do not offer debug tracing).

Output: a JSON run record (`--out`). A --local record is a local EVM run, not public-chain evidence.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
EVM = os.path.dirname(HERE)
sys.path.insert(0, os.path.dirname(EVM))

import x3ccp  # noqa: E402

ANVIL_DEPLOYER = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
LOCAL_AGENT = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
LOCAL_RECIPIENT = "0x2222222222222222222222222222222222222222"
OUTSIDER = "0x6666666666666666666666666666666666666666"

DEPOSIT_WEI = 100_000_000_000_000  # 0.0001 ETH
MAX_WEI = 20_000_000_000_000  # 0.00002 ETH per withdrawal
ALLOWED_WEI = 10_000_000_000_000
TOO_MUCH_WEI = 30_000_000_000_000
FORCED_GAS = 300_000

POLICY_TEMPLATE = """control x3ccp_demo_treasury {{
    target {chain} {target}
    owner {owner}
    allow callers {agent}
    allow fn withdraw(address to, uint256 amount)
    limit withdraw.amount <= {max_wei}
    limit withdraw.to in {recipient}
    rate withdraw calls per 1h <= 3
    expires 7d
}}
"""


def _tool(name):
    for c in (shutil.which(name), f"/opt/foundry/{name}", os.path.expanduser(f"~/.foundry/bin/{name}")):
        if c and os.path.exists(c):
            return c
    sys.exit(f"{name} not found; install Foundry")


class Rpc:
    def __init__(self, url):
        self.url, self.n = url, 0

    def __call__(self, method, *params):
        self.n += 1
        req = urllib.request.Request(self.url, json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method,
                                                           "params": list(params)}).encode(),
                                     {"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=60) as resp:
            out = json.loads(resp.read())
        if "error" in out:
            raise RuntimeError(f"{method}: {out['error']}")
        return out["result"]

    def receipt(self, tx_hash):
        for _ in range(240):
            r = self("eth_getTransactionReceipt", tx_hash)
            if r:
                return r
            time.sleep(0.5)
        raise RuntimeError(f"{tx_hash} not mined")


def _run(cmd):
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd[:3])} failed: {out.stderr.strip()[-400:]}")
    return out.stdout


class Chain:
    """Sends transactions either as anvil-unlocked accounts (local) or via Foundry keystores."""

    def __init__(self, args, rpc_url):
        self.args, self.rpc_url, self.rpc = args, rpc_url, Rpc(rpc_url)
        self.cast, self.forge = _tool("cast"), _tool("forge")

    def _who(self, role):
        if self.args.local:
            return ["--unlocked", "--from", ANVIL_DEPLOYER if role == "owner" else LOCAL_AGENT]
        return ["--account", self.args.owner_account if role == "owner" else self.args.agent_account]

    def address(self, role):
        if self.args.local:
            return ANVIL_DEPLOYER if role == "owner" else LOCAL_AGENT
        name = self.args.owner_account if role == "owner" else self.args.agent_account
        return x3ccp.to_checksum(_run([self.cast, "wallet", "address", "--account", name]).strip())

    def deploy(self):
        cmd = [self.forge, "create", "src/X3ControlledTreasury.sol:X3ControlledTreasury", "--rpc-url",
               self.rpc_url, "--broadcast", "--json", *self._who("owner")]
        out = subprocess.run(cmd, cwd=EVM, capture_output=True, text=True)
        if out.returncode != 0:
            raise RuntimeError(f"forge create failed: {out.stderr.strip()[-400:]}")
        text = out.stdout
        data = json.loads(text[text.index("{"):text.rindex("}") + 1])
        return x3ccp.to_checksum(data["deployedTo"]), data["transactionHash"]

    def send(self, role, to, sig, *values, value=None, forced_gas=None):
        cmd = [self.cast, "send", to, *( [sig, *values] if sig else []), "--rpc-url", self.rpc_url, "--json",
               "--async", *self._who(role)]
        if value is not None:
            cmd += ["--value", str(value)]
        if forced_gas:
            cmd += ["--gas-limit", str(forced_gas)]
        tx_hash = _run(cmd).strip().strip('"')
        return self.rpc.receipt(tx_hash)


def _revert_at_parent(rpc, receipt, sender, to, data):
    parent = hex(int(receipt["blockNumber"], 16) - 1)
    try:
        rpc("eth_call", {"from": sender, "to": to, "data": data}, parent)
    except RuntimeError as exc:
        text = str(exc)
        start = text.find("0x", text.find("data"))
        if start >= 0:
            end = start + 2
            while end < len(text) and text[end] in "0123456789abcdefABCDEF":
                end += 1
            return text[start:end]
        return None
    return None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--local", action="store_true", help="run against a fresh local anvil chain")
    mode.add_argument("--base-sepolia", action="store_true", help="run against Base Sepolia (chain 84532)")
    ap.add_argument("--rpc-url")
    ap.add_argument("--owner-account", help="Foundry keystore name of the deployer/owner")
    ap.add_argument("--agent-account", help="Foundry keystore name of the allowed caller")
    ap.add_argument("--recipient", help="the only address withdrawals may pay")
    ap.add_argument("--broadcast", action="store_true")
    ap.add_argument("--i-approve-testnet-transactions", action="store_true")
    ap.add_argument("--out", help="write the run record here")
    args = ap.parse_args(argv)

    anvil = None
    if args.local:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        anvil = subprocess.Popen([_tool("anvil"), "--port", str(port), "--chain-id", "31337", "--auto-impersonate",
                                  "--silent"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        rpc_url, chain_name, recipient = f"http://127.0.0.1:{port}", "local", LOCAL_RECIPIENT
        for _ in range(100):
            try:
                Rpc(rpc_url)("eth_chainId")
                break
            except Exception:
                time.sleep(0.1)
    else:
        missing = [f for f in ("rpc_url", "owner_account", "agent_account", "recipient") if not getattr(args, f)]
        if missing:
            ap.error("--base-sepolia needs " + ", ".join("--" + m.replace("_", "-") for m in missing))
        if not (args.broadcast and args.i_approve_testnet_transactions):
            ap.error("refusing to send testnet transactions without --broadcast --i-approve-testnet-transactions")
        rpc_url, chain_name = args.rpc_url, "base-sepolia"
        recipient = x3ccp.to_checksum(args.recipient)

    try:
        chain = Chain(args, rpc_url)
        chain_id = int(chain.rpc("eth_chainId"), 16)
        expected_chain = x3ccp.CHAINS[chain_name]
        if chain_id != expected_chain:
            sys.exit(f"RPC chain id {chain_id} != {expected_chain}; refusing to continue")
        owner, agent = chain.address("owner"), chain.address("agent")
        record = {"mode": chain_name, "chain_id": chain_id, "started": datetime.now(timezone.utc).isoformat(),
                  "classification": "local EVM run, not public-chain evidence" if args.local
                  else "Base Sepolia testnet transactions", "steps": []}
        subprocess.run([chain.forge, "build"], cwd=EVM, check=True, capture_output=True)

        target, deploy_tx = chain.deploy()
        record["steps"].append({"step": "deploy X3ControlledTreasury", "tx": deploy_tx, "address": target})
        source = POLICY_TEMPLATE.format(chain=chain_name, target=target, owner=owner, agent=agent,
                                        max_wei=MAX_WEI, recipient=recipient)
        compiled = x3ccp.compile_source(source)
        record["policy_source"] = source
        record["policy_hash"] = compiled["policy_hash"]
        print(f"deployed {target}\npolicy {compiled['policy_hash']}")
        for item in x3ccp.explain(compiled):
            print(f"  line {item['line']}: {item['text']}")

        r = chain.send("owner", target, None, value=DEPOSIT_WEI)
        record["steps"].append({"step": "fund treasury", "tx": r["transactionHash"], "status": r["status"]})
        r = chain.send("owner", target, "installPolicy(bytes)", compiled["policy_hex"])
        record["steps"].append({"step": "install policy", "tx": r["transactionHash"], "status": r["status"]})
        if r["status"] != "0x1":
            raise RuntimeError("installPolicy reverted")
        on_chain = chain.rpc("eth_call", {"to": target, "data": "0x" + x3ccp.keccak256(b"x3PolicyHash()")[:4].hex()},
                             "latest")
        if on_chain != compiled["policy_hash"]:
            raise RuntimeError(f"on-chain policy hash {on_chain} != compiled {compiled['policy_hash']}")
        record["on_chain_policy_hash"] = on_chain

        state = x3ccp.EnforcerState()
        cases = [("allowed withdrawal", recipient, ALLOWED_WEI), ("refused: above per-call max", recipient,
                                                                   TOO_MUCH_WEI),
                 ("refused: recipient not allowed", OUTSIDER, ALLOWED_WEI)]
        ok = True
        for label, to, amount in cases:
            data = x3ccp.encode_call(compiled, "withdraw", [to, amount])
            latest = chain.rpc("eth_getBlockByNumber", "latest", False)
            expected = x3ccp.predict(compiled, state, sender=agent, calldata=data,
                                     timestamp=int(latest["timestamp"], 16) + 1, chain_id=chain_id)
            before = int(chain.rpc("eth_getBalance", to, "latest"), 16)
            r = chain.send("agent", target, "withdraw(address,uint256)", to, str(amount), forced_gas=FORCED_GAS)
            after = int(chain.rpc("eth_getBalance", to, "latest"), 16)
            step = {"step": label, "tx": r["transactionHash"], "block": int(r["blockNumber"], 16),
                    "status": r["status"], "predicted": expected, "paid_wei": after - before}
            if expected["allowed"]:
                step["matches_prediction"] = r["status"] == "0x1" and after - before == amount
            else:
                revert = _revert_at_parent(chain.rpc, r, agent, target, "0x" + data.hex())
                decoded = x3ccp.decode_refusal(bytes.fromhex(revert[2:])) if revert else None
                step["revert_data_replayed_at_parent_block"] = revert
                step["decoded"] = decoded
                step["matches_prediction"] = (r["status"] == "0x0" and after == before and decoded is not None
                                              and decoded["clause"] == expected["clause"]
                                              and decoded["code"] == expected["code"])
            ok &= step["matches_prediction"]
            record["steps"].append(step)
            print(f"{label}: tx {r['transactionHash']} status {r['status']} "
                  f"{'MATCHES' if step['matches_prediction'] else 'MISMATCH'} prediction {expected['reason']}")
        record["all_match"] = ok
        record["finished"] = datetime.now(timezone.utc).isoformat()
        if args.out:
            with open(args.out, "w", encoding="utf-8") as fh:
                json.dump(record, fh, indent=2)
                fh.write("\n")
            print(f"run record: {args.out}")
        return 0 if ok else 1
    finally:
        if anvil:
            anvil.kill()


if __name__ == "__main__":
    sys.exit(main())
