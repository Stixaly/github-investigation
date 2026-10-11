#!/bin/sh
# Create one GitHub issue per REM-* row of the roadmap backlog (§15 of
# docs/FEUILLE_DE_ROUTE_INVESTIGATION.md), with priority and lot labels.
# An issue whose title already starts with the same REM-xx is skipped, so the
# script can be re-run after the table grows.
#
# Usage: scripts/create-backlog-issues.sh [--dry-run]
# Needs `gh auth login` and a repository (current remote, or GH_REPO=owner/name).
set -eu
cd "$(dirname "$0")/.."
roadmap=docs/FEUILLE_DE_ROUTE_INVESTIGATION.md
dry_run=false
[ "${1:-}" = "--dry-run" ] && dry_run=true

rows=$(grep -E '^\| REM-[0-9]+ \|' "$roadmap")
[ -n "$rows" ] || { echo "No REM-* row found in $roadmap" >&2; exit 1; }

if ! $dry_run; then
    gh auth status >/dev/null
    existing=$(gh issue list --state all --limit 500 --search "REM- in:title" --json title --jq '.[].title')
    for label in P0 P1 P2 P3; do
        gh label create "$label" --color B60205 --description "Priorité $label de la feuille de route" --force >/dev/null
    done
fi

echo "$rows" | while IFS='|' read -r _ id priority work criterion _; do
    id=$(echo "$id" | xargs)
    priority=$(echo "$priority" | sed 's/^ *//; s/ *$//')
    work=$(echo "$work" | sed 's/^ *//; s/ *$//')
    criterion=$(echo "$criterion" | sed 's/^ *//; s/ *$//')
    # "P0 avant équipe / R8" -> labels P0 and R8; the delivered part "(0.2 : …)" stays in the body only.
    level=$(echo "$priority" | grep -oE '^P[0-3]' || true)
    lot=$(echo "$priority" | sed -n 's#.*/ *\([^ ]*\) *$#\1#p')
    title="$id — $(echo "$work" | sed 's/ *(0\.2 :.*)$//')"
    body=$(printf '%s\n\n%s\n\n%s\n\n%s\n' \
        "**Priorité / lot :** $priority" \
        "**Travail :** $work" \
        "**Critère de clôture :** $criterion" \
        "Détail et critères d'acceptation : \`$roadmap\` (sections 4 à 16, backlog §15).")
    if $dry_run; then
        printf '%-8s [%s, %s] %s\n' "$id" "${level:-?}" "${lot:-?}" "$title"
        continue
    fi
    if echo "$existing" | grep -q "^$id —"; then
        echo "skip $id (already exists)"
        continue
    fi
    labels=${level:-}
    if [ -n "$lot" ]; then
        gh label create "$lot" --color 0E8A16 --description "Lot $lot de la feuille de route" --force </dev/null >/dev/null
        labels="${labels:+$labels,}$lot"
    fi
    gh issue create --title "$title" --body "$body" ${labels:+--label "$labels"} </dev/null >/dev/null
    echo "created $id"
done
