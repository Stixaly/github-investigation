#!/bin/sh
# Upgrade check run by CI on every pull request: a database written by the
# base commit (what every deployment runs until the merge) migrates to this
# commit's schema with its rows intact, and this binary accepts it.
#
# Usage: scripts/check-migration-from-base.sh <base-sha>
# Needs a disposable PostgreSQL (DB_HOST, DB_PASSWORD for the postgres user) and psql.
set -eu
base="$1"
cd "$(dirname "$0")/.."
workdir="${RUNNER_TEMP:-/tmp}/observatory-base"
rm -rf "$workdir"
git worktree add --detach "$workdir" "$base" >/dev/null
trap 'git worktree remove --force "$workdir"' EXIT

if [ ! -f "$workdir/Cargo.toml" ]; then
    echo "The base commit predates the Rust backend; the Python upgrade path is covered by crates/observatory/tests/migrations.rs."
    exit 0
fi

export DB_HOST="${DB_HOST:-127.0.0.1}" DB_NAME="${DB_NAME:-investigation}" DB_USER=postgres
export APP_PASSWORD=ci-application-role-only PGPASSWORD="$DB_PASSWORD"
counts() {
    psql -h "$DB_HOST" -U postgres -d "$DB_NAME" -tAc \
        "SELECT concat_ws(',', (SELECT count(*) FROM users), (SELECT count(*) FROM scans),
                (SELECT count(*) FROM entities), (SELECT count(*) FROM relations),
                (SELECT count(*) FROM scan_entities), (SELECT count(*) FROM scan_relations),
                (SELECT count(*) FROM evidence))"
}

cargo build --locked --quiet --bin observatory --manifest-path "$workdir/Cargo.toml" --target-dir "$workdir/target"
cargo build --locked --quiet --bin observatory
base_bin="$workdir/target/debug/observatory"
head_bin=target/debug/observatory

"$base_bin" migrate
echo 'base-commit-only-password' | "$base_bin" create-user analyste --password-stdin
"$base_bin" demo --login analyste >/dev/null
before=$(counts)

"$head_bin" migrate
"$head_bin" migrate --check
after=$(counts)

echo "rows before: $before"
echo "rows after:  $after"
[ "$before" = "$after" ] || { echo "The migration changed the stored observations." >&2; exit 1; }
