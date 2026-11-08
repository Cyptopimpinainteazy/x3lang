# Base Stocks Maxxing 2026 — private application draft

> Internal preparation only. **Do not publish, register, submit the form, post on X, or make the repository public** without owner review. This is a proposed **new** Base-specific stock-linked feature, not a claim that tokenized-stock trading is already implemented.

## Opportunity

- Official event: https://www.base.org/stocksmaxxing
- Official rules: https://www.base.org/stocksmaxxing/rules
- Application: https://docs.google.com/forms/d/e/1FAIpQLSeS8uKavZzuH9-vB6bq3oaOsnxTDsrcLtekLBipzW-9x2A3HQ/viewform
- Event window: **2026-11-05 through 2026-12-05**; check website/form for the applicable submission timezone.
- Preferred track: **Agentic Stock Finance**, subject to final eligibility review.
- Critical restriction on application: **projects that enable stock trading for U.S. users are not in scope**. The proposed demo will evaluate stock-linked intents and **never enable real stock trades for U.S. users**.
- Rules permit existing owned code, but require a clear account of **work actually built during the Quest Period**; testnet is permitted.
- Finishing the final entry requires a **public X video demo tagging @buildonbase** and the official final submission form. Automated contest submissions are prohibited. Neither action has happened.

## Proposed entry (not yet shipped)

**Project name:** X3 Shield — Agentic Stock Guard

**One–two sentence description for application:**
> X3 Shield is a programmable authorization and evidence layer for autonomous financial agents. For Base Stocks Maxxing, we are building a **new**, Base-specific tokenized-stock intent evaluation prototype that checks instrument identity, owner mandates, recipient/allowance bounds, risk constraints and replay protection, then generates an auditable allow/deny record **without enabling stock trading for U.S. users**.

**Why select us (draft, edit after prototype ships):**
> We already have an experimental typed `.x3` policy engine, Guard controls and a separately verified Base Sepolia-to-NEAR testnet execution using an external relayer. Our competition work will add the missing stock-specific Base path and show the same adversarial agent request under deterministic authorization with transparent evidence and fail-closed behavior. Judges can see what was pre-existing, what we added during the contest, what failed safely, and what is still unproven. We will not claim a permissionless cross-chain bridge or a production brokerage.

**Most valuable support:** technical review of the tokenized-stock asset model and Base integration, trusted partner introduction, Base infrastructure credits, legal/compliance orientation, audit-readiness review. No endorsement or partner agreement assumed.

## Application form fields (verified 2026-11-07)

| Form question | Draft answer / needed action |
| --- | --- |
| Email | **josh@x3star.net** (user's preferred X3 correspondence address; verify receipt capability) |
| Project name | X3 Shield — Agentic Stock Guard |
| Full name | Josh Jacobsen (from connected GitHub profile; confirm legal application name before submission) |
| Email address | josh@x3star.net (confirm inbox is live) |
| X handle | @x3starchain |
| Describe in 1–2 sentences | Draft above |
| Current stage | Prototype for underlying X3 Shield; **new stock-specific feature is planned** |
| Does your project have a token? | **UNCONFIRMED — owner must select Yes/No accurately** |
| Entry type | Build New — *provisional; eligibility and pre-existing token rules need review* |
| Intended Base track | Agentic Stock Finance |
| Launch partners | **UNSELECTED**. Potential discussions: Bankr / o1 Exchange / Stonks Exchange, subject to actual fit; do not imply partnerships |
| Support requested | Draft above |
| Product/demo URL | N/A until a Base stock-feature demo exists |
| Why should we select you? | Draft above, revise to reflect delivered work |
| Share contact/project details with partners? | **Owner decision required** |
| Read rules, arbitration and conditions | **Owner must review and accept** |
| Acknowledge no US stock-trading projects | **Owner must review and accept** |

Other identifiers for materials (not official form fields): Telegram https://t.me/x3starchain ; GitHub https://github.com/Cyptopimpinainteazy/x3lang (currently **private**). Do not submit private GitHub as a publicly accessible judge URL.

## Existing evidence — do not inflate

- Read `docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md` for the historical owner-authorized Base Sepolia ERC-20 transfer and NEAR testnet payout via Tidepool.
- That is **not** a tokenized-stock trade, production X3DEX execution, or proof of native trustless atomic settlement.
- `x3DEX` is a separate project whose README labels wallet signing and trade execution disabled. Its read-only prices and charts are not executable liquidity.
- `README.md` documents NEAR mandate checks and Guard behavior, with explicit local vs public testnet labels.
- Dates, source refs, artifacts and any novel Base-specific commits must be cited in the eventual submission. Retain an immutable exact-head tag/evidence bundle after acceptance tests.

## Submission gates

1. Owner verifies personal/team eligibility, token/project history, partner disclosures, participation limits, stock-trading restriction, rules, arbitration and use of public materials.
2. Ship an **actually new** Base stock-linked feature in the Quest window, backed by source, working tests and meaningful Base integration.
3. Pin actual Base contract/network/token metadata and independently rerun the proof. Reject fake/stale assets and RPC-only assertions beyond proof scope.
4. Record safety negative tests, exact commit SHA, tests and build output; distinguish local unit tests from public testnet observations.
5. Prepare and film the Base-only demonstration. Record a real product UI/CLI, not an animation of an unbuilt system.
6. Owner approves public disclosure; publish X video tagging `@buildonbase` and submit the official form **manually** during the window.
7. Save the actual form confirmation and submission URL; status remains **NOT REGISTERED** until confirmed.

**Internal readiness now:** application copy drafted; registration not attempted; stock-linked feature and new video not built.