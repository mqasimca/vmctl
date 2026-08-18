CARGO ?= cargo

.PHONY: all fmt check test lint doc build verify clean

all: verify

fmt:
	$(CARGO) fmt --all -- --check

check:
	$(CARGO) check --all-targets --all-features

test:
	$(CARGO) test --all-targets --all-features
	$(CARGO) test --doc --all-features

lint:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

doc:
	$(CARGO) doc --no-deps --all-features

build:
	$(CARGO) build --release

verify: fmt check test lint doc build

clean:
	$(CARGO) clean
