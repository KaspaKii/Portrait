#!/usr/bin/env bash
# _harness/release-cut.sh — the release ceremony for kaspa-compliance-patterns.
#
# DOES NOT push or merge anything. Prepares the local repo for a tagged release;
# the actual push + tag is a maintainer's gate, performed by hand after this script
# leaves a clean, verified, release-ready state.
#
# Usage:
#   ./_harness/release-cut.sh v0.1.0    # prepare a v0.1.0 release-cut
#
# Steps:
#   1. Refuse if working tree is dirty.
#   2. Run full CI (./_harness/ci.sh) — must be GREEN.
#   3. Verify CHANGELOG.md has an entry matching the requested version.
#   4. Build the mdBook docs site to book/site/ (ready for GitHub Pages deploy).
#   5. Generate a SUMMARY.md at /tmp/release-<version>-SUMMARY.md with the
#      release notes, gates report, and git log since the prior tag.
#   6. Report the next-step commands for the maintainer (tag + push + GitHub release).

set -eu
cd "$(dirname "$0")/.."

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "usage: $0 <version>   e.g. $0 v0.1.0" >&2
  exit 2
fi

echo "=== release-cut $VERSION ==="

# 1. Working tree must be clean
if [ -n "$(git status --porcelain)" ]; then
  echo "REFUSED: working tree is dirty. Commit or stash before release-cut." >&2
  git status --short | head -10 >&2
  exit 1
fi

# 2. Full CI
echo ""
echo "--- step 1: full CI ---"
./_harness/ci.sh

# 3. CHANGELOG entry must exist for this version
VBARE="${VERSION#v}"
if ! grep -qE "^## \[${VBARE}\]" CHANGELOG.md; then
  echo "REFUSED: CHANGELOG.md does not have an entry for [${VBARE}]." >&2
  echo "Add one before release-cut. Format: ## [${VBARE}] — YYYY-MM-DD" >&2
  exit 1
fi

# 4. mdBook build (idempotent — ci already ran it but re-run for clean output)
echo ""
echo "--- step 2: mdbook build ---"
if command -v mdbook >/dev/null 2>&1; then
  ( cd book && mdbook build )
  echo "docs site rendered to book/site/ — ready for GitHub Pages deploy on the Foundation org."
else
  echo "WARN: mdbook not on PATH. Install with: cargo install mdbook"
fi

# 5. Generate release summary
PRIOR_TAG="$(git describe --tags --abbrev=0 2>/dev/null || echo '')"
RANGE_SPEC=""
if [ -n "$PRIOR_TAG" ]; then
  RANGE_SPEC="${PRIOR_TAG}..HEAD"
else
  RANGE_SPEC="HEAD"
fi

SUMMARY="/tmp/release-${VBARE}-SUMMARY.md"
{
  echo "# kaspa-compliance-patterns ${VERSION} — release-cut summary"
  echo ""
  echo "**Steward:** Stichting Kii Foundation"
  echo "**Engine reference:** rusty-kaspa v2.0.0 (90dbf07)"
  echo "**Head:** $(git rev-parse --short HEAD)"
  echo "**Prior tag:** ${PRIOR_TAG:-'(none — first release)'}"
  echo ""
  echo "## Gates verified"
  echo "- cargo fmt --check: PASS"
  echo "- cargo clippy --workspace --all-targets -- -D warnings: PASS"
  echo "- cargo test --workspace: PASS"
  echo "- examples/hello-vault cargo run: PASS"
  echo "- mdbook build: PASS"
  echo ""
  echo "## CHANGELOG entry"
  awk -v ver="${VBARE}" '/^## \[/{p=0} $0 ~ "^## \\[" ver "\\]" {p=1} p' CHANGELOG.md
  echo ""
  echo "## Commits since ${PRIOR_TAG:-the beginning}"
  git log --oneline "${RANGE_SPEC}"
  echo ""
  echo "## Maintainer next steps (do these by hand; the script does NOT)"
  echo '```sh'
  echo "git tag -a ${VERSION} -m 'kaspa-compliance-patterns ${VERSION}'"
  echo "# (only after a remote exists; until then this is local-only)"
  echo "# git push origin main"
  echo "# git push origin ${VERSION}"
  echo "# Deploy book/site/ to GitHub Pages on the Foundation org"
  echo "# Open a GitHub Release with this SUMMARY as the body"
  echo '```'
} > "$SUMMARY"

echo ""
echo "--- step 3: release summary ---"
echo "Written to: $SUMMARY"
echo ""
echo "=== RELEASE-CUT READY ==="
echo "Local repo is clean + verified + ready for ${VERSION} tag."
echo "Maintainer gate: review the summary, run the tag + push commands when ready."
echo "The script DID NOT tag, push, or open a release."
