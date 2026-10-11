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
        docker run --rm python:3.13-alpine@sha256:2d9aefe2fef018a7eb2c13064c89c71929800fd2e5dccdbf52ea5da5bb8d929a \
            python -c 'import secrets; print(secrets.token_hex(32))' > ".secrets/$name"
    fi
    # Compose binds individual files: readable in the container; parent is private on host.
    chmod 444 ".secrets/$name"
done
if [ ! -f .secrets/github_token ]; then
    : > .secrets/github_token
fi
chmod 444 .secrets/github_token
if [ ! -f .env ]; then
    printf '%s\n' 'BIND_ADDRESS=127.0.0.1' 'PORT=8088' 'PUBLIC_ORIGIN=http://localhost:8088' > .env
fi
printf '%s\n' 'Configuration prête. Prochaine étape : docker compose up --build -d'
