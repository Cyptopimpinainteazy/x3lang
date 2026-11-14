"""X3 Agent Forge core: registries, build validation, .x3 mandate generation and real compile.

Nothing here decides admissibility on its own. The generated mandate source is compiled by
`near_mandate` (the same lowering `near_intents.py mandate args` uses), and the agent manifest
is the Agent Law runtime's `AgentManifest`. This module turns a character build into those two
inputs and reports what each part of the build actually compiled to.
"""
from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import near_intents as ni
import registry as chain_registry
from agent_constitution import AgentManifest

FORGE_DIR = Path(__file__).resolve().parent
REGISTRY_DIR = FORGE_DIR / "registry"
DEFAULT_TOKENS = FORGE_DIR.parent / "tests" / "fixtures" / "near_intents_tokens.json"

BUILD_VERSION = 1
ENFORCERS = {"x3-guard", "agent-runtime"}
PARAM_TYPES = {"asset", "amount", "address", "percent", "bps", "int", "asset_list", "receiver_list",
               "near_account_list"}
CONFIG_TYPES = {"url", "near_account", "text_list", "choice_list", "int"}
CONNECTOR_TYPES = {"ide", "agent", "pay", "wallet", "data"}
AUTH_TYPES = {"cloudflare_access", "api_key", "near_account", "forge_session", "none"}
IMPLANTS = {"neural_jack", "resonance", "vault_gem", "chain_sigil", "sensor_port"}
PARTS = {"eyes", "feelers", "antennae", "core", "tendrils", "shield", "egg_sacs", "carapace", "sensor_fins",
         "brain_lobes", "claws", "gills", "spikes"}
BODIES = {"serpent", "crustacean", "insectoid", "cephalopod", "titan"}
# Chain names the mandate grammar accepts, in the spelling the UI shows.
CHAIN_LABELS = {"solana": "Solana", "near": "Near", "ethereum": "Ethereum", "bsc": "BSC", "arbitrum": "Arbitrum",
                "polygon": "Polygon", "bitcoin": "Bitcoin"}
MANDATE_SKILLS = ("spend", "payees", "cosign")
NAME_RE = re.compile(r"[A-Za-z][A-Za-z0-9 _-]{0,39}")
DURATION_RE = re.compile(r"[1-9][0-9]{0,5}(m|h|d)")
MAX_EXPIRY_DAYS = 365
MAX_BUDGET = 10 ** 12


class ForgeError(Exception):
    """A refusal with the build node that caused it, so the UI can point at the skill."""

    def __init__(self, code: str, message: str, node: Optional[str] = None, detail: Any = None):
        super().__init__(message)
        self.code, self.message, self.node, self.detail = code, message, node, detail

    def to_dict(self) -> Dict[str, Any]:
        out = {"code": self.code, "message": self.message, "node": self.node}
        if self.detail is not None:
            out["detail"] = self.detail
        return out


class ForgeErrors(Exception):
    def __init__(self, errors: List[ForgeError]):
        super().__init__("; ".join(e.message for e in errors))
        self.errors = errors

    def to_list(self) -> List[Dict[str, Any]]:
        return [e.to_dict() for e in self.errors]


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# --------------------------------------------------------------------------------------------
# Registries
# --------------------------------------------------------------------------------------------

@dataclass
class Registry:
    archetypes: Dict[str, Dict[str, Any]]
    skills: Dict[str, Dict[str, Any]]
    connectors: Dict[str, Dict[str, Any]]
    digest: str = ""

    def public(self) -> Dict[str, Any]:
        return {"archetypes": list(self.archetypes.values()), "skills": list(self.skills.values()),
                "connectors": list(self.connectors.values()), "digest": self.digest}


def _need(cond: bool, where: str, message: str) -> None:
    if not cond:
        raise ForgeError("X3_FORGE_REGISTRY", f"{where}: {message}", where)


def _keys(entry: Dict[str, Any], required: Tuple[str, ...], where: str) -> None:
    _need(isinstance(entry, dict), where, "must be an object")
    missing = [k for k in required if k not in entry]
    extra = [k for k in entry if k not in required]
    _need(not missing, where, f"missing {missing}")
    _need(not extra, where, f"unknown fields {extra}")


def _ident(value: Any, where: str) -> None:
    _need(isinstance(value, str) and re.fullmatch(r"[a-z][a-z0-9_]{1,31}", value) is not None, where,
          f"bad id {value!r}")


