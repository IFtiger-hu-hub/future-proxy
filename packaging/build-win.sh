#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -n1 | sed -E 's/.*"([^"]+)".*/\1/')"
MINGW_CACHE="/tmp/mingw-root"
BUNDLE_DIR="/tmp/win-bundle/Future-Proxy"

echo "==> [1/4] Preparing MinGW-w64 sysroot & LLVM linker..."
RUST_BIN_DIR="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin"
ln -sf "$RUST_BIN_DIR/llvm-ar" /tmp/llvm-dlltool
ln -sf "$RUST_BIN_DIR/llvm-ar" /tmp/x86_64-w64-mingw32-dlltool

if [ ! -f "$MINGW_CACHE/usr/x86_64-w64-mingw32/sys-root/mingw/lib/libmingwex.a" ]; then
  mkdir -p "$MINGW_CACHE"
  pushd "$MINGW_CACHE" >/dev/null
  curl -sL -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/noarch/mingw64-runtime-14.0.0-1.1.noarch.rpm \
            -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/noarch/mingw64-winpthreads-devel-14.0.0-1.1.noarch.rpm \
            -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/x86_64/mingw64-cross-gcc-16.2.0-1.1.x86_64.rpm
  for f in *.rpm; do rpm2cpio "$f" | cpio -idmu 2>/dev/null; done
  popd >/dev/null
fi

cat << EOF > /tmp/x86_64-w64-mingw32-gcc
#!/usr/bin/env bash
exec /usr/bin/clang --target=x86_64-pc-windows-gnu \\
  -fuse-ld="$RUST_BIN_DIR/gcc-ld/ld.lld" \\
  -nostdlib \\
  -L"$MINGW_CACHE/usr/x86_64-w64-mingw32/sys-root/mingw/lib" \\
  -L"$MINGW_CACHE/usr/lib64/gcc/x86_64-w64-mingw32/16.2.0" \\
  "\$@"
EOF
chmod +x /tmp/x86_64-w64-mingw32-gcc

echo "==> [2/4] Compiling future-proxy.exe (x86_64-pc-windows-gnu)..."
cd "$ROOT_DIR"
PATH="/tmp:$PATH" RUSTFLAGS="-C link-self-contained=yes" cargo build --release --target x86_64-pc-windows-gnu
"$RUST_BIN_DIR/llvm-strip" "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy.exe"
cp -f "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy.exe" "$ROOT_DIR/future-proxy.exe"

echo "==> [3/4] Assembling Windows 10/11 portable bundle..."
mkdir -p "$BUNDLE_DIR"
cp -f "$ROOT_DIR/future-proxy.exe" "$BUNDLE_DIR/future-proxy.exe"
cp -f "$ROOT_DIR/packaging/config.yaml" "$BUNDLE_DIR/config.yaml"
cp -f "$ROOT_DIR/packaging/country.mmdb" "$BUNDLE_DIR/country.mmdb"
cp -f "$ROOT_DIR/packaging/geosite.dat" "$BUNDLE_DIR/geosite.dat"

echo "==> [4/4] Creating Future-Proxy-${VERSION}-windows-x86_64.zip..."
cd /tmp/win-bundle
python3 -m zipfile -c "$ROOT_DIR/Future-Proxy-${VERSION}-windows-x86_64.zip" Future-Proxy/

echo "Done!"
ls -lh "$ROOT_DIR/future-proxy.exe" "$ROOT_DIR/Future-Proxy-${VERSION}-windows-x86_64.zip"

