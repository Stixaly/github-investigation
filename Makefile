.PHONY: setup up down logs test test-rust test-ui check fmt lint e2e audit
COMPOSE_TEST = docker compose --profile test

setup:
	sh scripts/setup.sh

up:
	docker compose up --build -d --wait

down:
	docker compose down

logs:
	docker compose logs --tail=100 api worker

# Everything CI blocks on, inside Alpine containers: Rust integration tests
# against a throwaway PostgreSQL, then the frontend checks.
test: test-rust test-ui

test-rust:
	$(COMPOSE_TEST) build test
	$(COMPOSE_TEST) run --rm test; status=$$?; $(COMPOSE_TEST) rm -fsv test-postgres >/dev/null; exit $$status

test-ui:
	docker build --target ui-check -t github-investigation:ui-check .

# Same gates with local toolchains (rustup, Node 24), for a faster loop.
check: fmt lint
	cargo test --locked
	cd frontend && npm run check

fmt:
	cargo fmt --all --check

lint:
	cargo clippy --all-targets --locked -- -D warnings

audit:
	cargo deny check
	cd frontend && npm audit --omit=dev --audit-level=high

e2e:
	docker build -f frontend/Dockerfile.browser -t github-investigation:browser .
	mkdir -p .local/screenshots
	docker run --rm --network host --user "$$(id -u):$$(id -g)" \
	  --cap-drop ALL --security-opt no-new-privileges \
	  -e HOME=/tmp -e LOGIN=analyste \
	  -v "$$PWD/.secrets/analyste_password:/bootstrap_password:ro" \
	  -v "$$PWD/.local/screenshots:/output" \
	  github-investigation:browser
