"""Read what GPUs this machine has. Everything here is self-reported to the coordinator."""

from __future__ import annotations

import json
import shutil
import subprocess
from typing import Any, Dict, List


def _run(argv: List[str], timeout: float = 10.0) -> str:
    try:
        out = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.SubprocessError):
        return ""
    return out.stdout if out.returncode == 0 else ""


def _int(value: str) -> int:
    try:
        return max(0, int(float(value.strip())))
    except (ValueError, AttributeError):
        return 0


def parse_nvidia_smi(text: str) -> List[Dict[str, Any]]:
    """Parse ``nvidia-smi --query-gpu=name,memory.total,memory.used,utilization.gpu,temperature.gpu
    --format=csv,noheader,nounits``."""
    gpus = []
    for line in text.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) < 5 or not parts[0]:
            continue
        gpus.append({"name": parts[0][:96], "vendor": "nvidia", "vram_mb": _int(parts[1]),
                     "mem_used_mb": _int(parts[2]), "util_pct": min(100, _int(parts[3])), "temp_c": _int(parts[4])})
    return gpus


def parse_rocm_smi(text: str) -> List[Dict[str, Any]]:
    """Parse ``rocm-smi --showproductname --showmeminfo vram --json``."""
    try:
        data = json.loads(text)
    except (json.JSONDecodeError, TypeError):
        return []
    gpus = []
    for key, card in sorted(data.items()) if isinstance(data, dict) else []:
        if not key.startswith("card") or not isinstance(card, dict):
            continue
        name = card.get("Card series") or card.get("Card SKU") or card.get("Card model") or "AMD GPU"
        total = card.get("VRAM Total Memory (B)")
        used = card.get("VRAM Total Used Memory (B)")
        gpus.append({"name": str(name)[:96], "vendor": "amd",
                     "vram_mb": _int(str(total)) // (1024 * 1024) if total else 0,
                     "mem_used_mb": _int(str(used)) // (1024 * 1024) if used else 0})
    return gpus


def probe() -> List[Dict[str, Any]]:
    if shutil.which("nvidia-smi"):
        gpus = parse_nvidia_smi(_run(["nvidia-smi", "--query-gpu=name,memory.total,memory.used,utilization.gpu,"
                                      "temperature.gpu", "--format=csv,noheader,nounits"]))
        if gpus:
            return gpus[:16]
    if shutil.which("rocm-smi"):
        gpus = parse_rocm_smi(_run(["rocm-smi", "--showproductname", "--showmeminfo", "vram", "--json"]))
        if gpus:
            return gpus[:16]
    return []