def validate_registry(archetypes: Any, skills: Any, connectors: Any) -> Registry:
    """Strict schema check. Any bad entry stops the Forge from starting."""
    _need(isinstance(skills, list) and skills, "skills", "must be a non-empty list")
    _need(isinstance(archetypes, list) and archetypes, "archetypes", "must be a non-empty list")
    _need(isinstance(connectors, list) and connectors, "connectors", "must be a non-empty list")
    sk: Dict[str, Dict[str, Any]] = {}
    for s in skills:
        where = f"skill {s.get('id') if isinstance(s, dict) else s!r}"
        _keys(s, ("id", "name", "description", "category", "risk", "cost", "requires", "conflicts", "needs",
                  "compiles_to", "enforced_by", "params", "mutation"), where)
        _ident(s["id"], where)
        _need(s["id"] not in sk, where, "duplicate id")
        _need(isinstance(s["risk"], int) and 0 <= s["risk"] <= 4, where, "risk must be 0..4")
        _need(isinstance(s["cost"], int) and 1 <= s["cost"] <= 5, where, "cost must be 1..5")
        _need(s["enforced_by"] in ENFORCERS, where, f"enforced_by must be one of {sorted(ENFORCERS)}")
        kind = s["compiles_to"].get("kind")
        _need(kind in ("mandate", "capability"), where, "compiles_to.kind must be mandate or capability")
        if kind == "mandate":
            _need(s["id"] in MANDATE_SKILLS and s["enforced_by"] == "x3-guard", where,
                  "only spend, payees and cosign compile to mandate clauses, enforced by x3-guard")
        else:
            cap = s["compiles_to"].get("capability")
            _need(isinstance(cap, str) and re.fullmatch(r"[a-z]+\.[a-z_]+", cap) is not None, where, "bad capability")
            _need(s["enforced_by"] == "agent-runtime", where, "capabilities are enforced by the agent runtime")
        _need(s["mutation"].get("part") in PARTS, where, f"mutation.part must be one of {sorted(PARTS)}")
        ids = set()
        for p in s["params"]:
            pw = f"{where} param {p.get('id')}"
            _need(isinstance(p, dict) and isinstance(p.get("id"), str) and p["id"] not in ids, pw, "bad or duplicate id")
            ids.add(p["id"])
            _need(p.get("type") in PARAM_TYPES, pw, f"type must be one of {sorted(PARAM_TYPES)}")
            _need("default" in p and isinstance(p.get("label"), str), pw, "needs label and default")
        sk[s["id"]] = s
    for s in sk.values():
        for rel in ("requires", "conflicts", "needs"):
            for other in s[rel]:
                _need(other in sk and other != s["id"], f"skill {s['id']}", f"{rel} names unknown skill {other!r}")
    # No prerequisite cycles.
    state: Dict[str, int] = {}

    def visit(node: str, path: Tuple[str, ...]) -> None:
        if state.get(node) == 2:
            return
        _need(state.get(node) != 1, f"skill {node}", f"prerequisite cycle {' -> '.join(path + (node,))}")
        state[node] = 1
        for nxt in sk[node]["requires"]:
            visit(nxt, path + (node,))
        state[node] = 2

    for s in sk:
        visit(s, ())
    ar: Dict[str, Dict[str, Any]] = {}
    for a in archetypes:
        where = f"archetype {a.get('id') if isinstance(a, dict) else a!r}"
        _keys(a, ("id", "name", "body", "blurb", "points", "skill_pool"), where)
        _ident(a["id"], where)
        _need(a["id"] not in ar, where, "duplicate id")
        _need(a["body"] in BODIES, where, f"body must be one of {sorted(BODIES)}")
        _need(isinstance(a["points"], int) and 1 <= a["points"] <= 40, where, "points must be 1..40")
        for sid in a["skill_pool"]:
            _need(sid in sk, where, f"skill_pool names unknown skill {sid!r}")
        ar[a["id"]] = a
    co: Dict[str, Dict[str, Any]] = {}
    surfaces = set()
    for c in connectors:
        where = f"connector {c.get('id') if isinstance(c, dict) else c!r}"
        _keys(c, ("id", "name", "type", "description", "setup_url", "auth", "scopes", "surface", "requires_skills",
                  "risk", "implant", "config"), where)
        _ident(c["id"], where)
        _need(c["id"] not in co, where, "duplicate id")
        _need(c["type"] in CONNECTOR_TYPES, where, f"type must be one of {sorted(CONNECTOR_TYPES)}")
        _need(c["auth"] in AUTH_TYPES, where, f"auth must be one of {sorted(AUTH_TYPES)}")
        _need(c["implant"] in IMPLANTS, where, f"implant must be one of {sorted(IMPLANTS)}")
        _need(isinstance(c["risk"], int) and 0 <= c["risk"] <= 4, where, "risk must be 0..4")
        _need(isinstance(c["surface"], str) and re.fullmatch(r"[a-z]+\.[a-z0-9_]+", c["surface"]) is not None, where,
              "bad surface")
        _need(c["surface"] not in surfaces, where, "duplicate surface")
        surfaces.add(c["surface"])
        for sid in c["requires_skills"]:
            _need(sid in sk, where, f"requires_skills names unknown skill {sid!r}")
        url = c["setup_url"]
        _need(url is None or url == "forge:self" or (isinstance(url, str) and url.startswith("https://")), where,
              "setup_url must be https://, forge:self or null")
        has_url_config = any(f.get("type") == "url" for f in c["config"])
        _need(url is not None or has_url_config, where, "a connector without setup_url needs a url config field")
        for f in c["config"]:
            _need(isinstance(f, dict) and f.get("type") in CONFIG_TYPES and isinstance(f.get("id"), str), where,
                  f"bad config field {f!r}")
        co[c["id"]] = c
    reg = Registry(ar, sk, co)
    reg.digest = sha256_hex(canonical({"archetypes": archetypes, "skills": skills, "connectors": connectors}))
    return reg


