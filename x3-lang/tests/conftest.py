"""Shared test isolation for the x3-lang suite."""
import pytest


@pytest.fixture(autouse=True)
def _no_live_shield_from_the_environment(monkeypatch):
    """A developer's NEAR Intents Shield token must never make a test reach the network.

    Tests that exercise the incident check inject a fake `ShieldIncidents` explicitly.
    """
    for name in ("NEAR_INTENTS_SHIELD_JWT", "NEAR_INTENTS_SHIELD_URL", "X3_REQUIRE_INCIDENT_CHECK"):
        monkeypatch.delenv(name, raising=False)
