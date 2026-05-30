#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <tag> [--build-only] [--skip-build] [--registry REPO]"
  echo "  e.g. $0 oraculo"
  echo "       $0 oraculo-c --registry ghcr.io/sl4ureano/rinha2026"
  exit 1
}

TAG="${1:-}"
[[ -n "$TAG" ]] || usage
shift

REGISTRY="ghcr.io/sl4ureano/rinha2026"
LOCAL_NAME="rinha2026"
BUILD_ONLY=0
SKIP_BUILD=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-only) BUILD_ONLY=1; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --registry)
      REGISTRY="${2:?}"
      shift 2
      ;;
    *) usage ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REMOTE="${REGISTRY}:${TAG}"
LOCAL="${LOCAL_NAME}:${TAG}"

cd "$ROOT"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "==> docker build -t $LOCAL ."
  docker build -t "$LOCAL" .
fi

echo "==> docker tag $LOCAL $REMOTE"
docker tag "$LOCAL" "$REMOTE"

if [[ "$BUILD_ONLY" -eq 1 ]]; then
  echo "BuildOnly: skipped push. Local=$LOCAL Remote=$REMOTE"
  exit 0
fi

echo "==> docker push $REMOTE"
docker push "$REMOTE"

echo ""
echo "OK  $REMOTE"
echo "    local: $LOCAL"