def load_registry(directory: Path = REGISTRY_DIR) -> Registry:
    def read(name: str) -> Any:
        return json.loads((directory / name).read_text())

    return validate_registry(read("archetypes.json"), read("skills.json"), read("connectors.json"))


# --------------------------------------------------------------------------------------------
# Assets: only what the loaded 1Click token list can resolve, so the compiler never guesses
# --------------------------------------------------------------------------------------------

@dataclass
class Assets:
    tokens: List[Dict[str, Any]]
    source: str
    by_ref: Dict[str, Dict[str, Any]] = field(default_factory=dict)

    def __post_init__(self) -> None:
        for chain, label in CHAIN_LABELS.items():
            for symbol in sorted(chain_registry.ASSETS_BY_CHAIN.get(chain, ())):
                try:
                    tok = ni.resolve_asset(self.tokens, chain, symbol)
                except ni.NearIntentsError:
                    continue
                if str(tok.get("symbol", "")).upper() != symbol.upper():
                    continue  # an alias (Near.NEAR is delivered as wNEAR): list the token once
                self.by_ref[f"{label}.{symbol}"] = {"ref": f"{label}.{symbol}", "chain": chain, "symbol": symbol,
                                                    "asset_id": tok["assetId"], "decimals": int(tok["decimals"])}

    def public(self) -> Dict[str, Any]:
        return {"source": self.source, "assets": list(self.by_ref.values())}


def load_assets(tokens_path: Optional[str] = None, *, live: bool = False) -> Assets:
    if live:
        client = ni.OneClickClient()
        return Assets(ni.load_tokens(client, None), f"live 1Click /v0/tokens from {client.base_url}")
    path = Path(tokens_path) if tokens_path else DEFAULT_TOKENS
    tokens = json.loads(path.read_text())
    label = "committed 1Click snapshot" if path == DEFAULT_TOKENS else "token list file"
    return Assets(tokens, f"{label}: {path.name} (sha256 {sha256_hex(path.read_bytes())[:16]})")


# --------------------------------------------------------------------------------------------
# Build validation
# --------------------------------------------------------------------------------------------

def _decimal(value: Any) -> Decimal:
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise InvalidOperation
    d = Decimal(str(value).strip())
    if not d.is_finite():
        raise InvalidOperation
    return d


def _fmt(d: Decimal) -> str:
    text = format(d.normalize(), "f")
    return text


class _Collector:
    def __init__(self) -> None:
        self.errors: List[ForgeError] = []

    def add(self, code: str, message: str, node: str, detail: Any = None) -> None:
        self.errors.append(ForgeError(code, message, node, detail))


