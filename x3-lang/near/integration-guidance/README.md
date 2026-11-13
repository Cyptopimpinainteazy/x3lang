# NEAR Intents integration guidance

Educational, side-effect-free quote planning plus a read-only provider incident adapter for X3 integrations. These modules do not authorize payments, submit incidents, or implement a solver. See [Bootcamp](../../../docs/hackathon/BOOTCAMP_INTENTS.md) for integration boundaries and official references.

Run with Node 22.18+:

```sh
node --test x3-lang/near/integration-guidance/*.test.mjs
```

`planDeadline` uses seconds for durations and milliseconds for timestamps. Dry quotes are informational. Oversized execution estimates produce a capped proposed deadline and `ok: false`; never execute them. Provider incident status is distinct from a surface security mode.

`createShieldReader` reads `NEAR_INTENTS_SHIELD_JWT` on the server only. It returns unknown when unavailable or unconfigured, projects only public incident fields, and caches successful reads for 30 seconds. Consumers must expire their displayed observations after that interval. An Explorer key is not a SHIELD key. Never put credentials in source, browser bundles, or copied documentation.
