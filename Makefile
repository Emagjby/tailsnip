.PHONY: test run build install

test:
	cargo test

run:
	cargo run

build:
	cargo build

install:
	cargo build --release && mkdir -p ~/.local/bin && install -m 0755 target/release/tailsnip ~/.local/bin/tailsnip
