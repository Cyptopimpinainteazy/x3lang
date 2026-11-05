"""Focused tests for the public readiness report's error sanitization."""
import unittest
from bnb_gate import extract_result


class BnbReadinessTests(unittest.TestCase):
    def test_provider_minimum_is_extractable_without_raw_response(self):
        raw = '{"errors":[{"code":"X3_NEAR_PROVIDER_MINIMUM","detail":{"providerMinimumUsd":"1000","secret":"never echo"}}]}'
        result = extract_result(raw)
        self.assertEqual(result["kind"], "rejected")
        self.assertEqual(result["provider_minimum_usd"], "1000")
        self.assertNotIn("secret", str(result))
        self.assertNotIn("never echo", str(result))

    def test_plan_proves_only_dry_preparation(self):
        raw = '{"status":"planned","quote_request":{"amount":"1000000000000000","dry":true,"refundTo":"private"},' \
              '"leg":{"origin":{"chain":"bsc","asset":"BNB"},"destination":{"chain":"near","asset":"wNEAR"}}}'
        result = extract_result(raw)
        self.assertEqual(result["kind"], "planned")
        self.assertTrue(result["dry"])
        self.assertEqual(result["source_chain"], "bsc")
        self.assertNotIn("private", str(result))

    def test_malformed_response_fails_closed(self):
        self.assertEqual(extract_result("not-json")["kind"], "unreadable_response")
        self.assertEqual(extract_result(None)["kind"], "unreadable_response")

    def test_unknown_status_does_not_claim_funding(self):
        result = extract_result('{"status":"quote_preview","tx":"forged"}')
        self.assertFalse(result["funded_transfer_proven"])
        self.assertNotIn("forged", str(result))


if __name__ == "__main__":
    unittest.main()