def _param_value(reg_param: Dict[str, Any], raw: Any, node: str, ctx: Dict[str, Any], assets: Assets,
                 errs: _Collector) -> Any:
    t, pid = reg_param["type"], reg_param["id"]
    where = f"{node}.{pid}"
    label = reg_param["label"]
    if t == "asset":
        if raw not in assets.by_ref:
            errs.add("X3_FORGE_PARAM", f"{label}: {raw!r} is not in the loaded 1Click token list", where)
            return None
        return raw
    if t in ("amount", "percent"):
        try:
            d = _decimal(raw)
        except (InvalidOperation, ValueError):
            errs.add("X3_FORGE_PARAM", f"{label}: {raw!r} is not a number", where)
            return None
        lo, hi = Decimal(reg_param["min"]), Decimal(reg_param["max"])
        if not lo <= d <= hi:
            errs.add("X3_FORGE_BOUND", f"{label} must be between {lo} and {hi}", where)
            return None
        if t == "percent":
            return _fmt(d)
        ref = ctx.get(reg_param["of"])
        if ref is None:
            errs.add("X3_FORGE_PARAM", f"{label}: the asset it is measured in is not set", where)
            return None
        decimals = assets.by_ref[ref]["decimals"]
        base = d * (Decimal(10) ** decimals)
        if base != base.to_integral_value():
            errs.add("X3_FORGE_PARAM", f"{label}: {ref} has {decimals} decimals; {raw} is finer than that", where)
            return None
        return {"amount": _fmt(d), "asset": ref, "base_units": str(int(base))}
    if t in ("bps", "int"):
        if isinstance(raw, bool) or not isinstance(raw, int):
            errs.add("X3_FORGE_PARAM", f"{label} must be a whole number", where)
            return None
        if not reg_param["min"] <= raw <= reg_param["max"]:
            errs.add("X3_FORGE_BOUND", f"{label} must be between {reg_param['min']} and {reg_param['max']}", where)
            return None
        return raw
    if t == "address":
        ref = ctx.get(reg_param["of"])
        chain = assets.by_ref[ref]["chain"] if ref in assets.by_ref else None
        if not isinstance(raw, str) or chain is None or raw == "sender" or not chain_registry.is_valid_receiver(chain, raw) \
                or not raw:
            errs.add("X3_FORGE_PARAM", f"{label}: {raw!r} is not an address on {chain or 'the spend chain'}", where)
            return None
        return raw
    # Lists
    if not isinstance(raw, list) or not all(isinstance(x, str) for x in raw):
        errs.add("X3_FORGE_PARAM", f"{label} must be a list", where)
        return None
    items = [x.strip() for x in raw]
    if not reg_param["min"] <= len(items) <= reg_param["max"] or len(set(items)) != len(items):
        errs.add("X3_FORGE_BOUND", f"{label}: {reg_param['min']}..{reg_param['max']} distinct entries", where)
        return None
    if t == "asset_list":
        bad = [x for x in items if x not in assets.by_ref]
        if bad:
            errs.add("X3_FORGE_PARAM", f"{label}: {bad} not in the loaded 1Click token list", where)
            return None
        return items
    if t == "near_account_list":
        bad = [x for x in items if not chain_registry.is_valid_near_account(x)]
        if bad:
            errs.add("X3_FORGE_PARAM", f"{label}: {bad} are not NEAR accounts", where)
            return None
        return items
    # receiver_list: each must be an address on at least one destination chain
    chains = {assets.by_ref[r]["chain"] for r in (ctx.get(reg_param["of"]) or []) if r in assets.by_ref}
    bad = [x for x in items if x == "sender" or not any(chain_registry.is_valid_receiver(c, x) for c in chains)]
    if bad or not chains:
        errs.add("X3_FORGE_PARAM", f"{label}: {bad or items} are not addresses on {sorted(chains) or 'a destination'}",
                 where)
        return None
    return items


