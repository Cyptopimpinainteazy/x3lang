# X3Lang / X3 Shield — Hackathon Judge Video Assets

Created November 6, 2026. This pack contains **nine 1920×1080 editable SVG scenes**, nine rendered PNG frames, and three 24fps MP4 clips placed on x3star1.

## The 9 scenes
| File prefix | Scene | Suggested use |
|---|---|---|
| 01 | Opening title | X3 SHIELD / Powered by X3Lang |
| 02 | Owner sets rules | `.x3` mandate, spending caps, allowed recipient |
| 03 | Attack injected prompt | Illustrative hostile instruction to an AI agent |
| 04 | Shield activates | Independent policy boundary |
| 05 | Mandate guard evaluation | Illustrative guard checks; two example violations |
| 06 | Guard refuses attacker | Actual NEAR testnet refusal by `mandate_recipient` |
| 07 | NEAR testnet proof | x3lang.testnet deployment; accept / refuse / revoke |
| 08 | Verification receipts | Reproducible source and actual transaction IDs |
| 09 | Cinematic ending | "Don't trust the prompt. Verify the payment." |

## Videos
- `video/X3Lang_24s_Judge_Visual_Reel.mp4`: 24-second overview with fades.
- `video/X3Lang_Attack_Shield_Refusal.mp4`: 11.5-second attack → check → refusal sequence.
- `video/X3Shield_Activation_Loop.mp4`: 4-second shield pulse animation.

These are **visual B-roll and intro/outro assets**, not replacements for the actual screen recording of the judge demo. The reel is a quick-edit preview and has no narration/audio.

## Suggested voiceover
- 01: "AI agents can propose transactions. But a prompt is not financial authority."
- 02: "The owner writes spending limits, recipient rules, fee caps and approvals in X3Lang."
- 03: "Even if a model follows a malicious instruction, the proposed payment has to pass policy."
- 04–05: "X3 Shield checks the request independently."
- 06: "The NEAR testnet guard refused an unauthorized recipient, with a verifiable transaction."
- 07–08: "We've demonstrated on-chain mandate registration, refusal, acceptance and revocation on testnet."
- 09: "Don't trust the prompt. Verify the payment."

## Evidence and accuracy
- Evidence: `evidence/near-guard-testnet-live-2026-11-06.md` in Cyptopimpinainteazy/x3lang.
- Testnet contract: `x3lang.testnet`
- Refused testnet intent: `AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8`
- Accepted testnet intent: `DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct`
- **Not proven by these tests:** funded wallet transfers, independent cross-chain settlement, cross-chain causality, or mainnet production readiness.
- Illustrative card 03 proposes 100 USDC. The on-chain refusal in card 06 is a separate real testnet scenario. Do not portray the illustrative amount as the real on-chain transaction.
- Different language paths exist: the NEAR mandate-policy path is not identical to the X3BC trading VM execution path.

## Rebuild
```bash
cd ~/Downloads/X3Lang-Video/premium
../.assets-venv/bin/python render_and_preview.py
../.assets-venv/bin/python shield_animation.py
```
Requires CairoSVG, Pillow and ffmpeg. Source SVGs are also in the GitHub branch `assets/judge-video-diagrams-20261106` under `docs/hackathon/video-assets/premium/`.

All visuals are project illustrations. Verify production claims against code and on-chain receipts before publication.
