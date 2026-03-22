.PHONY: build test install update

build:
	cargo build

test:
	cargo test

install:
	cargo install --path . --force

update: install
