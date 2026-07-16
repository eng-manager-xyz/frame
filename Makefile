.PHONY: check test web media-probe media-smoke

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo check -p frame-control-plane --target wasm32-unknown-unknown

test:
	cargo test --workspace

web:
	cargo run -p frame-web

media-probe:
	cargo run -p frame-media-worker -- probe

media-smoke:
	cargo run -p frame-media-worker -- smoke target/frame-smoke.webm
