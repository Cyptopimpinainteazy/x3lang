#!/usr/bin/env bash
set -euo pipefail
gallery="${X3_GALLERY_DIR:-$HOME/Downloads/X3Lang-Video/batch2}"
port="${X3_GALLERY_PORT:-8877}"
if [[ ! -f "$gallery/index.html" ]]; then
  echo "Gallery missing: $gallery/index.html" >&2
  echo "Run the authenticated batch2 installer on THIS machine first." >&2
  exit 2
fi
if ! [[ "$port" =~ ^[0-9]+$ ]] || ((port < 1024 || port > 65535)); then
  echo "Invalid port: $port" >&2; exit 2
fi
cd "$gallery"
echo "Serving X3Lang judge gallery on this host only: http://127.0.0.1:$port/index.html"
echo "For a different computer, forward port $port over SSH and open it there."
exec python3 -m http.server "$port" --bind 127.0.0.1
