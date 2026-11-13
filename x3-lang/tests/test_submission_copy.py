"""The owner's submission text must fit the organizer's form and posting rules.

Rules from https://nearhackathon.com/HACKATHON.md (re-read 2026-11-13): `idea` is
20-2,000 characters, `tracks` are organizer track IDs, and the public X post says the
project was built at NEAR HACKATHON and tags @NEARProtocol and #NEARHackathon.
Browsers submit form text with CRLF line breaks and a JavaScript backend counts UTF-16
code units, so the description must fit under every one of those counts.
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUBMISSION = ROOT / "submission" / "submission-copy.md"
X_POSTS = ROOT / "submission" / "x-post-draft.md"
README = ROOT / "README.md"
CHECKLIST = ROOT / "docs" / "hackathon" / "SUBMISSION_CHECKLIST.md"
REPOSITORY_URL = "https://github.com/Cyptopimpinainteazy/x3lang"

ORGANIZER_TRACKS = {
    "defi", "thesis", "privacy", "business", "swarms", "security", "agents",
    "realworld", "games", "nfts", "memecoins", "social", "open",
}
IDEA_MIN, IDEA_MAX = 20, 2000
X_LIMIT = 280
X_URL_WEIGHT = 23  # X shortens every link to t.co and counts it as 23 characters.
URL = re.compile(
    r"https?://\S+|\b[\w-]+(?:\.[\w-]+)*\.(?:com|net|org|io|ai|xyz|dev|app)\b(?:/\S*)?"
)


def _section(text: str, heading_prefix: str) -> str:
    """Body of the first `## ` section whose heading starts with heading_prefix."""
    match = re.search(
        rf"^## {re.escape(heading_prefix)}[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S
    )
    assert match, f"missing section starting with '## {heading_prefix}'"
    return match.group(1).strip()


def _description() -> str:
    text = SUBMISSION.read_text(encoding="utf-8")
    return _section(text, "20–2,000 character description")


def _utf16_length(text: str) -> int:
    return len(text.encode("utf-16-le")) // 2


def _x_length(post: str) -> int:
    return len(URL.sub("x" * X_URL_WEIGHT, post))


def _posts() -> dict[str, str]:
    text = X_POSTS.read_text(encoding="utf-8")
    posts = re.findall(r"^## (Post \d+)[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    assert posts, "x-post-draft.md has no '## Post N' sections"
    return {name: body.strip() for name, body in posts}


def _track_ids_in_sentence(text: str, lead: str) -> set[str]:
    """Backticked track IDs in the sentence that starts with `lead`."""
    assert lead in text, f"no sentence starting with {lead!r}"
    start = text.index(lead)
    return set(re.findall(r"`([a-z]+)`", text[start : text.index(".", start)]))


def test_description_fits_the_form_under_every_count():
    body = _description()
    counts = {
        "LF characters": len(body),
        "UTF-16 units": _utf16_length(body),
        "CRLF characters": _utf16_length(body.replace("\n", "\r\n")),
    }
    for name, count in counts.items():
        assert IDEA_MIN <= count <= IDEA_MAX, f"description is {count} {name}"


def test_tracks_are_organizer_ids_and_every_document_names_the_same_set():
    submission = SUBMISSION.read_text(encoding="utf-8")
    tracks_line = next(
        line for line in submission.splitlines() if line.startswith("**Tracks:**")
    )
    submitted = re.findall(r"`([a-z]+)`", tracks_line)
    assert submitted, "no track IDs on the **Tracks:** line"
    assert len(submitted) == len(set(submitted)), f"duplicate track IDs: {submitted}"
    assert set(submitted) <= ORGANIZER_TRACKS, set(submitted) - ORGANIZER_TRACKS
    assert set(submitted) == {"security", "agents", "defi"}, "owner confirmed all three tracks"

    readme = _track_ids_in_sentence(README.read_text(encoding="utf-8"), "Submitted tracks:")
    checklist = _track_ids_in_sentence(
        CHECKLIST.read_text(encoding="utf-8"), "Owner-selected tracks:"
    )
    assert readme == set(submitted), f"README {sorted(readme)} vs {sorted(submitted)}"
    assert checklist == set(submitted), f"checklist {sorted(checklist)} vs {sorted(submitted)}"


def test_every_x_post_fits_as_x_counts_it():
    for name, body in _posts().items():
        assert _x_length(body) <= X_LIMIT, f"{name} is {_x_length(body)} characters"


def test_first_post_says_built_at_near_hackathon_and_tags_both():
    first = _posts()["Post 1"]
    assert "NEAR HACKATHON" in first.upper(), "post 1 must say it was built at NEAR HACKATHON"
    assert "@NEARProtocol" in first
    assert "#NEARHackathon" in first


def test_posts_carry_the_public_repository_link_not_a_placeholder():
    posts = _posts()
    assert not [name for name, body in posts.items() if "REPOSITORY_URL" in body]
    assert any(REPOSITORY_URL in body for body in posts.values())


def test_x_length_counts_links_as_23_characters():
    assert _x_length(f"see {REPOSITORY_URL}") == len("see ") + X_URL_WEIGHT
    assert _x_length("pay x3lang.near") == len("pay x3lang.near")
    assert _x_length("open dotx3.net") == len("open ") + X_URL_WEIGHT


def test_judge_track_surfaces_name_all_owner_selected_tracks():
    surfaces = [
        ("JUDGES.md", "Owner-confirmed selection:"),
        ("docs/hackathon/JUDGE_PACKAGE.md", "**Owner-selected tracks:**"),
        ("docs/hackathon/X3LANG_THREE_TRACK_CONTROL.md", "**Scope:**"),
    ]
    for path, lead in surfaces:
        assert _track_ids_in_sentence((ROOT / path).read_text(), lead) == {"security", "agents", "defi"}, path
