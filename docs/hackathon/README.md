# Hackathon materials

**Judges: [start here — payment, limits, proof and reproduction](../../JUDGES.md).**

## Competition framing

**.x3 is an owner-controlled, typed intent language for AI-agent payments, combining compiler checks with Guard enforcement of signed quote terms.**

An intent states the desired outcome. The owner’s `.x3` mandate sets the limits.
The compiler rejects invalid programs; deterministic mandate checks and NEAR Guard
refuse prohibited terms. NEAR Intents / 1Click supplies the supported execution route.
Read [the stage-by-stage explanation](X3_LANGUAGE.md#x3-intents-with-compiler-checks-and-guard-enforcement)
for compile-time versus runtime checks and the evidence boundaries.


- [Three-track control architecture](X3LANG_THREE_TRACK_CONTROL.md)
- [Judge package](JUDGE_PACKAGE.md)
- [Three-minute walkthrough](VIDEO_WALKTHROUGH.md)
- [Submission checklist](SUBMISSION_CHECKLIST.md)
- [CI and developer-experience audit](CI_AUDIT.md)

Start with the local judge demonstration after dependency provisioning:

```bash
python3 scripts/hackathon/demo.py
```

## Optional historical 1Click observations

[Explorer client and evidence boundaries](../../x3-lang/near/explorer/README.md)
provides read-only historical records with shared partner rate limiting. These
are provider-reported observations, not independently verified settlement.
