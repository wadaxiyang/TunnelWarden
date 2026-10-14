<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="TunnelWarden 图标" />
</p>

<h1 align="center">TunnelWarden</h1>

<p align="center">让 SSH 隧道成为随手可用的桌面工具。</p>

<p align="center">
  保存服务器与转发规则，一键连接本地服务、远程端口和 SOCKS5 代理。<br />
  自动重连、实时流量与托盘控制，让常用隧道安静地运行在后台。
</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="https://github.com/wadaxiyang/TunnelWarden/releases/latest">下载安装</a> ·
  <a href="#功能">功能</a> ·
  <a href="#开发">开发</a> ·
  <a href="https://github.com/wadaxiyang/TunnelWarden/issues">反馈问题</a>
</p>

TunnelWarden 是一款开源的 Windows SSH 隧道管理器，使用 **Tauri 2、Svelte 5 与 Rust** 构建，SSH 连接与端口转发由 **Oryxis** 提供。无论是通过跳板机访问内网数据库、将本地服务转发到远程服务器，还是建立临时 SOCKS5 代理，都可以通过图形界面完成配置和管理。

## 功能

- **三种端口转发**：支持本地转发（`-L`）、远程转发（`-R`）与动态 SOCKS5 转发（`-D`），分别配置监听地址、SSH 主机和目标服务。
- **SSH 主机管理**：保存服务器地址、独立 SSH 端口、用户名与认证方式，支持跳板机和多级跳转，并可在创建隧道前单独测试连接。
- **多种认证方式**：支持密码、私钥、加密私钥与外部 SSH Agent。密码存入 Windows 凭据管理器；私钥口令不落盘，解锁后的密钥仅缓存在当前进程内存中。
- **主机指纹确认**：首次连接或指纹发生变化时提示确认。无人值守连接不会自动信任未知或变化的主机密钥。
- **隧道生命周期管理**：按规则启动、停止、重启，也可批量启停。同一主机的多个转发共享 SSH 连接，停止某条规则不影响其他隧道。
- **自动启动与断线重连**：可为规则启用应用启动时自动连接，支持断线后的退避重试；手动停止会取消当前重试。
- **实时流量与状态**：查看上传、下载速率，当前连接的累计流量与持续时间，以及最近 1 分钟或 5 分钟的流量曲线。
- **活动记录与诊断**：按隧道查看活动历史、筛选错误，并导出 JSON 诊断报告，便于排查连接问题。
- **托盘与后台运行**：关闭窗口后可继续保持隧道；通过托盘启停规则，悬停查看运行中隧道的流量。支持开机登录时在后台启动，后台模式不保留 WebView 窗口。
- **中英文界面**：默认跟随系统语言，中文系统使用中文，其他语言使用英文。可在设置中切换“跟随系统”、简体中文或 English。

## 安装

