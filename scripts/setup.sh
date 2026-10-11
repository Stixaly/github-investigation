#!/bin/sh
# Run from a Linux host with Docker + Compose. Existing secrets are never replaced.
set -eu
cd "$(dirname "$0")/.."
command -v docker >/dev/null
docker compose version >/dev/null
umask 077
mkdir -p .secrets
chmod 700 .secrets
for name in db_admin db_app; do
    if [ ! -s ".secrets/$name" ]; then
        # 32 random bytes, hex encoded, from the host kernel.
        od -An -vtx1 -N32 /dev/urandom | tr -d ' \n' > ".secrets/$name"
    fi
    # Compose binds individual files: readable in the container; parent is private on host.
    chmod 444 ".secrets/$name"
done
if [ ! -f .secrets/github_token ]; then
    : > .secrets/github_token
fi
chmod 444 .secrets/github_token
if [ ! -f .env ]; then
    cp .env.example .env
fi
printf '%s\n' 'Configuration prête. Prochaine étape : docker compose up --build -d --wait'
