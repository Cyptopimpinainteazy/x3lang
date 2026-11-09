#!/usr/bin/env bash
set -euo pipefail
base="https://raw.githubusercontent.com/Cyptopimpinainteazy/x3lang/assets/judge-video-diagrams-20261106/docs/hackathon/video-assets/batch2"
dest="$HOME/Downloads/X3Lang-Video/batch2"
mkdir -p "$dest/svg" "$dest/png" "$dest/video"
cd "$dest"
for n in \
  10_x3_policy_blueprint 11_parser_ast 12_semantic_gates 13_x3bc_bytecode \
  14_x3vm_runtime 15_two_distinct_paths 16_agent_request_flow 17_signed_1click_quote \
  18_near_guard_decision_fork 19_1click_deposit_boundary 20_tidepool_route \
  21_historical_guard_authorization 22_source_base_receipt 23_destination_near_receipt \
  24_trust_boundary 25_end_to_end_judge_map
do
  curl --fail --location --show-error --silent --retry 3 "$base/$n.svg" -o "svg/$n.svg"
  test -s "svg/$n.svg"
done
curl --fail --location --show-error --silent "$base/render_batch2.py" -o render_batch2.py
curl --fail --location --show-error --silent "$base/render_motion.py" -o render_motion.py
curl --fail --location --show-error --silent "$base/README.md" -o README.md
if test -x "$HOME/Downloads/X3Lang-Video/.assets-venv/bin/python"; then
  py="$HOME/Downloads/X3Lang-Video/.assets-venv/bin/python"
else
  python3 -m venv "$dest/.venv"
  py="$dest/.venv/bin/python"
  "$py" -m pip install --quiet cairosvg pillow
fi
command -v ffmpeg >/dev/null || { echo "ffmpeg required" >&2; exit 1; }
command -v ffprobe >/dev/null || { echo "ffprobe required" >&2; exit 1; }
"$py" render_batch2.py
echo "GALLERY: $dest/index.html"
echo "ZIP: $HOME/Downloads/X3Lang-Video/X3Lang_Batch2_Technical_Assets.zip"
