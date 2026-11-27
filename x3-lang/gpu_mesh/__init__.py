"""X3 GPU contributor mesh: contributed GPUs serve X3 agents, the Super IDE and dotx3.net.

See ``docs/gpu-mesh/README.md``. Modules:

* ``protocol``    signed envelopes, canonical requests and responses
* ``store``       SQLite registry, app keys, hash-chained receipts
* ``coordinator`` OpenAI-compatible endpoint and the job router
* ``worker``      the contributor daemon (wraps a local Ollama)
* ``ledger``      the signed contribution ledger used for payouts
* ``cli``         ``python -m gpu_mesh ...``
"""

__version__ = "0.1.0"
