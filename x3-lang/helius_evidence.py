"""Read-only Helius Parsed Events cross-check for X3 finalized Solana RPC evidence.

Helius parsing is *not* a cryptographic chain proof. A verified SolanaRpc
getTransaction verdict is required first; any disagreement fails closed.
No wallets, signing, transfer submission, or transactions are performed here.
"""
import hashlib
import json
import os
import urllib.error
import urllib.parse
import urllib.request


class HeliusEvidenceError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code

    def to_dict(self):
        return {"code": self.code, "message": str(self)}


def _http_transport(method, url, headers, body, timeout):
    try:
        request = urllib.request.Request(url, data=body, method=method, headers=headers)
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as exc:
        # Do not include the URL, whose query string contains a credential.
        return exc.code, b""
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise HeliusEvidenceError("X3_HELIUS_UNAVAILABLE", f"Helius transport failed: {type(exc).__name__}")


def _units(value):
    if isinstance(value, bool) or not isinstance(value, (int, str)):
        raise HeliusEvidenceError("X3_HELIUS_FORMAT", "transfer rawTokenAmount is not integer base units")
    text = str(value)
    if not text.isdigit():
        raise HeliusEvidenceError("X3_HELIUS_FORMAT", "transfer rawTokenAmount is not integer base units")
    return int(text)


class ParsedEventsWitness:
    """Request one parsed signature, cross-check against an already-verified RPC receipt."""

    ENDPOINT = "https://mainnet.helius-rpc.com/v1/parsed-events/transactions"

    def __init__(self, *, api_key=None, transport=None, timeout=20.0):
        key = api_key if api_key is not None else os.environ.get("HELIUS_API_KEY")
        if not key:
            raise HeliusEvidenceError("X3_HELIUS_CONFIG", "HELIUS_API_KEY is required for the Helius evidence gate")
        self._api_key = key
        self.transport = transport or _http_transport
        self.timeout = timeout

    def attest(self, *, signature, mint, owner, min_amount, canonical):
        if not isinstance(canonical, dict) or canonical.get("verified") is not True:
            raise HeliusEvidenceError("X3_HELIUS_RPC_REQUIRED", "finalized Solana RPC proof must verify first")
        if canonical.get("signature") != signature or canonical.get("mint") != mint or canonical.get("owner") != owner:
            raise HeliusEvidenceError("X3_HELIUS_MISMATCH", "RPC evidence concerns another transaction or asset")
        if canonical.get("commitment") != "finalized":
            raise HeliusEvidenceError("X3_HELIUS_RPC_REQUIRED", "RPC proof is not finalized")
        if not isinstance(canonical.get("slot"), int) or not isinstance(canonical.get("block_time"), int):
            raise HeliusEvidenceError("X3_HELIUS_RPC_REQUIRED", "RPC proof is missing slot or block time")
        try:
            observed_rpc = _units(canonical["observed"])
        except (KeyError, TypeError):
            raise HeliusEvidenceError("X3_HELIUS_RPC_REQUIRED", "RPC evidence has no observed base-unit amount")
        if observed_rpc < min_amount:
            raise HeliusEvidenceError("X3_HELIUS_RPC_REQUIRED", "RPC amount is below the required deposit")

        request = json.dumps({"transactions": [signature], "rawTransaction": "none"}).encode("utf-8")
        url = self.ENDPOINT + "?api-key=" + urllib.parse.quote(self._api_key, safe="")
        try:
            status, response_body = self.transport("POST", url,
                {"Content-Type": "application/json", "Accept": "application/json"}, request, self.timeout)
        except HeliusEvidenceError:
            raise
        except Exception as exc:
            # Never interpolate transport exceptions: they can include the credential-bearing URL.
            raise HeliusEvidenceError("X3_HELIUS_UNAVAILABLE", f"Helius request failed: {type(exc).__name__}")
        if status != 200:
            raise HeliusEvidenceError("X3_HELIUS_HTTP", f"Helius returned HTTP {status}")
        try:
            response = json.loads(response_body)
        except (ValueError, UnicodeDecodeError, TypeError):
            raise HeliusEvidenceError("X3_HELIUS_FORMAT", "Helius did not return valid JSON")
        if not isinstance(response, list) or len(response) != 1:
            raise HeliusEvidenceError("X3_HELIUS_FORMAT", "Helius must return exactly one parsed transaction")
        tx = response[0]
        if not isinstance(tx, dict) or tx.get("signature") != signature or tx.get("parserStatus") != "OK":
            raise HeliusEvidenceError("X3_HELIUS_MISMATCH", "Helius did not parse the requested signature")
        parsed = tx.get("parsed")
        if not isinstance(parsed, dict) or parsed.get("transactionStatus") != "OK" or parsed.get("error") is not None:
            raise HeliusEvidenceError("X3_HELIUS_MISMATCH", "Helius did not attest a successful transaction")
        if parsed.get("slot") != canonical["slot"] or parsed.get("blockTime") != canonical["block_time"]:
            raise HeliusEvidenceError("X3_HELIUS_MISMATCH", "Helius slot or block time disagrees with finalized Solana RPC")
        transfers = parsed.get("tokenTransfers")
        if not isinstance(transfers, list):
            raise HeliusEvidenceError("X3_HELIUS_FORMAT", "Helius returned no tokenTransfers list")

        net_amount = 0
        for transfer in transfers:
            if not isinstance(transfer, dict):
                raise HeliusEvidenceError("X3_HELIUS_FORMAT", "invalid Helius transfer entry")
            if transfer.get("mint") != mint:
                continue
            incoming = transfer.get("toUserAccount") == owner
            outgoing = transfer.get("fromUserAccount") == owner
            if incoming or outgoing:
                amount = _units(transfer.get("rawTokenAmount"))
                net_amount += amount * (int(incoming) - int(outgoing))
        if net_amount < min_amount or net_amount != observed_rpc:
            raise HeliusEvidenceError("X3_HELIUS_MISMATCH",
                                     "Helius net token transfer disagrees with the finalized RPC evidence")

        return {
            "verified": True,
            "kind": "helius_parsed_events_crosscheck",
            "signature": signature,
            "mint": mint,
            "owner": owner,
            "slot": parsed["slot"],
            "block_time": parsed["blockTime"],
            "raw_token_net_amount": str(net_amount),
            "response_sha256": hashlib.sha256(response_body).hexdigest(),
            "note": "Provider parsing cross-check only; finalized Solana RPC is the canonical chain evidence.",
        }
