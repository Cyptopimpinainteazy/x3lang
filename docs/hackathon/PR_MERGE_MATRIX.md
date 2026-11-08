# PR merge matrix — private release coordination

Latest inspected master: `038e82d73a522642ae755af48eb6acb838902329`. Snapshot: November 8, 2026 UTC. **One open PR**, #159. The older 30-PR snapshot below is historical; its dispositions are not current. Current head/checks must be refreshed immediately before a merge.

## Current queue

| PR | Exact head / base | Status / dependency | Review / checks | Decision |
|---|---|---|---|---|
| [#159](https://github.com/Cyptopimpinainteazy/x3lang/pull/159) | `7129cafb77cdbeb41275baf7371239940a34cde2` → master `038e82d73a522642ae755af48eb6acb838902329` | Draft; depends on integrated vault/Guard/source-binding work | Independent merge clearance not established. **Verify identical proof code in both GitHub repos: FAILURE**. Other visible executed checks passed, including compiled-WASM Sandbox lifecycle; manual advisory two-build job skipped | HOLD. Do not bypass parity, invent readiness or modify another worker's branch |

[Latest metadata snapshot](evidence/points-maximizer-20261108/current-pr-snapshot.json). The #159 description reports peer drift in the changed harness and missing `raw_capture.rs`; this continuation has not yet independently inspected that job log. The peer repository is read-only for this sprint, so its owner must authorize/perform an exact matching branch/byte update. No peer writes were made.

## Actual integrated history

Our #128 malformed lifecycle fix (`5df23da8f73edd55a37dd3967cd60b8b3d955ca6`) was independently reviewed with no findings and 12 passing LOCAL tests. Another actor merged it as `90d58d55ade8c19aa42d9a143728096ebc7dbeac` on November 7. **This continuation performed no PR merge.** #118 (`c335fb2`), #124 (`9e53890`), #122 (`ce4cfac`), #126 (`8fa8266`), #135, #151 and #145 were already integrated before the history below.

Other workers subsequently advanced master through these first-parent commits:

```text
038e82d Merge pull request #158 from fix/gauntlet-proof-binding-20261108
63a6f67 Merge pull request #155 from feat/attack-lab-control-room-20261108
45bab7f Merge pull request #120 from fix/bridge-trust-anchor-admission-20261107
e04e27d Merge pull request #156 from fix/vault-review-reapply-20261108
e19d5f2 Merge pull request #150 from fix/gauntlet-mandate-bounds-20261108
9f3e651 Merge pull request #149 from fix/gauntlet-proof-binding-20261108
7993918 Merge pull request #148 from codex/x3-guard-agent-discovery
8860d27 Merge pull request #146 from fix/gauntlet-judge-coverage-20261108
f1d55c3 Merge pull request #123 from docs/base-stocksmaxxing-2026-prep
f8ae6c9 Merge pull request #127 from test/scoped-agent-near-sandbox-20261107
dff231e Merge pull request #154 from feat/vault-origin-two-chain-finality-readonly-20261108
a1692f4 Merge pull request #140 from feat/base-vault-maker-quote-binding-20261107
7db838c Merge pull request #138 from feat/base-sepolia-custody-vault-20261107
13a835d Merge pull request #117 from fix/vm-bridge-config-failclosed-20261107
d2cae19 Merge pull request #137 from fix/agent-vault-owner-escrow-recovery-20261107
88744ac Merge pull request #130 from fix/agent-vault-bounded-payment-history-20261107
43b1f38 Merge pull request #125 from feat/shield-nonbypassable-near-vault-20261107
f139b68 Merge pull request #105 from agent/guard-race-stress-20261106
09980d5 Merge pull request #144 from fix/gauntlet-base58-bounds-20261108
2f14298 Merge pull request #112 from agent/near-proof-crash-recovery-20261107
25a8e11 Merge pull request #116 from fix/bridge-explicit-rpc-threshold-20261107
f72ebee Merge pull request #132 from agent/guard-node-restart-20261107
c8dfa4c Merge pull request #108 from agent/guard-deposit-collision-20261106
127d5a3 Merge pull request #103 from agent/guard-upgrade-replay-20261106
ac30c8b Merge pull request #102 from agent/guard-quote-replay-audit-20261106
74d7496 Merge pull request #101 from agent/guard-heavy-attacks-20261106
bffb940 Merge pull request #98 from agent/guard-attack-matrix-20261106
3c87676 Merge pull request #94 from agent/security-redteam-breakfix-20261106
59ec527 Merge pull request #133 from fix/guard-cfg-near-readable-refusals-20261107
277cba0 docs: separate judge verification paths for policy, Guard, and settlement (#147)
508e9b6 fix(ci): fail closed when allocator dependency inspection fails (#143)
```

This log proves integration, not that this agent independently approved every financial/security change or reproduced each old CI run. The current exact-commit release gate is required after integration.

## Historical dependency tree (before the later merge wave)

```text
master
  #94 -> #98 -> #102 -> #103 -> #108 -> #132
              -> #101 (overlapped #105)
              -> #105
  #117 -> #120 (#116 overlapped #117)
  #125 -> #130 -> #136 / #137 (recovery alternatives)
       -> #134 (overlapped #130)
  #138 -> #140 -> #142
  #112, #123, #127, #133, #143, #144, #146, #148, #149, #150
  merged #145 branch -> #147
```

Every arrow was the declared PR base branch. Historical exact heads/check states are retained in [the 30-PR snapshot](evidence/points-maximizer-20261108/pr-snapshot.json). Do not use its old failed runs as the state of later heads. This agent closed no duplicates; no unreached commit was discarded.

## Historical 30-PR table — superseded, not current merge advice

| PR | Exact head | Base/dependency | Draft/readiness | Review | Gates | Risk/overlap | Disposition |
|---|---|---|---|---|---|---|---|
| [#94](https://github.com/Cyptopimpinainteazy/x3lang/pull/94) | `6bf0cb80580e44b1f9eb2b763d038a40d1b89631` | `master` (master) | DRAFT | Fresh independent clearance not established | real-deployed-wasm-exact-byte-proof (FAILURE); Verify identical proof code in both GitHub repos (FAILURE) | Guard expiry; root of Guard stack | HOLD: failed checks; head reachable: False |
| [#98](https://github.com/Cyptopimpinainteazy/x3lang/pull/98) | `cb63eb4a3a3e09d5df34a85702da8f762607722d` | `agent/security-redteam-breakfix-20261106` (#94) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Sandbox/mutations; overlaps #101/#105 | HOLD: failed checks; head reachable: False |
| [#101](https://github.com/Cyptopimpinainteazy/x3lang/pull/101) | `2ab303712f393a6c7217c43eb38a5ffb8973e7b1` | `agent/guard-attack-matrix-20261106` (#98) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Nonce stress; overlaps #105 | HOLD: failed checks; head reachable: False |
| [#102](https://github.com/Cyptopimpinainteazy/x3lang/pull/102) | `1135569ec733d0b8ecd2043e60e9c6e94ac78132` | `agent/guard-attack-matrix-20261106` (#98) | DRAFT | Fresh independent clearance not established | real-deployed-wasm-exact-byte-proof (FAILURE); Verify identical proof code in both GitHub repos (FAILURE) | Quote replay state/migration | HOLD: failed checks; head reachable: False |
| [#103](https://github.com/Cyptopimpinainteazy/x3lang/pull/103) | `b1270f47e66117e39a85b7773582f59339df8566` | `agent/guard-quote-replay-audit-20261106` (#102) | DRAFT | Fresh independent clearance not established | real-deployed-wasm-exact-byte-proof (FAILURE); Verify identical proof code in both GitHub repos (FAILURE) | Legacy replay-history migration | HOLD: failed checks; head reachable: False |
| [#105](https://github.com/Cyptopimpinainteazy/x3lang/pull/105) | `426a953c4824eedf5208e351eb869f4e80b8469b` | `agent/guard-attack-matrix-20261106` (#98) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Repeated nonce races/owner isolation | HOLD: failed checks; head reachable: False |
| [#108](https://github.com/Cyptopimpinainteazy/x3lang/pull/108) | `ea1796f24459f3138bdec7b5d3f2401e64827fe4` | `agent/guard-upgrade-replay-20261106` (#103) | DRAFT | Fresh independent clearance not established | real-deployed-wasm-exact-byte-proof (FAILURE); Verify identical proof code in both GitHub repos (FAILURE) | Deposit-address collision protection | HOLD: failed checks; head reachable: False |
| [#112](https://github.com/Cyptopimpinainteazy/x3lang/pull/112) | `d8063394ccd66fa728ba65142581df466de1bc72` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Proof persistence/RPC quorum | HOLD: failed checks; head reachable: False |
| [#116](https://github.com/Cyptopimpinainteazy/x3lang/pull/116) | `1a3653ec3f5889b438e5674f96ac6a6b73e5cf1b` | `master` (master) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | VM RPC/thresholds; overlaps #117 | HOLD: draft/review/dependencies; head reachable: False |
| [#117](https://github.com/Cyptopimpinainteazy/x3lang/pull/117) | `ac407564d32b50c4785d182eb128f81e35f1ea26` | `master` (master) | DRAFT | 2 documentation findings unresolved | No failed reported check in snapshot | VM bootstrap; overlaps #116; parent of #120 | HOLD: draft/review/dependencies; head reachable: False |
| [#120](https://github.com/Cyptopimpinainteazy/x3lang/pull/120) | `81d534fd6d60aea499f74f71f27a4625f6910d1f` | `fix/vm-bridge-config-failclosed-20261107` (#117) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Trust anchors; requires #117 | HOLD: draft/review/dependencies; head reachable: False |
| [#123](https://github.com/Cyptopimpinainteazy/x3lang/pull/123) | `6a2b108fecc1d3b6ddae14f4e3048b9394766f2d` | `master` (master) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Separate contest; outside NEAR sprint | HOLD: draft/review/dependencies; head reachable: False |
| [#125](https://github.com/Cyptopimpinainteazy/x3lang/pull/125) | `a9f0e4169969c5009b0dc2052890746aff4b54b0` | `master` (master) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | NEAR custody; parent of #130/#134 | HOLD: draft/review/dependencies; head reachable: False |
| [#127](https://github.com/Cyptopimpinainteazy/x3lang/pull/127) | `7146a066b4bfc2e5a703bf67c6c41d244e1436d3` | `master` (master) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Protocol access-key adversarial proof | HOLD: draft/review/dependencies; head reachable: False |
| [#130](https://github.com/Cyptopimpinainteazy/x3lang/pull/130) | `b447d25d2b83e92f5ff68baf31ff7d8ece668650` | `feat/shield-nonbypassable-near-vault-20261107` (#125) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Vault bounded storage; overlaps #134 | HOLD: failed checks; head reachable: False |
| [#132](https://github.com/Cyptopimpinainteazy/x3lang/pull/132) | `79ffd275fa03a1102c408a60e83da0d6dcf4eb50` | `agent/guard-deposit-collision-20261106` (#108) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Guard persisted restart; requires #108 | HOLD: failed checks; head reachable: False |
| [#133](https://github.com/Cyptopimpinainteazy/x3lang/pull/133) | `9a86d3ec4d1a2d2d6633d7d4f48c831f4a10e69a` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Guard on-chain build/refusal diagnostics | HOLD: failed checks; head reachable: False |
| [#134](https://github.com/Cyptopimpinainteazy/x3lang/pull/134) | `2acfff219c6f2f6f4ae83230b729a2ebe7b99e8f` | `feat/shield-nonbypassable-near-vault-20261107` (#125) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Vault economics/recovery; overlaps #130 | HOLD: failed checks; head reachable: False |
| [#136](https://github.com/Cyptopimpinainteazy/x3lang/pull/136) | `850c18e80b80ed062e6be3d94eed721322b14f3b` | `fix/agent-vault-bounded-payment-history-20261107` (#130) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Owner recovery; overlaps #137 | HOLD: failed checks; head reachable: False |
| [#137](https://github.com/Cyptopimpinainteazy/x3lang/pull/137) | `8b9650b3a5c0daf319c4eedda7f738701afbbde0` | `fix/agent-vault-bounded-payment-history-20261107` (#130) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE) | Owner recovery+Sandbox; overlaps #136 | HOLD: failed checks; head reachable: False |
| [#138](https://github.com/Cyptopimpinainteazy/x3lang/pull/138) | `8b1ab64780b392d16e21d30efe600a094fafe188` | `master` (master) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Base custody; parent of #140 | HOLD: draft/review/dependencies; head reachable: False |
| [#140](https://github.com/Cyptopimpinainteazy/x3lang/pull/140) | `096e2692b72ac225bc36b8a26f79b7cfca6b1a91` | `feat/base-sepolia-custody-vault-20261107` (#138) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Signed-quote/Base binding; requires #138 | HOLD: draft/review/dependencies; head reachable: False |
| [#142](https://github.com/Cyptopimpinainteazy/x3lang/pull/142) | `1907df570daa3035be538945095cd84a059c0bd7` | `feat/base-vault-maker-quote-binding-20261107` (#140) | DRAFT | Fresh independent clearance not established | No failed reported check in snapshot | Two-provider Guard quorum; requires #140 | HOLD: draft/review/dependencies; head reachable: False |
| [#143](https://github.com/Cyptopimpinainteazy/x3lang/pull/143) | `5c3171f0b74b088417cf7e73cdd0b7634ed3a690` | `master` (master) | READY | P1 unresolved | recurseml/analysis (ERROR) | Allocator inspection; unresolved P1 | HOLD: failed checks; head reachable: False |
| [#144](https://github.com/Cyptopimpinainteazy/x3lang/pull/144) | `b817f17343e3583aa32f1443e727e1bd566bc665` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE); recurseml/analysis (ERROR) | Base58 input bounds | HOLD: failed checks; head reachable: False |
| [#146](https://github.com/Cyptopimpinainteazy/x3lang/pull/146) | `38ccade0b58edf840cbf1a4b720790cdf819eb0c` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE); recurseml/analysis (ERROR) | Full-mode quick-test coverage | HOLD: failed checks; head reachable: False |
| [#147](https://github.com/Cyptopimpinainteazy/x3lang/pull/147) | `ef8df59fb54344577d5d33734f0cd0bb5729594d` | `docs/no-ai-slop-20261107` (merged #145 branch) | DRAFT | Fresh independent clearance not established | recurseml/analysis (ERROR) | Judge docs; base branch #145 already merged | HOLD: failed checks; head reachable: False |
| [#148](https://github.com/Cyptopimpinainteazy/x3lang/pull/148) | `11a9151f9b2c7be2366c9d24410324970d271f0c` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE); recurseml/analysis (ERROR) | Demo discovery; privacy-sensitive hosting | HOLD: failed checks; head reachable: False |
| [#149](https://github.com/Cyptopimpinainteazy/x3lang/pull/149) | `aeacc392f0b9459bf4ef1ff3a325d850b2b711d7` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE); recurseml/analysis (ERROR) | Proof source binding; overlaps merged #151 | HOLD: failed checks; head reachable: False |
| [#150](https://github.com/Cyptopimpinainteazy/x3lang/pull/150) | `b08a002b866c88ddd27386923b43f609b3057144` | `master` (master) | DRAFT | Fresh independent clearance not established | Verify identical proof code in both GitHub repos (FAILURE); recurseml/analysis (ERROR) | Mandate numeric/expiry bounds | HOLD: failed checks; head reachable: False |


## What changed since the historical failures

- The old #112 parity job (`42350851642`, job `128338460443`) failed because the peer branch did not exist; its credential-presence line was shell source, not proof that a secret was absent. #112 was subsequently integrated by another actor. Current #159 still fails parity and remains held.
- The old #94 byte job compiled 545017 bytes and failed against deployed digest `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`. Current reproducibility code explicitly rebuilds deployed source `800a077eec0b36bc66d7115995cb6903fa9d4f0a` from Git history. That proves historical bytes only; changed HEAD is not thereby deployed. No new deployment was authorized.
- #143 was revised to match both TOML quote styles and merged. #146 is now integrated; full mode includes the quick-only script test and all browser files. #117 and #120 are also integrated. Historical draft/review findings must not be restated as current open PR blockers.
- The safety of newly merged financial/security surfaces remains subject to current tests, review and appropriate real infrastructure evidence. A merge event alone is not that evidence.

Privacy: GitHub unexpectedly reported X3Lang public on November 8. On explicit owner instruction, this agent restored it to private and verified both X3Lang and xxxstar private. Prior exposure cannot be undone. No publication, visibility change, deployment, signing or submission is now authorized.
