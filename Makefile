.PHONY: build-attestor-image lint lint-fix fmt fmt-check test

build-attestor-image:
	docker build -t attestor-local -f apps/ibc-attestor/Dockerfile .

# Run clippy with strict lints
lint:
	cargo clippy --all-targets --all-features -- -D warnings

# Run clippy with automatic fixes (requires cargo-fix)
lint-fix:
	cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features

# Format code
fmt:
	cargo fmt --all

# Check if code is formatted
fmt-check:
	cargo fmt --all -- --check

# Run tests
test:
	cargo test --all-features
