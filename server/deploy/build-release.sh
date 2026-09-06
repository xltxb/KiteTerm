#!/usr/bin/env bash
# 在 macOS/Linux 上交叉编译出 Linux 静态二进制并打包。
# 依赖:zig(brew install zig)、cargo-zigbuild(cargo install cargo-zigbuild)、
#       rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
# 用法:server/deploy/build-release.sh [x86_64|aarch64|all]   (默认 all)
set -euo pipefail
cd "$(dirname "$0")/.."
ARCHES=${1:-all}
VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')
OUT=dist; mkdir -p "$OUT"
build() {
  local arch=$1 target="$1-unknown-linux-musl"
  echo "==> $target"
  cargo zigbuild --release --locked --target "$target"
  local pkg="kite-server-${VERSION}-linux-${arch}"
  rm -rf "$OUT/$pkg"; mkdir -p "$OUT/$pkg"
  cp "target/$target/release/kite-server" "$OUT/$pkg/"
  cp deploy/kite-server.service deploy/kite-server.env.example deploy/README.md "$OUT/$pkg/"
  tar -C "$OUT" -czf "$OUT/$pkg.tar.gz" "$pkg"
  (cd "$OUT" && shasum -a 256 "$pkg.tar.gz" > "$pkg.tar.gz.sha256")
  rm -rf "$OUT/$pkg"
  echo "    -> $OUT/$pkg.tar.gz"
}
case "$ARCHES" in
  x86_64|aarch64) build "$ARCHES" ;;
  all) build x86_64; build aarch64 ;;
  *) echo "未知架构 $ARCHES" >&2; exit 2 ;;
esac
