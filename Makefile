.PHONY: setup up test check down logs
setup:
	sh scripts/setup.sh
up:
	docker compose up --build -d
test:
	docker compose --profile test build test
	docker compose --profile test run --rm test
check: test
down:
	docker compose down
logs:
	docker compose logs --tail=100 api worker
