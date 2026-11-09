# X3Lang / X3 Shield — Judge video assets, batch 2

Prepared November 8, 2026. Sixteen 16:9 source SVGs, with an automated x3star1 builder for PNG, six MP4 previews, a gallery and ZIP.

## Scene list
10. Owner-defined `.x3` financial mandate
11. Mandate parser and structured rules
12. Trading compiler semantic and admission checks
13. Canonical X3BC shared semantic core
14. X3VM deterministic interpreter and transactional host (architecture)
15. Separate NEAR Shield mandate path and X3 trading VM path
16. AI intent proposed; policy independently refuses an unauthorized destination
17. Official manager-signed NEAR 1Click quote verification
18. NEAR Guard allow/refuse decision on signed terms
19. Deposit authorization is **not** a funded transfer
20. **Separate** Tidepool-funded Base Sepolia → NEAR testnet pilot
21. Historical NEAR Guard authorization preceding Base transfer
22. Finalized Base Sepolia source transaction: 1.000000 test USDC
23. Finalized NEAR payout transaction: 0.997400 test USDC
24. What is proven versus trusted relay / unproven atomicity
25. Judge reproducibility map

## Key truth boundaries
- NEAR Shield's typed `.x3` mandate/Guard path is distinct from X3BC trading bytecode and X3VM. Native X3VM execution on NEAR is **not** claimed.
- The official signed 1Click **live** quote demo authorized a deposit address but **stopped before funds moved**. The refused official quote was a **dry quote**.
- A separate Tidepool community testnet pilot delivered Base Sepolia **1.000000 test USDC** and NEAR **0.997400 test USDC**, recorded on both finalized chains. It relied on a **trusted relayer**, not cryptographic cross-chain atomicity.
- An agent can bypass this Guard with an unrelated unrestricted wallet key; this product is not universal wallet-level custody enforcement.

## Canonical evidence
- `README.md` in x3lang
- `docs/hackathon/JUDGE_GUIDE.md`
- `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- `docs/hackathon/FIRST_FINALIZED_SWAP_EVIDENCE.md`
- `evidence/near-guard-testnet-live-2026-11-06.md`
- Base TX: `0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9`
- NEAR payout TX: `EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d`

## Download and render on x3star1
```bash
curl -fL https://raw.githubusercontent.com/Cyptopimpinainteazy/x3lang/assets/judge-video-diagrams-20261106/docs/hackathon/video-assets/batch2/build_batch2.sh -o /tmp/build_x3lang_batch2.sh
bash /tmp/build_x3lang_batch2.sh
xdg-open ~/Downloads/X3Lang-Video/batch2/index.html
```

The script reuses your existing `.assets-venv` and ffmpeg if available; it installs nothing system-wide. It renders all 16 images to 1920×1080 PNG and three **silent** 1280×720 video reels for review. Source SVGs remain at full 1920×1080 canvas size. Use the PNG/SVG masters for final HD/4K exports.

**Scope:** Technical explanatory B-roll, not a recording of on-chain execution. The animation transitions between static diagrams, and some SVGs contain decorative CSS pulses; these are not transaction traces.

Remote Desktop Commander for x3star1 was online but its monthly tool-call limit was reached, so this batch is published through the GitHub asset branch and requires the explicit user-run bootstrap above. GitHub master is unchanged.
## Animated micro-clips
The renderer also creates three 4-second 720p moving-pulse clips: `X3BC_Bytecode_Pulse_4s.mp4`, `X3Shield_Guard_Decision_4s.mp4`, and `Tidepool_Trusted_Relay_4s.mp4`. They are illustrative motion graphics layered over the diagrams, **not live transaction playback**.