def normalize_build(reg: Registry, assets: Assets, build: Any) -> Dict[str, Any]:
    """Validate a client build and return its canonical form. Cosmetics are dropped here: they
    never reach the hash, the mandate or the manifest. Raises ForgeErrors with every problem."""
    errs = _Collector()
    if not isinstance(build, dict):
        raise ForgeErrors([ForgeError("X3_FORGE_BUILD", "the build must be an object", "build")])
    allowed = {"version", "name", "archetype", "identity", "skills", "connections", "cosmetics"}
    extra = sorted(set(build) - allowed)
    if extra:
        errs.add("X3_FORGE_BUILD", f"unknown build fields {extra}", "build")
    if build.get("version") != BUILD_VERSION:
        errs.add("X3_FORGE_BUILD", f"build version must be {BUILD_VERSION}", "build")
    name = build.get("name")
    if not isinstance(name, str) or not NAME_RE.fullmatch(name):
        errs.add("X3_FORGE_NAME", "name: 1-40 letters, digits, spaces, _ or -, starting with a letter", "origin.name")
    arch = reg.archetypes.get(build.get("archetype"))
    if arch is None:
        errs.add("X3_FORGE_ARCHETYPE", f"unknown archetype {build.get('archetype')!r}", "origin.archetype")

    ident = build.get("identity") if isinstance(build.get("identity"), dict) else {}
    owner, agent = ident.get("owner"), ident.get("agent")
    for key, value in (("owner", owner), ("agent", agent)):
        if not isinstance(value, str) or not chain_registry.is_valid_near_account(value):
            errs.add("X3_FORGE_ACCOUNT", f"{key}: {value!r} is not a NEAR account", f"origin.{key}")
    if owner and owner == agent:
        errs.add("X3_FORGE_ACCOUNT", "the agent must be a different account from the owner", "origin.agent")
    expires = ident.get("expires")
    if not isinstance(expires, str) or not DURATION_RE.fullmatch(expires):
        errs.add("X3_FORGE_EXPIRES", "expires: like 30d, 12h or 90m", "origin.expires")
    elif _duration_ms(expires) > MAX_EXPIRY_DAYS * 86_400_000:
        errs.add("X3_FORGE_EXPIRES", f"expires: at most {MAX_EXPIRY_DAYS}d", "origin.expires")
    budget = ident.get("budget")
    if isinstance(budget, bool) or not isinstance(budget, int) or not 1 <= budget <= MAX_BUDGET:
        errs.add("X3_FORGE_BUDGET", f"runtime budget must be 1..{MAX_BUDGET}", "origin.budget")
    extra_ident = sorted(set(ident) - {"owner", "agent", "expires", "budget"})
    if extra_ident:
        errs.add("X3_FORGE_BUILD", f"unknown identity fields {extra_ident}", "origin")

    raw_skills = build.get("skills")
    if not isinstance(raw_skills, dict):
        errs.add("X3_FORGE_SKILL", "skills must be an object of skill id to parameters", "skills")
        raw_skills = {}
    chosen = set(raw_skills)
    for sid in sorted(chosen):
        if sid not in reg.skills:
            errs.add("X3_FORGE_SKILL", f"unknown skill {sid!r}; it does not exist and cannot be granted", f"skill.{sid}")
    chosen &= set(reg.skills)
    if arch is not None:
        for sid in sorted(chosen - set(arch["skill_pool"])):
            errs.add("X3_FORGE_POOL", f"{reg.skills[sid]['name']} is not in the {arch['name']} skill pool", f"skill.{sid}")
        spent = sum(reg.skills[s]["cost"] for s in chosen)
        if spent > arch["points"]:
            errs.add("X3_FORGE_POINTS", f"{spent} skill points used, {arch['name']} has {arch['points']}", "skills")
    for sid in sorted(chosen):
        s = reg.skills[sid]
        for req in s["requires"]:
            if req not in chosen:
                errs.add("X3_FORGE_REQUIRES", f"{s['name']} requires {reg.skills[req]['name']}", f"skill.{sid}")
        for need in s["needs"]:
            if need not in chosen:
                errs.add("X3_FORGE_NEEDS", f"{s['name']} needs {reg.skills[need]['name']} to compile", f"skill.{sid}")
        for con in s["conflicts"]:
            if con in chosen:
                errs.add("X3_FORGE_CONFLICT", f"{s['name']} conflicts with {reg.skills[con]['name']}", f"skill.{sid}")

    # Parameters, in dependency order so amounts know their asset.
    norm_skills: Dict[str, Dict[str, Any]] = {}
    ctx: Dict[str, Any] = {}
    order = [s for s in ("spend", "payees", "cosign") if s in chosen] + sorted(chosen - set(MANDATE_SKILLS))
    for sid in order:
        s = reg.skills[sid]
        given = raw_skills.get(sid)
        if given is None:
            given = {}
        if not isinstance(given, dict):
            errs.add("X3_FORGE_PARAM", f"{s['name']}: parameters must be an object", f"skill.{sid}")
            continue
        known = {p["id"] for p in s["params"]}
        for pid in sorted(set(given) - known):
            errs.add("X3_FORGE_PARAM", f"{s['name']} has no parameter {pid!r}", f"skill.{sid}.{pid}")
        out: Dict[str, Any] = {}
        for p in s["params"]:
            local = dict(ctx)
            local.update({k: v for k, v in out.items()})
            local["spend.asset"] = ctx.get("spend.asset")
            value = _param_value(p, given.get(p["id"], p["default"]), f"skill.{sid}", local, assets, errs)
            out[p["id"]] = value
            if p["type"] in ("asset", "asset_list"):
                ctx[f"{sid}.{p['id']}"] = value
                if sid == "spend" and p["id"] == "asset":
                    ctx["spend.asset"] = value
                    ctx["asset"] = value
                if sid == "payees" and p["id"] == "assets":
                    ctx["assets"] = value
            if sid == "spend" and p["id"] == "asset":
                ctx["asset"] = value
        norm_skills[sid] = out

    raw_conns = build.get("connections")
    if not isinstance(raw_conns, list):
        errs.add("X3_FORGE_CONNECTION", "connections must be a list", "comms")
        raw_conns = []
    norm_conns: List[Dict[str, Any]] = []
    seen = set()
    for i, c in enumerate(raw_conns):
        if not isinstance(c, dict) or set(c) - {"connector", "connection_id"} or "connector" not in c:
            errs.add("X3_FORGE_CONNECTION", "each connection is {connector, connection_id}", f"comms.{i}")
            continue
        cid = c["connector"]
        spec = reg.connectors.get(cid)
        if spec is None:
            errs.add("X3_FORGE_CONNECTION", f"unknown connector {cid!r}", f"connector.{cid}")
            continue
        if cid in seen:
            errs.add("X3_FORGE_CONNECTION", f"{spec['name']} is plugged in twice", f"connector.{cid}")
            continue
        seen.add(cid)
        for sid in spec["requires_skills"]:
            if sid not in chosen:
                errs.add("X3_FORGE_CONNECTION", f"{spec['name']} needs the {reg.skills[sid]['name']} skill",
                         f"connector.{cid}")
        norm_conns.append({"connector": cid})
    if not norm_conns:
        errs.add("X3_FORGE_CONNECTION", "plug in at least one connection: an agent with no surface can't be born",
                 "comms")
    if errs.errors:
        raise ForgeErrors(errs.errors)
    return {
        "version": BUILD_VERSION,
        "name": name,
        "archetype": arch["id"],
        "identity": {"owner": owner, "agent": agent, "expires": expires, "budget": budget},
        "skills": {k: norm_skills[k] for k in sorted(norm_skills)},
        "connections": sorted(norm_conns, key=lambda c: c["connector"]),
    }


