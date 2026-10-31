#!/usr/bin/env bash
# 一键编译 Rust GUI 并构建 Future Proxy RPM 安装包
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$DIR/.." && pwd)"

echo "=== 1. 编译 Rust GUI (Release) ==="
cd "$PROJECT_ROOT"
cargo build --release

echo "=== 2. 准备 RPM 构建环境 ==="
cd "$DIR"
mkdir -p rpmbuild/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

echo "=== 3. 收集打包资源 ==="
cp "$PROJECT_ROOT/target/release/future-proxy" rpmbuild/SOURCES/
cp "$DIR"/config.yaml "$DIR"/country.mmdb "$DIR"/geosite.dat \
   "$DIR"/future-proxy.desktop "$DIR"/future-proxy.svg "$DIR"/mihomo \
   "$DIR"/future-proxy-sysproxy "$DIR"/future-proxy.service \
   rpmbuild/SOURCES/
cp "$DIR"/future-proxy.spec rpmbuild/SPECS/

echo "=== 4. 执行 rpmbuild 构建 ==="
rpmbuild --define "_topdir $DIR/rpmbuild" -ba rpmbuild/SPECS/future-proxy.spec

RPM_PATH=$(find "$DIR/rpmbuild/RPMS" -name "future-proxy-*.rpm" | sort -V | tail -n 1)
cp "$RPM_PATH" "$PROJECT_ROOT/"

echo ""
echo "=========================================================="
echo "  Future Proxy RPM 构建成功！"
echo "  安装包位置: $PROJECT_ROOT/$(basename "$RPM_PATH")"
echo "=========================================================="
