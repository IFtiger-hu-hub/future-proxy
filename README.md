# Future Proxy

Ultra-lightweight Standalone Native GUI Proxy Client written in Rust (`eframe` / `egui`) with a custom Bento Grid design system and built-in Mihomo kernel management.

![Future Proxy Dashboard](assets/dashboard.png)

## 核心特性

- **原生矢量 Bento Grid 界面**：
  - 全矢量自绘 Bento 仪表盘（实时上下行速率波形图、一键节点连通性测试、服务状态双路探活、订阅配额正弦波浪进度条、6 列高密度节点网格卡片），支持浅色与深色双主题切换，零 WebView / Electron 依赖。
- **精细化内存监控与一键释放**：
  - 实时区分采集 **GUI 私有堆内存 (`RssAnon`)**、**含共享显存/字体的总驻留集 (`VmRSS`)**、**Mihomo 内核占用** 与 **宿主系统物理内存 (`/proc/meminfo`)**。
  - 仪表盘与工具页均支持一键 **`[ ↻ 释放 ]`** 闲置内存（调用 `malloc_trim(0)` / `K32EmptyWorkingSet` 立即将空闲堆页归还操作系统）。
  - 关闭主窗口回收到系统托盘后，自动销毁渲染上下文并释放显存，后台驻留仅 **~6 ~ 8 MB**。
- **内核级区域锁定与自动容灾（Auto-Failover）**：
  - 自动在订阅中注入 `⚡ 🌍 非港台-自动优选`（自动过滤香港/澳门/台湾/中国大陆及信息提示节点，保障 AI 与流媒体服务可用）及各主流地区最优组（新加坡 / 日本 / 美国 / 香港）。
  - 即使完全关闭前端界面，后台内核依然每 180 秒自动探活并在节点超时时毫秒级无感切换到可用节点。
- **高内聚模块化架构**：
  - 按职责划分为 [`main.rs`](src/main.rs)、[`state.rs`](src/state.rs)、[`core_api.rs`](src/core_api.rs)、[`config_patch.rs`](src/config_patch.rs)、[`platform.rs`](src/platform.rs)、[`theme.rs`](src/theme.rs)、[`ui_dashboard.rs`](src/ui_dashboard.rs)、[`ui_proxies.rs`](src/ui_proxies.rs) 与 [`ui_tabs.rs`](src/ui_tabs.rs) 九大模块。
- **跨平台原生支持（Linux & Windows 10/11）**：
  - **Linux**：支持 `systemd --user` 后台服务守护、KDE 桌面系统代理自动配置、StatusNotifierItem (`ksni`) 系统托盘与 RPM 一键打包。
  - **Windows 10 / 11**：原生 Win32 系统托盘（`Shell_NotifyIconW`）、WinInet 注册表系统代理热切换、计划任务（`schtasks`）/ 注册表 `Run` 开机静默自启（`CREATE_NO_WINDOW` 无黑框）、自动加载系统微软雅黑字体。

## 开发与编译

### Linux
```bash
# 运行单元测试
cargo test

# 启动本地开发界面
cargo run --release

# 一键构建 RPM 安装包（自动输出至项目根目录）
./packaging/build-rpm.sh
```

### Windows 10 / 11
1. 在 Windows 环境下直接编译：
   ```powershell
   cargo build --release
   ```
2. 将编译生成的 `target\release\future-proxy.exe` 与 Windows 版 Mihomo 内核（重命名为 `future-proxy-core.exe`）、`country.mmdb`、`geosite.dat`（如需 TUN 模式则附带 `wintun.dll`）放在同一目录即可直接双击运行。
3. 也可在 Linux 下直接进行跨平台编译检查：
   ```bash
   rustup target add x86_64-pc-windows-gnu
   cargo check --target x86_64-pc-windows-gnu
   ```
4. 仓库已内置 GitHub Actions 工作流（[`.github/workflows/release.yml`](.github/workflows/release.yml)），推送代码后将自动构建 Linux 与 Windows 10/11 便携版压缩包。