def raw_build(normalized: Dict[str, Any]) -> Dict[str, Any]:
    """The client-shaped build that normalizes back to `normalized` (amounts as plain decimals)."""
    skills = {sid: {k: (v["amount"] if isinstance(v, dict) and "base_units" in v else v) for k, v in params.items()}
              for sid, params in normalized["skills"].items()}
    return {"version": normalized["version"], "name": normalized["name"], "archetype": normalized["archetype"],
            "identity": dict(normalized["identity"]), "skills": skills,
            "connections": [{"connector": c["connector"]} for c in normalized["connections"]]}


def _duration_ms(text: str) -> int:
    n, unit = int(text[:-1]), text[-1]
    return n * {"m": 60_000, "h": 3_600_000, "d": 86_400_000}[unit]


def build_hash(normalized: Dict[str, Any]) -> str:
    return sha256_hex(canonical(normalized))


# --------------------------------------------------------------------------------------------
# .x3 generation
# --------------------------------------------------------------------------------------------

def mandate_ident(normalized: Dict[str, Any]) -> str:
    slug = re.sub(r"[^a-z0-9_]+", "_", normalized["name"].lower()).strip("_") or "agent"
    return f"forge_{slug}"[:48]


def generate_mandate_source(normalized: Dict[str, Any], digest: str) -> Tuple[Optional[str], Dict[int, str]]:
    """The mandate `.x3` for this build, or None when the agent has no spending skill.
    Returns the source and a map of source line number -> build node for compiler errors."""
    skills = normalized["skills"]
    if "spend" not in skills:
        return None, {}
    sp, pay = skills["spend"], skills["payees"]
    ident = normalized["identity"]
    rows: List[Tuple[str, str]] = [
        ("", f"// Generated by X3 Agent Forge from build {digest}."),
        ("", f"// Agent: {normalized['name']} ({normalized['archetype']}). Edit the build, not this file."),
        ("", f"mandate {mandate_ident(normalized)} {{"),
        ("origin.owner", f"    owner {ident['owner']}"),
        ("origin.agent", f"    agent {ident['agent']}"),
        ("skill.spend.asset", f"    spend {sp['asset']}"),
        ("skill.spend.per_payment", f"    per_payment {sp['per_payment']['base_units']}"),
        ("skill.spend.per_24h", f"    per_24h {sp['per_24h']['base_units']}"),
        ("skill.payees.recipients", f"    allow recipients {', '.join(pay['recipients'])}"),
        ("skill.payees.assets", f"    allow assets {', '.join(pay['assets'])}"),
        ("skill.spend.refund_to", f"    refund_to {sp['refund_to']}"),
        ("skill.spend.max_slippage", f"    require slippage <= {sp['max_slippage']}%"),
        ("skill.spend.max_fees", f"    require fees <= {sp['max_fees']} bps"),
    ]
    if "cosign" in skills:
        co = skills["cosign"]
        rows.append(("skill.cosign", f"    approve above {co['above']['base_units']} by {co['threshold']} of "
                                     f"{', '.join(co['approvers'])}"))
    rows.append(("origin.expires", f"    expires {ident['expires']}"))
    rows.append(("", "}"))
    lines, line_map = [], {}
    for i, (node, text) in enumerate(rows, 1):
        lines.append(text)
        if node:
            line_map[i] = node
    return "\n".join(lines) + "\n", line_map


# --------------------------------------------------------------------------------------------
# Compile
# --------------------------------------------------------------------------------------------

GUARD_DEPOSIT_MANDATE = "0.1 NEAR"


def _map_compiler_error(exc: Exception, line_map: Dict[int, str]) -> ForgeError:
    code = getattr(exc, "code", "X3_COMPILE")
    message = getattr(exc, "message", str(exc))
    detail = getattr(exc, "detail", None)
    line = detail.get("line") if isinstance(detail, dict) else None
    node = line_map.get(line) if line else None
    if node is None:
        fld = getattr(exc, "field", None) or ""
        node = {"per_payment": "skill.spend.per_payment", "per_24h": "skill.spend.per_24h",
                "refund_to": "skill.spend.refund_to", "allow recipients": "skill.payees.recipients",
                "allow assets": "skill.payees.assets", "approve": "skill.cosign", "agent": "origin.agent",
                "owner": "origin.owner", "expires": "origin.expires", "spend": "skill.spend.asset"}.get(fld, "compile")
    return ForgeError(code, f"compiler: {message}", node, {"line": line} if line else None)


