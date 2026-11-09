#!/usr/bin/env bash
set -euo pipefail
# Authenticated read-only distribution for a PRIVATE GitHub repository.
# Downloads only batch2 assets, without cloning the full X3Lang repository.
repo="Cyptopimpinainteazy/x3lang"
ref="assets%2Fjudge-video-diagrams-20261106"
prefix="docs/hackathon/video-assets/batch2"
dest="$HOME/Downloads/X3Lang-Video/batch2"
if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI (gh) is required: install gh, then run 'gh auth login'." >&2
  exit 1
fi
if ! gh auth status -h github.com >/dev/null 2>&1; then
  echo "No valid GitHub CLI login. Run: gh auth login" >&2
  exit 1
fi
# If the account lacks X3Lang repo access, fail immediately.
if ! gh api -H "Accept: application/vnd.github+json" "repos/$repo" --jq '.full_name' >/dev/null; then
  echo "Your GitHub CLI account cannot read the private repository $repo." >&2
  exit 1
fi
mkdir -p "$dest/svg" "$dest/png" "$dest/video"
cd "$dest"
fetch_private() {
  local remote="$1" local_path="$2"
  # REST contents endpoint returns raw bytes with the official media type.
  gh api -X GET -H "Accept: application/vnd.github.raw+json" \
    "repos/$repo/contents/$prefix/$remote?ref=$ref" > "$local_path"
  test -s "$local_path" || { echo "Empty download: $remote" >&2; exit 1; }
}
for n in \
  10_x3_policy_blueprint 11_parser_ast 12_semantic_gates 13_x3bc_bytecode \
  14_x3vm_runtime 15_two_distinct_paths 16_agent_request_flow 17_signed_1click_quote \
  18_near_guard_decision_fork 19_1click_deposit_boundary 20_tidepool_route \
  21_historical_guard_authorization 22_source_base_receipt 23_destination_near_receipt \
  24_trust_boundary 25_end_to_end_judge_map
do
  fetch_private "$n.svg" "svg/$n.svg"
  if ! head -c 5 "svg/$n.svg" | grep -q "<svg"; then
    echo "Unexpected content in svg/$n.svg (expected raw SVG)" >&2
    exit 1
  fi
  echo "DOWNLOADED $n.svg"
done
fetch_private "render_batch2.py" "render_batch2.py"
fetch_private "render_motion.py" "render_motion.py"
fetch_private "README.md" "README.md"
if test -x "$HOME/Downloads/X3Lang-Video/.assets-venv/bin/python"; then
  py="$HOME/Downloads/X3Lang-Video/.assets-venv/bin/python"
else
  python3 -m venv "$dest/.venv"
  py="$dest/.venv/bin/python"
  "$py" -m pip install --quiet cairosvg pillow
fi
command -v ffmpeg >/dev/null || { echo "ffmpeg required for video export" >&2; exit 1; }
command -v ffprobe >/dev/null || { echo "ffprobe required for video verification" >&2; exit 1; }
"$py" render_batch2.py
echo "GALLERY: $dest/index.html"
echo "ZIP: $HOME/Downloads/X3Lang-Video/X3Lang_Batch2_Technical_Assets.zip"