前往 **[GitHub Releases](https://github.com/wadaxiyang/TunnelWarden/releases/latest)** 下载 Windows x64 安装包：

| 文件 | 安装方式 |
| --- | --- |
| `TunnelWarden_<版本>_x64-setup.exe` | 常规安装，安装到当前用户目录 |
| `TunnelWarden_<版本>_x64_en-US.msi` | Windows Installer 安装包 |

运行安装程序后，打开 TunnelWarden：

1. 在 **SSH Hosts** 中添加服务器，选择认证方式，使用 **Test connection** 检查连接。
2. 在 **Tunnels** 中新建 Local、Remote 或 Dynamic 规则，填写监听地址与目标地址。
3. 启动隧道；首次连接时核对服务器指纹，再确认信任。

目前提供 Windows x64 安装包。安装程序会在缺少 Microsoft WebView2 时下载运行时。

使用时请留意：

- 远程转发的外部可访问性取决于 SSH 服务端的转发策略、`GatewayPorts`、防火墙和网络配置。
- 完全退出应用后，加密私钥需要重新解锁；无人值守启动可配合已解锁的外部 SSH Agent 使用。
- 流量曲线仅在详情页打开时采样，离开后清空；活动历史属于当前进程会话，退出应用后清空。

## 开发

### 准备环境

当前开发与 CI 以 Windows x64 为目标。请先安装：

- **Git**。
- **Node.js 24**，包含 npm。
- **Rust 1.94.1**，使用 MSVC 工具链，并安装 `rustfmt`、`clippy`。
- **Visual Studio 2022 Build Tools**，勾选“使用 C++ 的桌面开发”及 Windows SDK。
- **Microsoft Edge WebView2 Runtime**。
- **Python 3.13**，用于校验 vendored Oryxis 源码与补丁。

### 克隆并运行

```powershell
git clone https://github.com/wadaxiyang/TunnelWarden.git
cd TunnelWarden

rustup toolchain install 1.94.1 --profile minimal --component rustfmt --component clippy
rustup override set 1.94.1

npm ci
npm run licenses
npm run tauri -- dev
```

首次构建会下载 Rust 依赖及锁定版本的 Oryxis 源码。`npm run licenses` 会根据锁文件生成打包所需的 `THIRD_PARTY_LICENSES.txt`，依赖发生变化后应重新生成。

### 检查与构建

```powershell
# Rust 格式、编译、单元测试、Clippy、源码边界与上游补丁校验，以及前端检查和构建
./scripts/check.ps1 -Frontend

# 构建 Windows EXE / MSI 安装包
npm run bundle
```

安装包输出到 `target/release/bundle/nsis/` 和 `target/release/bundle/msi/`。

### 项目结构

```text
src/                         Svelte 界面与 TypeScript
src-tauri/                   Tauri 命令、事件、托盘与桌面集成
crates/tunnelwarden-core/    隧道运行时、配置、凭据与应用逻辑
vendor/oryxis-ssh/           锁定版本的 SSH 引擎与流量统计补丁
upstream/                   Oryxis 应用层参考源码及补丁记录
scripts/                    构建检查、许可证收集与发布工具
```

SSH、认证和转发逻辑优先复用 Oryxis。修改相关代码前，请阅读 [AGENTS.md](AGENTS.md) 与 [第三方来源说明](THIRD_PARTY_NOTICES.md)；如需重新获取上游应用层参考文件，可运行 `./scripts/fetch-oryxis-reference.ps1`。

## 致谢

TunnelWarden 建立在这些开源项目的工作之上：

- **[Oryxis](https://github.com/wilsonglasser/oryxis)** — 感谢 Wilson Glasser 及贡献者。TunnelWarden 复用了其 SSH 引擎、认证、跳板路由、端口转发与密钥导入能力，并改编了独立转发运行时及日志脱敏逻辑，是本项目最重要的上游。
- **[russh](https://github.com/Eugeny/russh)** — Oryxis 使用的 Rust SSH 实现。
- **[Tauri](https://github.com/tauri-apps/tauri)** — 桌面应用框架与系统集成。
- **[Svelte](https://github.com/sveltejs/svelte)**、**[Vite](https://github.com/vitejs/vite)** 与 **[TypeScript](https://github.com/microsoft/TypeScript)** — 界面开发与前端工具链。
- **[Tokio](https://github.com/tokio-rs/tokio)** — Rust 异步运行时。

也感谢所有间接依赖的维护者与贡献者。完整依赖许可证由 `npm run licenses` 收集，并随安装包分发。

## 许可证

TunnelWarden 采用 **[GNU AGPL-3.0-or-later](LICENSE)** 许可证。

Oryxis 固定于提交 [`16ad2af8`](https://github.com/wilsonglasser/oryxis/tree/16ad2af83b424780ff60b6420a6c20a3b30c7143)。上游版权、改编来源和 SSH 补丁说明见 [NOTICE.ORYXIS](NOTICE.ORYXIS)、[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 与 [PROVENANCE.md](vendor/oryxis-ssh/PROVENANCE.md)。
