#!/usr/bin/env bash
# Deploy site/ to the web server — only what is merged.
#
#   scripts/deploy-site.sh              deploy
#   scripts/deploy-site.sh --dry-run    show what would change, change nothing
#
# Where it goes comes from .deploy.env (git-ignored; see .deploy.env.example), so the server's
# address and key stay out of the repo. It deploys main only, clean and the same as GitHub's:
# the deploy copies the working tree, and a branch checked out for review must never go live.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .deploy.env ] || { echo "no .deploy.env — copy .deploy.env.example to .deploy.env and fill it in" >&2; exit 1; }
# shellcheck disable=SC1091
source .deploy.env
: "${DEPLOY_HOST:?DEPLOY_HOST is missing from .deploy.env}"
: "${DEPLOY_KEY:?DEPLOY_KEY is missing from .deploy.env}"
: "${DEPLOY_PATH:?DEPLOY_PATH is missing from .deploy.env}"
KEY="${DEPLOY_KEY/#\~/$HOME}"

branch="$(git branch --show-current)"
if [ "$branch" != main ]; then
  echo "on '$branch' — the site deploys from main only:  git switch main && git pull" >&2
  exit 1
fi
if [ -n "$(git status --porcelain -- site)" ]; then
  echo "site/ has changes that aren't committed — the site deploys what is merged" >&2
  exit 1
fi
git fetch -q origin main
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
  echo "main here isn't the same as GitHub's — git pull (or push) first" >&2
  exit 1
fi

DRY=()
[ "${1:-}" = "--dry-run" ] && DRY=(--dry-run)
echo "→ site/ from main $(git rev-parse --short HEAD) to $DEPLOY_PATH${DRY:+  (dry run)}"
# --delete: the server mirrors site/. README.md stays off it (and --delete-excluded takes away a copy
# an older deploy left): it says how the site is deployed, which the public needn't read.
rsync -avz "${DRY[@]}" --delete --delete-excluded --exclude README.md -e "ssh -i $KEY" site/ "$DEPLOY_HOST:$DEPLOY_PATH/"
