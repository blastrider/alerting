set dotenv-load := false

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo nextest run --all-features

audit:
	cargo audit

deny:
	cargo deny check

geiger:
	cargo geiger -q

udeps:
	cargo +nightly udeps --all-targets

ci:
	just fmt
	just clippy
	just test
	just deny
	just audit
	just geiger
