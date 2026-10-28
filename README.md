# Future Proxy

Ultra-lightweight Standalone Native GUI Proxy Client written in Rust with eframe/egui.

## 核心特性
- **轻量独立**：纯 Rust 原生渲染界面，内存占用极低（~50MB）。
- **零外部复杂依赖**：内置核心引擎与配置，支持订阅管理与节点一键切换。
- **开箱即用**：自动适配桌面环境与快捷方式。

## 开发与运行

```bash
# 启动本地开发界面
cargo run

# 编译发布版本
cargo build --release
```

## 打包 RPM 安装包

```bash
# 运行打包脚本（会自动编译 release 并生成 RPM 包至根目录）
./packaging/build-rpm.sh
```
