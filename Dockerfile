# syntax=docker/dockerfile:1
# Every stage is Alpine, pinned by digest (docs/adr/0001, 0002). Check a new
# digest with: docker run --rm <image> cat /etc/alpine-release

FROM node:24-alpine@sha256:44fce696f38ba8c987c4db89954051000bbd05ffd8d8659ba15ed4a8602e937f AS ui-deps
WORKDIR /ui
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./

# Lint, type check and unit tests; `make test` builds this stage.
FROM ui-deps AS ui-check
RUN npm run check

FROM ui-deps AS ui
RUN npm run build

FROM rust:1.99.0-alpine3.24@sha256:0cce0a5e0e8ba67b455257a3a02a1d99005f382748789d6464460028810f1627 AS rust-base
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY migrations/ migrations/

FROM rust-base AS build
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin observatory \
    && cp target/release/observatory /observatory

# Integration tests run against the test-postgres service (compose profile `test`).
FROM rust-base AS test
# Keyword-list tests read the roadmap example list.
COPY docs/examples/ docs/examples/
RUN cargo test --locked --no-run
CMD ["cargo", "test", "--locked", "--offline"]

FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime
RUN addgroup -g 10001 app && adduser -D -H -u 10001 -G app app
COPY --from=build /observatory /usr/local/bin/observatory
COPY --from=ui /ui/dist /app/static
ENV STATIC_DIR=/app/static LOG_FORMAT=json
USER 10001:10001
EXPOSE 8000
ENTRYPOINT ["observatory"]
CMD ["serve"]
