APP := learn
IMAGE ?= proxyxai/learn
VERSION ?= rust

.PHONY: check test integration-test fmt fmt-check clippy js-check compose-check release-check run docker-build

check:
	cargo check --locked --all-targets

test:
	cargo test --locked --all-targets

# Needs LEARN_TEST_DATABASE_URL: a disposable database whose name starts with learn_test.
integration-test:
	cargo test --locked postgres_ -- --ignored

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --locked --all-targets -- -D warnings

js-check:
	@for file in pages/assets/*.js; do node --check "$$file" || exit; done

compose-check:
	AI_API_KEY=x TMPFILE_API_KEY=x docker compose config >/dev/null

release-check: fmt-check test clippy js-check compose-check
	@echo "OK: $(APP) release checks passed."

run:
	cargo run

docker-build:
	docker build -t $(IMAGE):$(VERSION) .
