#!/bin/sh
set -e
RESOURCES="$1"
OUT="$2"
BUILDER="$3"
mkdir -p "$(dirname "$OUT")"
if [ -f "$OUT" ] && [ -f "$RESOURCES/references.json.gz" ]; then
  if [ "$OUT" -nt "$RESOURCES/references.json.gz" ]; then
    echo "index cache hit: $OUT"
    exit 0
  fi
fi
echo "building index..."
"$BUILDER" "$RESOURCES" "$OUT" 128
