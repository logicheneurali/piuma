#!/bin/sh
# Build a release tarball that carries the license material required for
# binary distribution (LICENSE, THIRD_PARTY_LICENSES.md, LICENSES/).
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
: "${version:?cannot read the version from Cargo.toml}"

cargo build --release

stage=$(mktemp -d "${TMPDIR:-/tmp}/piuma-release.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM

install -m755 target/release/piuma target/release/piuma-setup "$stage/"
install -m644 LICENSE THIRD_PARTY_LICENSES.md README.md "$stage/"
cp -R LICENSES "$stage/"

archive="piuma-$version-linux-x86_64.tar.gz"
tar -czf "$archive" -C "$stage" .
echo "$root/$archive"