def compile_mandate_source(source: str, line_map: Dict[int, str], assets: Assets, workdir: Path,
                           now: datetime) -> Dict[str, Any]:
    """Run the real mandate compiler on the generated source."""
    path = workdir / "agent.x3"
    path.write_text(source)
    try:
        return ni.mandates().compile_mandate(str(path), assets.tokens, now=now)
    except ni.NearIntentsError as exc:
        raise ForgeErrors([_map_compiler_error(exc, line_map)]) from exc


def agent_manifest(reg: Registry, normalized: Dict[str, Any], expires_at: int) -> AgentManifest:
    caps = set()
    for sid in normalized["skills"]:
        ct = reg.skills[sid]["compiles_to"]
        caps.add(ct["capability"] if ct["kind"] == "capability" else f"pay.{sid}")
    surfaces = {reg.connectors[c["connector"]]["surface"] for c in normalized["connections"]}
    spawn = normalized["skills"].get("brood", {}).get("spawn_limit", 0)
    return AgentManifest(agent_id=normalized["identity"]["agent"], parent_id=None, capabilities=frozenset(caps),
                         surfaces=frozenset(surfaces), budget=normalized["identity"]["budget"], expires_at=expires_at,
                         lineage=(), spawn_limit=spawn)


def manifest_dict(m: AgentManifest) -> Dict[str, Any]:
    return {"agent_id": m.agent_id, "parent_id": m.parent_id, "capabilities": sorted(m.capabilities),
            "surfaces": sorted(m.surfaces), "budget": m.budget, "expires_at": m.expires_at,
            "lineage": list(m.lineage), "spawn_limit": m.spawn_limit}


def manifest_from_dict(d: Dict[str, Any]) -> AgentManifest:
    return AgentManifest(d["agent_id"], d["parent_id"], frozenset(d["capabilities"]), frozenset(d["surfaces"]),
                         d["budget"], d["expires_at"], tuple(d["lineage"]), d["spawn_limit"])


def capability_table(reg: Registry, normalized: Dict[str, Any]) -> List[Dict[str, Any]]:
    rows = []
    for sid, params in normalized["skills"].items():
        s = reg.skills[sid]
        ct = s["compiles_to"]
        if ct["kind"] == "mandate":
            grants = {
                "spend": lambda p: (f"spend {p['asset']}: at most {p['per_payment']['amount']} a payment, "
                                    f"{p['per_24h']['amount']} per 24h; refunds pinned to {p['refund_to']}; "
                                    f"slippage <= {p['max_slippage']}%, fees <= {p['max_fees']} bps"),
                "payees": lambda p: f"pay only {', '.join(p['recipients'])} in {', '.join(p['assets'])}",
                "cosign": lambda p: (f"payments above {p['above']['amount']} {p['above']['asset']} wait for "
                                     f"{p['threshold']} of {', '.join(p['approvers'])}"),
            }[sid](params)
            compiles = ", ".join(ct["clauses"])
        else:
            grants = s["description"]
            compiles = f"runtime capability {ct['capability']}"
        rows.append({"source": f"skill.{sid}", "name": s["name"], "grants": grants, "compiles_to": compiles,
                     "enforced_by": s["enforced_by"], "risk": s["risk"]})
    for c in normalized["connections"]:
        spec = reg.connectors[c["connector"]]
        rows.append({"source": f"connector.{spec['id']}", "name": spec["name"],
                     "grants": "a channel only; what passes through it is set by the skills above",
                     "compiles_to": f"runtime surface {spec['surface']}", "enforced_by": "agent-runtime",
                     "risk": spec["risk"]})
    return rows


# --------------------------------------------------------------------------------------------
# Stats and creature traits: functions of the normalized build only, so an exported manifest
# reproduces them exactly
# --------------------------------------------------------------------------------------------

def _clamp(x: float) -> int:
    return int(max(0, min(100, round(x))))


