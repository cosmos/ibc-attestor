.PHONY: build-attestor-image lint lint-fix fmt fmt-check test

build-attestor-image:
	docker build -t attestor-local -f apps/ibc-attestor/Dockerfile .

lint:
	cargo clippy --all-targets --all-features -- -D warnings

fmt-check:
	cargo fmt --all -- --check

test:
	cargo test --all-features
