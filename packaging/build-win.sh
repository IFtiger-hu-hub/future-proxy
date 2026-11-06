#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -n1 | sed -E 's/.*"([^"]+)".*/\1/')"
MINGW_CACHE="/tmp/mingw-root"

echo "==> [1/4] Preparing MinGW-w64 sysroot, windres & LLVM linker..."
RUST_BIN_DIR="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin"
ln -sf "$RUST_BIN_DIR/llvm-ar" /tmp/llvm-dlltool
ln -sf "$RUST_BIN_DIR/llvm-ar" /tmp/x86_64-w64-mingw32-dlltool

if [ ! -f "$MINGW_CACHE/usr/x86_64-w64-mingw32/sys-root/mingw/lib/libmingwex.a" ] || [ ! -f "$MINGW_CACHE/usr/bin/x86_64-w64-mingw32-windres" ]; then
  mkdir -p "$MINGW_CACHE"
  pushd "$MINGW_CACHE" >/dev/null
  curl -sL -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/noarch/mingw64-runtime-14.0.0-1.1.noarch.rpm \
            -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/noarch/mingw64-winpthreads-devel-14.0.0-1.1.noarch.rpm \
            -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/x86_64/mingw64-cross-gcc-16.2.0-1.1.x86_64.rpm \
            -O https://mirrors.ustc.edu.cn/opensuse/tumbleweed/repo/oss/x86_64/mingw64-cross-binutils-2.45.1-3.4.x86_64.rpm
  for f in *.rpm; do rpm2cpio "$f" | cpio -idmu 2>/dev/null; done
  popd >/dev/null
fi

echo "==> Generating multi-size Windows .ico & compiling PE .rsrc icon resource..."
python3 "$ROOT_DIR/packaging/gen_win_icon.py"
"$MINGW_CACHE/usr/bin/x86_64-w64-mingw32-windres" \
  --preprocessor=/usr/bin/cpp \
  -I "$ROOT_DIR/packaging" \
  "$ROOT_DIR/packaging/future-proxy.rc" \
  -O coff -o /tmp/future-proxy-res.o

RUST_WIN_LIB="$(rustc --print sysroot)/lib/rustlib/x86_64-pc-windows-gnu/lib"
cat << EOF > /tmp/x86_64-w64-mingw32-gcc
#!/usr/bin/env bash
exec /usr/bin/clang --target=x86_64-pc-windows-gnu \\
  -fuse-ld="$RUST_BIN_DIR/gcc-ld/ld.lld" \\
  -nostdlib \\
  "$MINGW_CACHE/usr/x86_64-w64-mingw32/sys-root/mingw/lib/crt2.o" \\
  "$RUST_WIN_LIB/rsbegin.o" \\
  -L"$MINGW_CACHE/usr/x86_64-w64-mingw32/sys-root/mingw/lib" \\
  -L"$MINGW_CACHE/usr/lib64/gcc/x86_64-w64-mingw32/16.2.0" \\
  "\$@" \\
  /tmp/future-proxy-res.o \\
  "$RUST_WIN_LIB/rsend.o"
EOF
chmod +x /tmp/x86_64-w64-mingw32-gcc

echo "==> [2/4] Compiling future-proxy.exe (x86_64-pc-windows-gnu)..."
cd "$ROOT_DIR"
touch "$ROOT_DIR/src/main.rs"
PATH="/tmp:$PATH" RUSTFLAGS="-C link-self-contained=no" cargo build --release --bin future-proxy --target x86_64-pc-windows-gnu
"$RUST_BIN_DIR/llvm-strip" --strip-debug "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy.exe"

echo "==> [3/4] Staging Windows runtime payload..."
mkdir -p "$ROOT_DIR/Future-Proxy"
cp -f "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy.exe" "$ROOT_DIR/Future-Proxy/future-proxy.exe"
cp -f "$ROOT_DIR/packaging/config.yaml" "$ROOT_DIR/Future-Proxy/config.yaml"
cp -f "$ROOT_DIR/packaging/country.mmdb" "$ROOT_DIR/Future-Proxy/country.mmdb"
cp -f "$ROOT_DIR/packaging/geosite.dat" "$ROOT_DIR/Future-Proxy/geosite.dat"
cp -f "$ROOT_DIR/packaging/future-proxy.ico" "$ROOT_DIR/Future-Proxy/future-proxy.ico"

echo "==> [4/4] Compiling standalone GUI installer Future-Proxy-${VERSION}-Windows-x86_64-Setup.exe..."
rm -f "$ROOT_DIR"/Future-Proxy-*-windows-x86_64.zip "$ROOT_DIR"/Future-Proxy-*-Windows-x86_64-Setup.exe
touch "$ROOT_DIR/src/bin/setup.rs"
PATH="/tmp:$PATH" RUSTFLAGS="-C link-self-contained=no" cargo build --release --bin future-proxy-setup --target x86_64-pc-windows-gnu
"$RUST_BIN_DIR/llvm-strip" --strip-debug "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy-setup.exe"
cp -f "$ROOT_DIR/target/x86_64-pc-windows-gnu/release/future-proxy-setup.exe" "$ROOT_DIR/Future-Proxy-${VERSION}-Windows-x86_64-Setup.exe"

echo "Done!"
ls -lh "$ROOT_DIR/Future-Proxy-${VERSION}-Windows-x86_64-Setup.exe"