def derive_stats(reg: Registry, normalized: Dict[str, Any]) -> Dict[str, int]:
    skills = normalized["skills"]
    risk_tiers = [reg.skills[s]["risk"] for s in skills] + [reg.connectors[c["connector"]]["risk"]
                                                            for c in normalized["connections"]]
    power = sum((reg.skills[s]["risk"] + 1) * 5 for s in skills)
    spend = skills.get("spend")
    if spend:
        daily = float(Decimal(spend["per_24h"]["amount"]))
        power += 6 * (1 + len(str(int(daily))))
    pay = skills.get("payees")
    chains = {a.split(".")[0] for a in (pay["assets"] if pay else [])}
    reach = 10 * len(normalized["connections"]) + (6 * len(pay["assets"]) + 4 * len(pay["recipients"]) if pay else 0) \
        + 8 * len(chains) + (10 if "cross_chain" in skills else 0)
    autonomy = 25 + (20 if spend else 0) + (15 if "brood" in skills else 0) \
        + 4 * skills.get("brood", {}).get("spawn_limit", 0) + (10 if "deployer" in skills else 0)
    cos = skills.get("cosign")
    if cos:
        autonomy -= 10 + 5 * cos["threshold"]
    risk = 18 * max(risk_tiers) + 3 * len(skills)
    if spend:
        ratio = float(Decimal(spend["per_payment"]["base_units"]) / Decimal(spend["per_24h"]["base_units"]))
        risk += 10 * ratio
    if cos:
        risk -= 12
    speed = 60 + (12 if "quote_hunter" in skills else 0) + (8 if "cross_chain" in skills else 0) \
        - (15 * cos["threshold"] if cos else 0) - 2 * len(normalized["connections"])
    if spend and Decimal(spend["max_slippage"]) < Decimal("0.5"):
        speed -= 8
    return {"power": _clamp(power), "reach": _clamp(reach), "autonomy": _clamp(autonomy), "risk": _clamp(risk),
            "speed": _clamp(speed)}


def derive_traits(reg: Registry, normalized: Dict[str, Any], digest: str) -> Dict[str, Any]:
    arch = reg.archetypes[normalized["archetype"]]
    parts: Dict[str, Dict[str, Any]] = {}
    for sid, params in normalized["skills"].items():
        m = reg.skills[sid]["mutation"]
        part = m["part"]
        entry = parts.setdefault(part, {"part": part, "count": 0, "level": 0, "sources": []})
        entry["sources"].append(sid)
        if "count" in m:
            entry["count"] += m["count"]
        if "level" in m:
            entry["level"] = max(entry["level"], m["level"])
        if m.get("count_by") == "recipients":
            entry["count"] += 2 * len(params["recipients"])
        if m.get("count_by") == "spawn_limit":
            entry["count"] += params["spawn_limit"]
        if m.get("scale_by") == "per_24h":
            entry["level"] = max(entry["level"], 1 + len(str(int(Decimal(params["per_24h"]["amount"])))))
    max_risk = max([reg.skills[s]["risk"] for s in normalized["skills"]] or [0])
    implants = [{"implant": reg.connectors[c["connector"]]["implant"], "connector": c["connector"]}
                for c in normalized["connections"]]
    return {"seed": digest, "body": arch["body"], "risk_tier": max_risk,
            "parts": sorted(parts.values(), key=lambda p: p["part"]), "implants": implants}


# --------------------------------------------------------------------------------------------
# One call: validate → generate → compile → manifest → stats
# --------------------------------------------------------------------------------------------

def compile_build(reg: Registry, assets: Assets, build: Any, workdir: Path, *, now: Optional[datetime] = None
                  ) -> Dict[str, Any]:
    now = now or datetime.now(timezone.utc)
    normalized = normalize_build(reg, assets, build)
    digest = build_hash(normalized)
    source, line_map = generate_mandate_source(normalized, digest)
    compiled = None
    create_mandate = None
    if source is not None:
        compiled = compile_mandate_source(source, line_map, assets, workdir, now)
        create_mandate = {"method": "create_mandate", "args": {"agent": compiled["agent"], "terms": compiled["terms"]},
                          "signer": compiled["owner"], "attached_deposit": GUARD_DEPOSIT_MANDATE, "gas": ni.GUARD_GAS,
                          "expires": compiled["expires"]}
        expires_at = compiled["terms"]["expires_at_ms"] // 1000
    else:
        expires_at = int(now.timestamp()) + _duration_ms(normalized["identity"]["expires"]) // 1000
    manifest = agent_manifest(reg, normalized, expires_at)
    return {
        "build_hash": digest,
        "normalized": normalized,
        "source": source,
        "source_sha256": sha256_hex(source.encode()) if source else None,
        "line_map": {str(k): v for k, v in line_map.items()},
        "compiled": compiled,
        "compiled_sha256": sha256_hex(canonical(compiled)) if compiled else None,
        "create_mandate": create_mandate,
        "manifest": manifest_dict(manifest),
        "capabilities": capability_table(reg, normalized),
        "stats": derive_stats(reg, normalized),
        "traits": derive_traits(reg, normalized, digest),
        "tokens_source": assets.source,
        "compiled_at": now.isoformat().replace("+00:00", "Z"),
        "registry_digest": reg.digest,
    }
