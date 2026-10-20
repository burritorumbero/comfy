#!/bin/sh
# Builds the server when needed, then starts it and publishes it to your tailnet with
# "tailscale serve". Extra arguments are passed on, e.g.  ./start.sh --tailscale 8443
set -e
cd "$(dirname "$0")"
if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Install it from https://rustup.rs (or: brew install rust), then run this again." >&2
  exit 1
fi
cargo build --release --quiet
exec ./target/release/comfyui-prompt-server --tailscale "$@"
