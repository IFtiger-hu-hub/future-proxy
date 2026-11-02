# Future Proxy

Ultra-lightweight Standalone Native GUI Proxy Client written in Rust (`eframe` / `egui`) with a 100% custom Apple Cupertino Dark Mode design system and built-in Mihomo kernel management.

## 核心特性

- **纯正 Apple Cupertino 界面**：全矢量自绘的 Cupertino 控件体系（Segmented Control、iOS/macOS Switch、Pop-up Button、高密度节点卡片），零外部前端框架依赖。
- **极致轻量与低内存占用**：
  - 内核后台守护常驻仅约 **45 ~ 60 MB**。
  - 关闭主窗口回收到系统托盘后，GUI 自动释放显存与工作集（`malloc_trim` / `K32EmptyWorkingSet`），后台驻留仅 **~6 ~ 8 MB**。
- **内核级区域锁定与自动容灾（Auto-Failover）**：
  - 自动在订阅中注入 `⚡ 🌍 非港台-自动优选`（自动过滤香港/澳门/台湾/中国大陆节点，保障 AI 与流媒体服务可用）及各主流地区最优组（新加坡 / 日本 / 美国 / 香港）。
  - 即使完全关闭前端界面，后台内核依然每 180 秒自动探活并在节点超时时毫秒级无感切换到可用节点。
- **跨平台原生支持（Linux & Windows 10/11）**：
  - **Linux**：支持 `systemd --user` 后台服务守护、KDE/桌面系统代理自动配置、StatusNotifierItem (`ksni`) 系统托盘与 RPM 一键打包。
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
