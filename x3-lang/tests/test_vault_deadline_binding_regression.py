"""Regression for the Nov 14 vault receipt's conflicting authorization deadline.

The receipt from the separately published vault pilot has a signed 1Click
request expiry on Nov 14, but the compiled .x3 leg expires on Nov 17.
Do not silently normalize this mismatch: the request signature does not
authorize an extension of the compiled leg.
"""
import json

import pytest

from test_chain_evidence import live_receipt
from test_near_intents import ni, _trust_the_test_key  # noqa: F401


def test_generic_verifier_accepts_matching_request_and_leg(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    assert ni._signed_quote(receipt)["depositAddress"] == receipt["deposit"]["address"]


@pytest.mark.parametrize("delta", ["2026-11-17T13:31:48.804Z", "2026-11-14T13:31:48.804Z"])
def test_generic_verifier_rejects_unapproved_leg_deadline_extension(tmp_path, delta):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    # The quote and saved signed request are left intact. Only the leg
    # (which can originate from compiled source) claims a later expiry.
    receipt["leg"]["deadline"] = delta
    with pytest.raises(ni.NearIntentsError) as exc:
        ni._signed_quote(receipt)
    assert exc.value.code == "X3_EVIDENCE_BINDING"
    assert "deadline" in str(exc.value).lower()


def test_signed_request_cannot_be_changed_to_match_leg_after_quote(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    changed = "2026-11-17T13:31:48.804Z"
    receipt["leg"]["deadline"] = changed
    receipt["quote_request"]["deadline"] = changed
    with pytest.raises(ni.NearIntentsError) as exc:
        ni._signed_quote(receipt)
    assert exc.value.code == "X3_EVIDENCE_BINDING"
