#!/usr/bin/env bash

set -e

echo "🚀 Building and installing mev globally..."

# This will compile the release binary and install it to ~/.cargo/bin
# --force ensures it overwrites any existing older version.
cargo install --path . --force

echo "✅ mev has been successfully updated!"
mev --version || echo "(Make sure ~/.cargo/bin is in your PATH)"
