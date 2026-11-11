# 1Click Explorer evidence

This read-only Node 22+ client retrieves historical 1Click transactions. It does
not request quotes, authorize deposits, sign transactions, or prove settlement.
Every response is `provider-reported`, with `independentlyVerified: false` and
`executionAllowed: false`, even when the provider reports SUCCESS.

## Configuration

Inject these server-only environment variables using your deployment secret manager:

- `NEAR_INTENTS_EXPLORER_JWT`: Explorer credential from the partner dashboard.
  This is separate from the swap adapter's `NEAR_INTENTS_JWT`.
- `X3_EXPLORER_REDIS_URL`: HTTPS Redis REST endpoint supporting JSON commands.
- `X3_EXPLORER_REDIS_TOKEN`: authentication for that Redis service.
- `X3_EXPLORER_SEARCH`: owner-approved public search scope (a deposit address,
  recipient, sender, or transaction hash). No unscoped public history is served.

Do not paste credentials into source, browser configuration, logs, or examples.
A credential previously shared in chat should be replaced before deployment;
revocation/rotation remains an explicitly owner-approved action.

All x3lang and dotx3 instances using the same partner MUST use the same Redis
service. The limiter derives its namespace from the JWT's partner_id, including
across credential rotation. Decoding is only for namespace selection, not JWT
signature validation: the official provider authenticates the credential.
Other applications making direct requests with that partner credential are
outside this limiter; coordinate them too. Missing Redis fails closed.

The client caches each validated query for 30 seconds, takes an atomic shared
lock before contacting the provider, and retains at least five seconds of
cooldown after completion. HTTP 429 extends cooldown using Retry-After.
There is no background polling or automatic retry storm.

## Read history

```sh
node x3-lang/near/explorer/cli.mjs 'statuses=SUCCESS'
node --test x3-lang/near/explorer/client.test.mjs
```

Supported filters: statuses (one official status), fromChainId, toChainId,
lastDepositAddress, lastDepositMemo. Page size is 25. Pass both cursor fields
when the returned nextCursor contains a memo. A full final page can offer one
extra next request that returns an empty page.

The server-controlled search is the provider's search operation, NOT an exact
ownership proof or access-control boundary. Review what it matches before
making results public. Only selected transaction fields are exposed, never raw
provider errors or authentication data. Do not use confidential records here.

## dotx3

The matching client is vendored in dotx3 `src/lib/explorer/client.mjs`.
Its `/api/explorer/transactions` endpoint feeds the `/proof` history panel.
Keep both copies identical, including tests. Use the same Redis deployment and
approved scope. No environment setup, credential use, or deployment is implied
by committing this implementation.

## Evidence boundaries

An empty result is not proof that no swap occurred. FAILED does not mean REFUNDED.
A provider record does not bind a trade to an X3 mandate or demonstrate Guard
acceptance. Link independent source/destination verification separately; never
upgrade this payload's flags based on its status alone. Historical Tidepool
community-testnet evidence remains separate from official 1Click records.

Sources inspected: official Explorer OpenAPI and documentation, 2026-11-11.
https://explorer.near-intents.org/api/v0/openapi.yaml
https://docs.near-intents.org/integration/distribution-channels/1click-api/explorer/introduction
