# X3 Shield — two-minute judge video storyboard (pre-publication)

**Status:** private recording plan; NOT a completed or published video. Do not expose the private repository or testnet signing credentials before the approved submission reveal.

## 0:00–0:20 — The problem

Visual: X3 Shield Attack Lab hero; show that a compromised AI agent can request payments.

Voice: "AI agents will eventually move real money. A signed instruction is not enough. X3 Shield evaluates actual quote terms against the owner's policy—and we are building enforceable custody boundaries so a compromised agent cannot ignore the limits."

## 0:20–0:48 — Verified real testnet trade

Visual: the real Base Sepolia receipt and signed quote; show 1.000000 test USDC transfer and the Guard policy hash.

Voice: "This is a real Base Sepolia test-USDC transfer, initiated only after a live Tidepool quote was signed and matched against an owner-authorized, one-use NEAR Guard mandate. The exact authorization was finalized on NEAR before the source transaction."

## 0:48–1:08 — NEAR receipt and proof

Visual: actual finalized NEAR transaction, terminal receipt, 100.000000 to 100.997400 test-USDC balance increase, strict two-chain proof JSON.

Voice: "Two independent providers verified the finalized source receipt. NEAR finalized the destination transfer of 0.997400 test USDC. The historical token balance change matches the signed quote. The verifier keeps a strict record of what was proven."

## 1:08–1:40 — Compromised-agent attack matrix

Visual: the browser offline-preview panel followed by the real Sandbox command/output on x3star1.

Voice: "Our next defense moves beyond advisory checking. A disposable agent holds only a scoped NEAR FunctionCall key. The vault itself restricts recipients, payment caps, budget, nonce and revocation. In a real local Sandbox, the allowed payment succeeded, while wrong recipient, oversize, replay and revoked calls failed."

## 1:40–2:00 — Distinguish the proof levels and finish

Visual: two evidence columns, real testnet Tidepool transfer and separate local custody-vault attack proof, then the source README and judge one-command path.

Voice: "We're precise about the boundary: the real trade uses a trusted Tidepool testnet relayer. The custody vault is a separate local proof, not a public deployed EVM smart account and not a trustless atomic bridge. Judges can inspect each receipt, test the offline policy UI, and rerun the code."

## Recording checklist

- Capture actual explorer pages, not fabricated transaction screenshots.
- Show the valid signature and historical approval on the same finalized chain.
- Show source ERC-20 transfer, destination NEP-141 ft_transfer and actual balance delta.
- Show real local Sandbox marker and successful command exit, not the JavaScript preview as on-chain proof.
- If including the separately reviewed 99.3247% validator stake-verification claim, first rerun/check that exact light-client evidence and identify its block and validator set.
- Record with sensitive wallet/SSH files and terminals closed; do not show keys, passwords or recovery phrases.
- Publish the repo, X video/post and submit only on the owner-approved release day before the official deadline.
