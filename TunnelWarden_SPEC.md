# TunnelWarden — Product & Engineering Specification

> **Working name:** TunnelWarden  
> **Tagline:** A native Rust GUI supervisor that keeps SSH tunnels alive.  
> **Primary platform:** Windows 10/11 x86_64  
> **Architecture:** Rust + Tokio + `russh` + GPUI Kit  
> **UI direction:** Loris Tunnel information architecture + original Zed-inspired visual system  
> **License:** Apache-2.0  
> **Status:** Implementation specification for v1.0

---

## 0. 文档目的

本文档定义 **TunnelWarden v1.0** 的完整产品范围、系统架构、状态机、SSH 转发实现、GUI 信息架构、配置模型、异常恢复策略、测试方案和发布门槛。

TunnelWarden 不是一个“附带 Tunnel 功能的终端”，也不是一个 SSH 客户端大全。它只解决一个问题：

> **让 SSH Tunnel 长期、透明、可观测、可恢复地工作。**

项目不采用“先做一个残缺 MVP，后续再补 `-L/-R`、Jump Host、`ssh_config`、流量图”的路线。  
**v1.0 必须一次性包含本文档列出的核心功能。**

开发过程可以内部拆成若干工作包，但 **公开 v1.0 的发布门槛是完整功能集全部实现、全部测试通过。**

---

# 1. 产品定义

## 1.1 核心定位

TunnelWarden 是一个 **SSH Tunnel GUI Supervisor**。

它负责：

- 建立 SSH 连接；
- 建立 Local / Remote / Dynamic forwarding；
- 长期维护这些连接；
- 自动检测失效；
- 自动重连；
- 明确区分“端口已绑定”和“Tunnel 真正健康”；
- 在 GUI 中持续显示真实状态；
- 确保 Stop / Quit 后端口立即释放；
- 确保应用崩溃后不存在孤儿 `ssh.exe`；
- 确保网络切换、服务器重启、短时断网后能够恢复；
- 对端口冲突、Host Key 变化、认证失败等问题给出明确原因；
- 提供完整日志与诊断能力。

TunnelWarden **不是 Terminal Emulator**，不提供 shell terminal、SFTP、AI Agent、Docker 管理等能力。

---

## 1.2 设计原则

### P0. Reliability before convenience

任何功能不得以降低 Tunnel 生命周期可靠性为代价。

### P1. Process existence is not health

绝不能以：

```text
task exists
socket exists
process exists
```

作为“Connected”的依据。

**Connected/Healthy 必须由当前 SSH Session 的可验证状态决定。**

---

### P2. No external `ssh.exe`

TunnelWarden 不调用：

```text
ssh.exe
plink.exe
stnlc.exe
```

SSH 数据面必须由当前进程中的 **`russh`** 实现。

因此：

```text
TunnelWarden.exe
    └── russh
```

而不是：

```text
TunnelWarden.exe
    └── ssh.exe
         └── orphan process
```

应用退出时，Rust 对象销毁、socket 被操作系统释放，不允许残留独立 SSH 子进程继续霸占端口。

---

### P3. Every resource has exactly one owner

每个运行中的 Tunnel 必须有唯一 `TunnelRuntime` 所有者。

该 Runtime 独占：

- cancellation token；
- 本地 listener；
- SSH chain；
- forwarding registration；
- active channels；
- reconnect loop；
- traffic counters；
- health monitor；
- runtime state。

不得由 UI、定时器、后台 callback 分别“顺手”持有资源生命周期。

---

### P4. Desired state 与 Observed state 分离

必须始终区分：

```text
用户希望它 Running
```

和：

```text
它现在实际上 Healthy
```

例如：

```text
desired_state = Running
runtime_state = Reconnecting
listener_state = Bound
ssh_state = Disconnected
```

这是合法状态。

GUI 不得把它显示成 Connected。

---

### P5. Stop is final

用户执行 Stop 后，旧 reconnect task **绝不允许复活**。

必须通过：

- `generation/epoch`
- root `CancellationToken`

共同保证。

---

### P6. Local-first / No cloud dependency

核心功能：

- 不需要账户；
- 不需要 License；
- 不访问商业 API；
- 不依赖服务器；
- 不上传配置；
- 不上传 Machine ID；
- 不做遥测。

更新检查可以访问 GitHub Releases，并且允许关闭。

---

# 2. 与 Loris Tunnel 的关系

TunnelWarden 的产品交互参考 **Loris Tunnel** 的优秀部分：

- 左侧导航结构；
- Overview；
- Jumpers；
- Tunnels；
- Logs；
- Settings；
- Tunnel Group；
- 搜索；
- Tunnel 编辑流程；
- Local / Remote / Dynamic 三种模式；
- Jump Host 管理；
- Auto Start；
- Traffic Monitor；
- Connection Test；
- 配置导入导出；
- `ssh_config` 导入；
- 系统托盘；
- 开机启动。

但是：

1. **不复制 Loris 源码作为工程基础；**
2. 不复制其品牌、Logo、应用名称；
3. 不保留 License / Pro / Upgrade / Machine ID / Usage Heartbeat；
4. 不保留托管 AI Debug；
5. UI 使用 GPUI Kit 原生重写；
6. 视觉设计改为 **Zed-style desktop tool**；
7. Tunnel 生命周期和状态机重新设计，以可靠性为第一目标。

目标是：

> **Feature/workflow parity, not codebase inheritance.**

---

# 3. v1.0 完整功能范围

## 3.1 SSH Connection

必须支持：

- Hostname / IPv4 / IPv6；
- 自定义 SSH Port；
- Username；
- Password auth；
- Private Key auth；
- encrypted private key；
- SSH Agent；
- Keyboard Interactive；
- Host Key verification；
- OpenSSH `known_hosts`；
- Keepalive；
- inactivity timeout；
- connect timeout；
- Jump Host；
- 多级 Jump Host；
- Test Connection。

---

## 3.2 Forwarding

必须支持：

### Local Forward

等价：

```bash
ssh -L 127.0.0.1:5432:10.0.0.8:5432 server
```

---

### Remote Forward

等价：

```bash
ssh -R 127.0.0.1:8080:127.0.0.1:3000 server
```

---

### Dynamic Forward / SOCKS5

等价：

```bash
ssh -D 127.0.0.1:1080 server
```

支持：

- SOCKS5 CONNECT；
- IPv4；
- IPv6；
- Domain name；
- Remote DNS；
- `socks5h` 语义。

v1 不实现：

- SOCKS BIND；
- SOCKS UDP ASSOCIATE。

它们不属于 OpenSSH `-D` 的核心 TCP forwarding 使用场景。

---

## 3.3 Jump Hosts

支持：

```text
Client
  ↓
Jump A
  ↓
Jump B
  ↓
Target
```

不限于单级。

GUI 中显示：

```text
Laptop → bastion-a → bastion-b → gpu-server
```

每一级必须独立：

- 验证 host key；
- 执行认证；
- 记录 RTT；
- 记录错误。

若第三跳失败，错误必须显示：

```text
Jump chain failed at hop 3/3
gpu-server.example.com:22
Authentication rejected
```

而不是简单：

```text
Connection failed
```

---

## 3.4 Tunnel Groups

支持：

- 创建 Group；
- 重命名；
- 删除；
- 拖动排序；
- Tunnel 移动到 Group；
- Ungrouped；
- 隐藏空 Ungrouped；
- Group 一键 Start All；
- Group 一键 Stop All。

---

## 3.5 Auto Start

两个概念严格分开：

### App Auto Run

Windows 登录后启动 TunnelWarden。

### Tunnel Auto Start

TunnelWarden 启动后自动运行指定 Tunnel。

二者均为免费核心功能。

---

## 3.6 Traffic Monitor

每个 Tunnel：

- upload B/s；
- download B/s；
- cumulative upload；
- cumulative download；
- active connections；
- recent throughput history；
- sparkline。

全局：

```text
↑ 1.23 MB/s
↓ 8.42 MB/s
```

默认采样：

```text
1 second
```

保存最近：

```text
120 samples
```

无需将历史流量永久写入数据库。

---

## 3.7 Latency / Health

必须测量 **SSH Session RTT**，而非只做 ICMP ping。

优先调用 `russh` 的 ping/keepalive 能力。

GUI：

```text
Healthy · 38 ms
```

对于 Jump Chain：

```text
bastion-a   21 ms
bastion-b   35 ms
target      42 ms
```

---

## 3.8 `~/.ssh/config` Import

支持选择：

```text
~/.ssh/config
```

以及用户指定文件。

至少识别：

- `Host`
- `HostName`
- `User`
- `Port`
- `IdentityFile`
- `IdentitiesOnly`
- `ProxyJump`
- `ConnectTimeout`
- `ServerAliveInterval`
- `ServerAliveCountMax`
- `HostKeyAlias`
- `UserKnownHostsFile`

对于：

```text
Include
```

尽量解析。

无法映射的 OpenSSH directive：

- 不得导致整个 Import 失败；
- 在 Preview 中标记 Unsupported；
- 保留警告；
- 不静默误解释。

Import 必须：

1. Parse；
2. Preview；
3. Conflict resolution；
4. User confirmation；
5. Write config。

不得直接覆盖现有 Host。

---

# 4. 技术栈

## 4.1 UI

使用：

```toml
gpui-kit = "0.6"
```

GPUI Kit 作为唯一 GPUI facade。

不得：

```toml
gpui = "..."
gpui-component = "..."
```

分别引用不同版本导致类型冲突。

采用：

- `gpui-kit`
- `gpui-base`
- `gpui-component`

其中：

- 基础交互使用 GPUI Kit；
- 视觉层通过 Theme Token 与应用级组件封装实现 Zed 风格；
- 不修改 GPUI Kit upstream 源码。

---

## 4.2 Async Runtime

```text
Tokio
```

单独后台 runtime。

建议：

```text
2 worker threads minimum
```

或：

```text
available_parallelism-based bounded worker count
```

不要在 GPUI render thread 上：

- DNS resolve；
- TCP connect；
- SSH handshake；
- file I/O；
- key decoding；
- tunnel relay。

---

## 4.3 SSH

使用：

```text
russh 0.63.x
```

禁止 shell out 到系统 OpenSSH。

需要使用：

- `client::connect`
- `client::connect_stream`
- `channel_open_direct_tcpip`
- `tcpip_forward`
- `cancel_tcpip_forward`
- `send_ping`
- `keepalive_interval`
- `keepalive_max`
- `inactivity_timeout`
- `ChannelStream`

---

## 4.4 SOCKS5

推荐：

```text
fast-socks5
```

但必须包在项目自己的：

```rust
trait SocksFrontend
```

后面。

业务层只能得到：

```rust
SocksConnectRequest {
    destination: Destination,
}
```

不得让 Tunnel Core 依赖第三方 SOCKS crate 的具体类型。

若第三方 crate 无法可靠地将请求接到 `russh::ChannelStream`，允许自行实现 RFC 1928 中最小的：

```text
NO AUTH
CONNECT
IPv4
IPv6
DOMAIN
```

解析器。

该实现必须独立测试。

---

## 4.5 OpenSSH Config

使用成熟 Rust parser，例如：

```text
ssh2-config
```

外面包：

```text
OpenSshConfigAdapter
```

避免 domain model 直接绑定 parser crate。

---

## 4.6 Secret Storage

不得把密码与 passphrase 写入：

```text
config.toml
```

Windows：

```text
Windows Credential Manager
```

通过 Rust keyring abstraction 管理。

Config 只保存：

```text
credential_ref = "uuid"
```

---

## 4.7 System Tray

使用：

```text
tray-icon
```

或 GPUI 已存在且稳定的 native tray 能力。

必须抽象成：

```rust
trait TrayBackend
```

避免 UI 框架与系统托盘生命周期互相污染。

---

## 4.8 Logging

使用：

```text
tracing
tracing-subscriber
```

禁止散落：

```rust
println!()
```

---

# 5. Workspace Architecture

建议目录：

```text
TunnelWarden/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── LICENSE
├── README.md
├── SPEC.md
├── AGENTS.md
│
├── crates/
│   ├── tunnel-domain/
│   │   ├── src/
│   │   │   ├── host.rs
│   │   │   ├── tunnel.rs
│   │   │   ├── group.rs
│   │   │   ├── state.rs
│   │   │   ├── error.rs
│   │   │   └── event.rs
│   │
│   ├── tunnel-core/
│   │   ├── src/
│   │   │   ├── manager.rs
│   │   │   ├── runtime.rs
│   │   │   ├── supervisor.rs
│   │   │   ├── state_machine.rs
│   │   │   ├── retry.rs
│   │   │   ├── health.rs
│   │   │   ├── traffic.rs
│   │   │   └── shutdown.rs
│   │
│   ├── ssh-engine/
│   │   ├── src/
│   │   │   ├── client.rs
│   │   │   ├── chain.rs
│   │   │   ├── auth.rs
│   │   │   ├── host_key.rs
│   │   │   ├── agent.rs
│   │   │   ├── known_hosts.rs
│   │   │   └── ping.rs
│   │
│   ├── forwarding/
│   │   ├── src/
│   │   │   ├── local.rs
│   │   │   ├── remote.rs
│   │   │   ├── dynamic.rs
│   │   │   ├── socks5.rs
│   │   │   ├── relay.rs
│   │   │   └── counters.rs
│   │
│   ├── config-store/
│   │   ├── src/
│   │   │   ├── schema.rs
│   │   │   ├── storage.rs
│   │   │   ├── migration.rs
│   │   │   ├── import_export.rs
│   │   │   ├── openssh.rs
│   │   │   └── secrets.rs
│   │
│   ├── platform/
│   │   ├── src/
│   │   │   ├── startup.rs
│   │   │   ├── single_instance.rs
│   │   │   ├── tray.rs
│   │   │   ├── paths.rs
│   │   │   ├── network.rs
│   │   │   └── notifications.rs
│   │
│   └── app/
│       ├── src/
│       │   ├── main.rs
│       │   ├── app_state.rs
│       │   ├── bridge.rs
│       │   ├── theme/
│       │   ├── components/
│       │   ├── pages/
│       │   ├── dialogs/
│       │   └── tray_controller.rs
│
└── tests/
    ├── fixtures/
    ├── state_machine/
    ├── integration/
    ├── chaos/
    └── ui/
```

---

# 6. Architecture Rules

## 6.1 Dependency direction

只允许：

```text
app
 ↓
tunnel-core
 ↓
ssh-engine / forwarding
 ↓
tunnel-domain
```

以及：

```text
app → config-store
app → platform
tunnel-core → tunnel-domain
```

绝对禁止：

```text
ssh-engine → app
forwarding → app
tunnel-domain → gpui
```

---

## 6.2 Domain 层必须纯净

`tunnel-domain` 不允许依赖：

- GPUI；
- Tokio networking；
- russh；
- Windows API。

只保存：

- 数据模型；
- enums；
- error code；
- state transition model；
- serializable configuration types。

这样状态机可以做纯单元测试和 property-based test。

---

# 7. 核心 Domain Model

## 7.1 Host / Jumper

```rust
pub struct SshHost {
    pub id: HostId,
    pub name: String,

    pub hostname: String,
    pub port: u16,
    pub username: String,

    pub auth: AuthConfig,

    pub host_key_policy: HostKeyPolicy,

    pub connect_timeout: Duration,
    pub keepalive_interval: Duration,
    pub keepalive_max: u32,

    pub notes: String,
}
```

---

## 7.2 Auth

```rust
pub enum AuthConfig {
    Password {
        credential_ref: SecretRef,
    },

    PrivateKey {
        key_path: PathBuf,
        passphrase_ref: Option<SecretRef>,
    },

    Agent {
        socket: Option<String>,
    },

    KeyboardInteractive,
}
```

允许后续实现 auth fallback sequence，但 v1 配置 UI 以明确选择一种主要方式为主。

---

## 7.3 Tunnel

```rust
pub struct TunnelConfig {
    pub id: TunnelId,
    pub name: String,
    pub group_id: Option<GroupId>,

    pub mode: TunnelMode,

    pub jump_chain: Vec<HostId>,

    pub local: LocalEndpoint,
    pub remote: Option<RemoteEndpoint>,

    pub auto_start: bool,

    pub reconnect: RetryPolicy,

    pub description: String,
}
```

---

## 7.4 Tunnel Mode

```rust
pub enum TunnelMode {
    Local,
    Remote,
    Dynamic,
}
```

---

## 7.5 Desired State

```rust
pub enum DesiredState {
    Stopped,
    Running,
}
```

这是用户意图。

---

# 8. 状态机：最高优先级设计

这是 TunnelWarden 最重要的部分。

## 8.1 不使用一个简单 Boolean

严禁：

```rust
connected: bool
```

严禁：

```rust
is_running: bool
```

来表达完整生命周期。

---

## 8.2 Runtime State

```rust
pub enum RuntimeState {
    Stopped,

    Starting {
        phase: StartPhase,
    },

    Connecting {
        hop: usize,
        total_hops: usize,
        attempt: u32,
    },

    Authenticating {
        hop: usize,
        total_hops: usize,
    },

    EstablishingForward,

    Healthy {
        since: Instant,
    },

    Degraded {
        since: Instant,
        reason: HealthIssue,
    },

    Reconnecting {
        attempt: u32,
        next_retry_at: Instant,
        reason: DisconnectReason,
    },

    Blocked {
        reason: BlockReason,
    },

    Stopping,

    Failed {
        reason: FailureReason,
    },
}
```

---

## 8.3 Start Phase

```rust
pub enum StartPhase {
    Validating,
    AcquiringListener,
    BuildingJumpChain,
    RegisteringRemoteForward,
    StartingHealthMonitor,
}
```

---

## 8.4 Listener State 必须单独表达

```rust
pub enum ListenerState {
    NotRequired,
    Unbound,
    Bound {
        address: SocketAddr,
    },
    Conflict {
        address: SocketAddr,
        owner_hint: Option<String>,
    },
}
```

原因：

```text
127.0.0.1:1080 已被 TunnelWarden 持有
```

并不等价于：

```text
SSH Tunnel Healthy
```

GUI 必须能够显示：

```text
Reconnecting
Local listener reserved on 127.0.0.1:1080
SSH session unavailable
```

绝对不能显示：

```text
Connected
```

---

# 9. 状态机的三个维度

每个 Tunnel Runtime 实际保存：

```rust
struct TunnelRuntimeState {
    desired: DesiredState,
    lifecycle: RuntimeState,
    listener: ListenerState,
    health: HealthState,
}
```

整体 UI 状态由它们派生，而非反过来。

---

# 10. Generation / Epoch 机制

这是防止“旧重连任务复活”的关键。

每个 Tunnel 保存：

```rust
generation: AtomicU64
```

每次：

- Start；
- Stop；
- Restart；
- 修改运行中配置；

都必须：

```text
generation += 1
```

后台 task 启动时捕获：

```rust
let my_generation = runtime.generation();
```

任何异步 side effect 前必须：

```rust
if my_generation != runtime.current_generation() {
    return;
}
```

---

## 10.1 错误案例

旧实现：

```text
Tunnel 掉线
↓
spawn reconnect
↓
用户 Stop
↓
reconnect sleep 5s
↓
task 醒来
↓
重新 bind 1080
```

TunnelWarden 必须变成：

```text
generation 41
↓
掉线 → reconnect task captures 41

用户 Stop
↓
generation 42
↓
cancel token

reconnect task wakes
↓
41 != 42
↓
return immediately
```

---

# 11. Cancellation Tree

每个 Tunnel：

```text
Tunnel Root CancellationToken
│
├── listener accept loop
├── SSH session task
├── health monitor
├── reconnect supervisor
├── traffic sampler
└── connection tasks
    ├── relay #1
    ├── relay #2
    └── ...
```

Stop：

```text
root.cancel()
```

所有子任务都必须可取消。

不得让：

```text
sleep()
read()
connect()
accept()
```

成为不可打断的无限阻塞。

使用：

```rust
tokio::select! {
    _ = cancellation.cancelled() => ...
    result = operation => ...
}
```

---

# 12. Single Instance：必须实现

NyaTerm 类问题中一个危险来源就是多实例抢同一端口。

Windows 上：

```text
TunnelWarden.exe
TunnelWarden.exe
```

不得同时启动两个独立 runtime。

实现：

```text
Named Mutex
```

例如：

```text
Global\TunnelWarden.<user-sid>
```

第二实例启动：

1. 检测已存在；
2. 不建立任何 Tunnel；
3. 向第一实例发送“show window”；
4. 第一实例恢复窗口并聚焦；
5. 第二实例退出。

Linux/macOS 后续通过 lock file / platform native mechanism 实现相同行为。

---

# 13. Tunnel 生命周期

## 13.1 Start

严格顺序：

```text
User Start
↓
increment generation
↓
create root cancellation token
↓
validate config
↓
validate jump chain
↓
acquire local listener if Local/Dynamic
↓
build SSH chain
↓
authenticate final target
↓
establish Remote Forward if needed
↓
start health monitor
↓
mark Healthy
```

---

## 13.2 Local/Dynamic Listener 策略

对于 `-L` 和 `-D`：

**Listener 归 Tunnel Runtime 所有。**

一旦用户希望 Tunnel Running：

```text
desired = Running
```

默认保持 listener ownership，即便 SSH 暂时 reconnecting。

理由：

1. 防止其它程序趁断线抢占固定端口；
2. 重连恢复时无需重新 bind；
3. 代理客户端配置始终指向同一个地址。

但是必须避免“NyaTerm 假健康”。

### Dynamic reconnecting 行为

如果 SOCKS client 在 SSH 不健康时连接：

返回标准 SOCKS failure，例如：

```text
Host unreachable / General failure
```

不得：

- 接收后无限 hang；
- 假装握手成功；
- 默默丢包。

### Local reconnecting 行为

对于 raw Local forward：

- accept 后立即 graceful close / reset；
- 不无限挂起等待 SSH 重连。

---

## 13.3 Stop

必须按以下顺序：

```text
DesiredState = Stopped
↓
generation += 1
↓
root cancellation token cancel
↓
stop listener accept
↓
cancel pending reconnect
↓
cancel active relay tasks
↓
cancel remote forwarding registration
↓
SSH disconnect
↓
close SSH chain from target back to first hop
↓
drop listeners
↓
join tasks with deadline
↓
RuntimeState = Stopped
```

正常 shutdown deadline：

```text
3 seconds
```

超过 deadline：

- abort remaining owned tasks；
- drop runtime；
- 记录 warning；
- 仍不得遗留外部进程。

---

# 14. Reconnect Supervisor

## 14.1 Transient Error

以下属于可重试：

- TCP reset；
- connection timeout；
- DNS temporary failure；
- Wi-Fi/network unavailable；
- SSH unexpected EOF；
- SSH keepalive timeout；
- remote host temporarily unavailable；
- server restart；
- jump host temporary unavailable；
- remote forwarding lost。

进入：

```text
Reconnecting
```

---

## 14.2 Blocked Error

以下不得无限重试：

- password rejected；
- private key rejected；
- private key missing；
- host key changed；
- malformed hostname；
- invalid local port；
- local bind conflict；
- impossible jump-chain reference；
- unsupported configuration；
- permission denied for remote forwarding。

进入：

```text
Blocked
```

直到：

- 用户修改配置；
- 用户手动 Retry；
- 相关本地资源状态明确变化。

---

## 14.3 Backoff

默认：

```text
1s
2s
5s
10s
20s
30s
30s
30s ...
```

加入：

```text
±20% jitter
```

防止多个 Tunnel 同时重连造成 thundering herd。

---

## 14.4 Backoff Reset

连续 Healthy：

```text
>= 60 seconds
```

后：

```text
attempt = 0
```

短暂连接 2 秒又断：

不得重置 retry counter。

---

## 14.5 Network Recovery

Platform layer 监听网络变化。

检测到：

```text
offline → online
```

时：

- 正在等待 backoff 的 Tunnel 立即触发一次 early retry；
- 不需要等待剩余 30 秒。

该通知只是：

```text
retry hint
```

不能直接把状态标记为 Healthy。

---

# 15. SSH Health Model

## 15.1 Healthy 条件

Healthy 至少要求：

- SSH chain 完整；
- final session authenticated；
- forwarding 已建立；
- 最近 ping 成功；
- runtime task 未退出。

---

## 15.2 Ping

默认：

```text
interval: 10s
timeout: 5s
```

连续失败：

```text
1 次 → Degraded
3 次 → Session considered dead
```

之后：

```text
Reconnecting
```

---

## 15.3 Degraded

Degraded 是短暂状态：

```text
Healthy
↓
one health failure
↓
Degraded
```

GUI：

```text
Unstable · last response 13s ago
```

若下一 ping 恢复：

```text
Healthy
```

若超过阈值：

```text
Reconnecting
```

---

# 16. SSH Chain 实现

## 16.1 第一跳

```text
TcpStream
↓
russh::client::connect
↓
authenticate
```

---

## 16.2 后续 Jump Hop

假设：

```text
A → B
```

在 A session 中：

```text
channel_open_direct_tcpip(B, 22)
```

得到 SSH Channel。

将 Channel 转成：

```text
ChannelStream
```

然后：

```text
russh::client::connect_stream(...)
```

在该 stream 上建立 B 的 SSH Session。

重复即可：

```text
A → B → C → Target
```

---

## 16.3 Chain Ownership

```rust
struct SshChain {
    sessions: Vec<SshHopSession>,
}
```

顺序：

```text
[hop1, hop2, ..., target]
```

关闭时反向：

```text
target
↓
hopN
↓
...
↓
hop1
```

---

## 16.4 不共享 SSH Session

v1 中，不同 Tunnel **不共享** SSH session。

例如：

```text
Tunnel A → server X
Tunnel B → server X
```

建立两条独立 session。

理由：

- blast radius 小；
- 一个 tunnel reconnect 不影响另一个；
- lifecycle ownership 清楚；
- 状态机简单；
- 调试容易；
- 不产生 session pool stale-state 问题。

少量额外 TCP/SSH 开销可以接受。

---

# 17. Host Key Verification

默认：

```text
Strict
```

顺序：

1. 读取 `~/.ssh/known_hosts`；
2. 读取 TunnelWarden app-local known_hosts；
3. 匹配 host + port；
4. unknown 时 GUI 弹窗；
5. mismatch 时 Blocked。

Unknown Host：

```text
The authenticity of host cannot be established.

ED25519
SHA256:...

[Trust Once]
[Trust & Save]
[Cancel]
```

Changed Host Key：

```text
HOST KEY CHANGED
```

必须使用危险状态视觉。

不得提供一个模糊的：

```text
Ignore
```

按钮。

高级设置可以允许：

```text
Bypass host verification
```

但：

- 默认 off；
- 显示 warning；
- 每个 Host 独立配置。

---

# 18. Authentication

认证过程必须返回明确阶段。

例如：

```text
Connecting
Host key verified
Trying private key ~/.ssh/id_ed25519
Authentication succeeded
```

失败：

```text
Authentication rejected
Method: publickey
Server allows: publickey,password
```

UI 不显示秘密值。

---

# 19. Local Forward

工作流：

```text
TcpListener
127.0.0.1:5432
↓
accept
↓
SSH Handle
↓
channel_open_direct_tcpip(
    remote_host,
    remote_port
)
↓
bidirectional relay
```

每个 accepted connection 是独立 child task。

---

# 20. Dynamic SOCKS5

## 20.1 数据流

```text
Application
    │
    │ SOCKS5
    ▼
127.0.0.1:1080
    │
    ▼
TunnelWarden
    │
    │ direct-tcpip
    ▼
SSH Server
    │
    ▼
Destination
```

---

## 20.2 Remote DNS

若 SOCKS 请求：

```text
ATYP = DOMAIN
host = huggingface.co
```

不得先在本机：

```text
DNS resolve
```

而是直接：

```text
channel_open_direct_tcpip(
    "huggingface.co",
    443
)
```

因此实现：

```text
socks5h
```

语义。

---

## 20.3 SOCKS Failure Mapping

SSH channel open error 必须映射到适当 SOCKS reply。

例如：

```text
Connection refused → Connection refused
Network unreachable → Network unreachable
Timeout → Host unreachable / TTL expired as appropriate
Other → General SOCKS server failure
```

不得成功握手后 silent hang。

---

# 21. Remote Forward

启动：

```text
SSH Session
↓
tcpip_forward(remote_bind, remote_port)
```

远端连接时 Handler 收到：

```text
forwarded-tcpip
```

TunnelWarden：

```text
forwarded channel
↓
TcpStream::connect(local_target)
↓
relay
```

Stop 时必须：

```text
cancel_tcpip_forward
```

再关闭 SSH Session。

---

# 22. Relay 语义

必须正确处理：

- backpressure；
- half-close；
- EOF；
- cancellation；
- byte counters。

不得简单 spawn 两个无限 copy loop 而没有统一生命周期。

建议抽象：

```rust
relay_bidirectional(
    left,
    right,
    cancellation,
    counters
)
```

当本地写侧 EOF：

- 向 SSH channel 发送 EOF；
- 保留反方向读取，直到 remote EOF 或 cancel。

特别注意 `russh` channel 的 EOF 语义，避免大传输结束时 hang。

---

# 23. Port Conflict

启动前：

```text
bind()
```

是唯一可信判断。

不得先：

```text
netstat
```

再 bind。

若 bind 返回：

```text
AddrInUse
```

状态：

```text
Blocked
reason = PortConflict
```

UI：

```text
Port 1080 is already in use
127.0.0.1:1080

[Retry]
[Change Port]
[Inspect]
```

Windows `Inspect` 可以额外使用系统 API / `GetExtendedTcpTable` 查 PID。

这只是诊断信息，不参与 ownership 判定。

---

# 24. 防止 1080 “鬼占用”的硬性不变量

以下全部必须成立。

### Invariant 1

```text
RuntimeState = Stopped
```

时：

该 Tunnel 不得拥有任何 listener。

### Invariant 2

```text
DesiredState = Stopped
```

后旧 generation 不得重新创建 listener。

### Invariant 3

App Quit 后不存在：

```text
ssh.exe
plink.exe
```

等 TunnelWarden 创建的 SSH 子进程。

因为根本不创建。

### Invariant 4

同一 Tunnel 只能存在一个 root supervisor task。

### Invariant 5

同一 Tunnel generation 只能存在一个 listener owner。

### Invariant 6

`Healthy` 必须对应当前 generation 的有效 SSH session。

### Invariant 7

配置修改产生新 generation，旧 generation 所有异步结果必须被丢弃。

---

# 25. Config Storage

默认：

```text
%APPDATA%\TunnelWarden\
├── config.toml
├── known_hosts
├── logs\
└── backups\
```

支持用户改变 config data directory。

---

## 25.1 TOML

主配置采用：

```text
TOML
```

理由：

- 人类可读；
- Git-friendly；
- 易 debug；
- 不需要数据库。

---

## 25.2 Schema Version

必须：

```toml
schema_version = 1
```

未来升级使用 migration。

不得无版本直接解析。

---

## 25.3 Atomic Write

保存：

```text
config.toml.tmp
↓
fsync
↓
atomic replace
```

保存前保留：

```text
backups/config-<timestamp>.toml
```

最多保留：

```text
5
```

份。

---

## 25.4 Example

```toml
schema_version = 1

[app]
theme = "system"
run_at_startup = true
minimize_to_tray = true
traffic_monitor = true

[[hosts]]
id = "lab"
name = "Lab A40"
hostname = "lab.example.com"
port = 22
username = "user"
auth_type = "private_key"
identity_file = "~/.ssh/id_ed25519"
connect_timeout_ms = 8000
keepalive_interval_ms = 10000
keepalive_max = 3

[[groups]]
id = "proxy"
name = "Proxy"

[[tunnels]]
id = "lab-socks"
name = "Lab SOCKS"
group_id = "proxy"
mode = "dynamic"
jump_chain = ["lab"]
local_host = "127.0.0.1"
local_port = 1080
auto_start = true
description = "Primary SOCKS5 proxy"
```

---

# 26. Configuration Import / Export

Export 默认不包含秘密。

输出：

```text
tunnelwarden-config.toml
```

Secrets：

```text
credential_ref
```

可以选择：

```text
Export portable bundle
```

但 v1 默认仍不导出密码。

---

# 27. OpenSSH Config Import UI

流程：

```text
Settings
↓
Import SSH Config
↓
Choose file
↓
Parse
↓
Preview table
↓
Resolve conflicts
↓
Import
```

Preview：

| Import | Host | Target | User | Identity | ProxyJump | Status |
|---|---|---|---|---|---|---|
| ✓ | lab | 10.0.0.2 | user | id_ed25519 | bastion | Ready |
| ✓ | bastion | 1.2.3.4 | user | id_ed25519 | — | Ready |
| □ | old | ... | ... | ... | ... | Duplicate |

---

# 28. Application Runtime

## 28.1 GPUI Thread

只负责：

- UI State；
- rendering；
- user actions；
- modal；
- keyboard；
- tray bridge。

---

## 28.2 Tokio Runtime

负责：

- SSH；
- DNS；
- TCP；
- relay；
- reconnect；
- health；
- logs sink；
- file operations。

---

## 28.3 Bridge

GUI → Core：

```rust
enum CoreCommand {
    StartTunnel(TunnelId),
    StopTunnel(TunnelId),
    RestartTunnel(TunnelId),
    StartGroup(GroupId),
    StopGroup(GroupId),
    TestHost(HostId),
    TestTunnel(TunnelId),
    ReloadConfig,
}
```

Core → GUI：

```rust
enum CoreEvent {
    TunnelSnapshot(TunnelSnapshot),
    TunnelTransition(StateTransition),
    TrafficUpdate(TrafficSnapshot),
    Log(LogEvent),
    HostKeyPrompt(HostKeyPrompt),
    CredentialPrompt(CredentialPrompt),
}
```

UI 不得直接调用 `russh`。

---

# 29. Actor / Single Writer 模型

`TunnelManager` 是 Runtime 状态唯一写者。

UI：

```text
Command
↓
TunnelManager
↓
Runtime
↓
Event
↓
UI
```

严禁 UI 与后台 task 同时直接 mutation 同一个 state struct。

---

# 30. Error Model

错误必须结构化。

```rust
pub enum TunnelErrorKind {
    Network,
    Dns,
    Timeout,

    Authentication,
    HostKeyUnknown,
    HostKeyMismatch,

    PortConflict,
    PermissionDenied,

    RemoteForwardRejected,

    InvalidConfig,

    Cancelled,

    Internal,
}
```

每个 error：

```rust
pub struct TunnelError {
    pub kind: TunnelErrorKind,
    pub stage: FailureStage,
    pub message: String,
    pub retryability: Retryability,
    pub source_chain: Vec<String>,
}
```

---

# 31. Failure Stage

```rust
pub enum FailureStage {
    ConfigValidation,
    BindLocalPort,
    ResolveHost,
    ConnectTcp,
    SshHandshake,
    HostKeyVerification,
    Authentication,
    BuildJumpChain,
    RegisterForward,
    Relay,
    HealthCheck,
    Shutdown,
}
```

这样日志可以明确：

```text
Tunnel "Lab SOCKS"
FailureStage: Authentication
Hop: 2/2
Error: public key rejected
Retryability: Blocked
```

---

# 32. Logs

## 32.1 GUI Logs

页面字段：

```text
Time
Level
Tunnel
Stage
Message
```

过滤：

- All；
- Debug；
- Info；
- Warning；
- Error；
- Tunnel；
- Host。

支持搜索。

---

## 32.2 Disk Logs

使用 rolling log。

默认：

```text
10 MB × 5 files
```

---

## 32.3 Secret Redaction

日志绝不得包含：

- passwords；
- key passphrases；
- full private keys；
- credential blobs；
- imported secret env values。

---

# 33. Diagnostics

不使用 AI。

提供：

```text
Diagnose
```

产生确定性诊断：

```text
✓ Local port can bind
✓ DNS resolved
✓ TCP connection established
✓ SSH handshake completed
✓ Host key accepted
✗ Authentication failed
```

支持：

```text
Copy diagnostic report
```

必须自动 redact：

- password；
- passphrase；
- private key；
- tokens。

---

# 34. UI Information Architecture

主导航严格保持简单：

```text
Overview
Jumpers
Tunnels
Logs

────────

Settings
```

这与 Loris 的信息架构接近，但视觉重新设计。

---

# 35. Zed-inspired Design Language

不是 pixel-copy Zed，而是提取其设计语言：

- dense；
- low-chrome；
- desktop-native；
- information-first；
- restrained borders；
- subtle elevation；
- compact controls；
- code-editor-like precision；
- clear status colors；
- minimal decorative gradients。

---

# 36. Window

默认：

```text
1180 × 760
```

最小：

```text
920 × 600
```

布局：

```text
┌──────────────────────────────────────────────┐
│ title / window controls                      │
├────────────┬─────────────────────────────────┤
│ Sidebar    │ Content                         │
│            │                                 │
│ Overview   │                                 │
│ Jumpers    │                                 │
│ Tunnels    │                                 │
│ Logs       │                                 │
│            │                                 │
│ Settings   │                                 │
└────────────┴─────────────────────────────────┘
```

---

# 37. Sidebar

Expanded：

```text
width 208 px
```

Collapsed：

```text
width 48 px
```

Item：

```text
height 32 px
```

不要大面积卡片菜单。

Active item：

- subtle background；
- 2 px accent indicator；
- label medium weight。

---

# 38. Typography

Windows：

```text
Segoe UI Variable
```

Fallback：

```text
system-ui
```

Monospace：

```text
Cascadia Mono
Consolas
```

不得随程序打包未经必要许可的字体文件。

---

# 39. Theme Tokens

所有颜色必须语义化。

示例：

```rust
struct AppTheme {
    bg: Hsla,
    surface: Hsla,
    surface_hover: Hsla,
    surface_active: Hsla,

    border: Hsla,
    border_strong: Hsla,

    text: Hsla,
    text_muted: Hsla,

    accent: Hsla,

    success: Hsla,
    warning: Hsla,
    danger: Hsla,
    info: Hsla,
}
```

业务代码不得出现大量：

```text
#181818
#252525
```

硬编码。

---

# 40. Status Visual Semantics

```text
Healthy       green
Degraded      yellow
Reconnecting  amber
Blocked       red
Failed        red
Stopped       neutral gray
Starting      blue
```

颜色不是唯一信息。

必须同时显示：

- icon；
- text；
- tooltip。

满足 accessibility。

---

# 41. Overview Page

顶部：

```text
3 Healthy
1 Reconnecting
0 Blocked
↑ 1.2 MB/s
↓ 6.8 MB/s
```

主体是 Tunnel rows，不用巨大 dashboard tiles。

示意：

```text
Lab SOCKS                                      ● Healthy
Dynamic · 127.0.0.1:1080                       38 ms

Lab → GPU Server
↑ 420 KB/s                ↓ 5.8 MB/s
▁▂▂▃▅▆▃▂                    ▁▂▅▇▆▄▂▃

3 active connections                          [Stop]
```

Reconnecting：

```text
Lab SOCKS                                   ◐ Reconnecting
SOCKS5 · 127.0.0.1:1080

SSH unavailable
Retry 4 · next attempt in 8.2 s

Local listener remains reserved
                                             [Stop] [Retry now]
```

这一状态文案非常重要。

---

# 42. Jumpers Page

顶部：

```text
Jumpers
[Search]                         [+ New Jumper]
```

表格：

```text
Name
Host
User
Auth
Latency
Last Status
```

Row actions：

- Test；
- Edit；
- Duplicate；
- Delete。

---

# 43. Jumper Editor

分两个 section。

### Connection

- Name
- Host
- Port
- User

### Authentication

- Password
- Private Key
- SSH Agent
- Keyboard Interactive

### Security

- Host Key Policy

### Reliability

- Connect timeout
- Keepalive interval
- Keepalive max

### Notes

---

# 44. Tunnels Page

顶部：

```text
Tunnels
[Search]            [Start All] [+ New Tunnel]
```

Group：

```text
Proxy
 ├─ Lab SOCKS
 └─ Browser Proxy

Databases
 ├─ PostgreSQL
 └─ Redis
```

支持 drag reorder。

---

# 45. Tunnel Editor

### Basic

- Name；
- Group；
- Description。

### Mode

Segment control：

```text
Local | Remote | Dynamic
```

---

## Local

```text
Listen
Host: 127.0.0.1
Port: 5432

Destination
Host: 10.0.0.5
Port: 5432
```

---

## Remote

```text
Remote Listen
Host: 127.0.0.1
Port: 8080

Local Destination
Host: 127.0.0.1
Port: 3000
```

---

## Dynamic

```text
SOCKS5 Listen
Host: 127.0.0.1
Port: 1080

Remote DNS: automatic
```

若用户输入：

```text
0.0.0.0
```

显示：

```text
This exposes the proxy to other devices on the network.
```

---

# 46. Jump Chain Editor

使用 ordered list：

```text
1  Bastion US
2  Internal Gateway
3  GPU Server
```

支持：

- Add；
- Remove；
- drag reorder。

最终 host 本身也是 chain endpoint。

---

# 47. Logs Page

类似 Zed diagnostics/log view。

```text
13:31:02 INFO  Lab SOCKS   SSH connection established
13:31:02 INFO  Lab SOCKS   Auth succeeded
13:31:02 INFO  Lab SOCKS   SOCKS5 listening 127.0.0.1:1080
13:34:17 WARN  Lab SOCKS   Keepalive timeout
13:34:18 INFO  Lab SOCKS   Reconnecting attempt 1
```

底部：

```text
Auto scroll [✓]
```

---

# 48. Settings

Sections：

### General

- Theme；
- Language；
- Minimize to tray；
- Start app at login。

### Tunnel Runtime

- Traffic monitor；
- default retry policy；
- default keepalive。

### Configuration

- Import config；
- Export config；
- Open config directory；
- Move config directory；
- Reset config directory。

### SSH

- known_hosts path；
- app known_hosts；
- default security policy。

### Updates

- Current version；
- Check updates；
- automatic update check toggle。

### About

- Open Source；
- Apache-2.0；
- GitHub link。

不存在：

```text
Account
License
Pro
Upgrade
Machine ID
```

---

# 49. Tray Behavior

Tray tooltip：

```text
TunnelWarden
3 connected · 1 reconnecting
```

Menu：

```text
Open TunnelWarden
────────────────
Lab SOCKS            ●
PostgreSQL           ●
Redis                ○
────────────────
Start All
Stop All
────────────────
Quit
```

点击 Tunnel：

toggle Start/Stop。

---

# 50. Close / Quit Semantics

Window close：

若有 Tunnel Running：

默认：

```text
Hide to tray
```

不停止 Tunnel。

真正 Quit：

```text
Stop all
↓
wait shutdown deadline
↓
release listeners
↓
quit process
```

用户可以在 Settings 修改：

```text
Close window → Quit
```

但必须明确提示。

---

# 51. Startup

Windows 开机启动采用 per-user 方式，不需要管理员权限。

应用以：

```text
--background
```

启动。

行为：

```text
start app
↓
load config
↓
create runtime
↓
start auto_start tunnels
↓
stay in tray
```

如果某 Tunnel 端口冲突：

- 其它 Tunnel 继续启动；
- 该 Tunnel Blocked；
- tray warning；
- 不阻塞整个程序。

---

# 52. Network Change

Platform layer 提供：

```rust
enum NetworkEvent {
    Offline,
    Online,
    InterfaceChanged,
}
```

Core 不依赖 Windows API 类型。

Windows implementation 可使用系统网络变化 notification。

---

# 53. Traffic Counters

Relay 层更新：

```rust
AtomicU64 uploaded
AtomicU64 downloaded
AtomicU32 active_connections
```

UI sampler：

```text
1 Hz
```

计算：

```text
delta bytes / elapsed
```

不要每个 packet 发 UI event。

---

# 54. Performance Targets

目标，不允许明显回退。

### Idle CPU

10 个 Healthy Tunnel、无业务流量：

```text
< 0.5% average
```

### Memory

1 Tunnel：

```text
target < 60 MB working set
```

20 Tunnel：

```text
target < 100 MB
```

不是为了追求一个漂亮数字而牺牲正确性，但不得出现 Electron 级数百 MB 常驻。

### UI

保持：

```text
60 FPS
```

正常交互。

流量图无需超过 UI refresh rate。

---

# 55. Security Requirements

## 55.1 Default bind

Dynamic / Local 默认：

```text
127.0.0.1
```

不是：

```text
0.0.0.0
```

---

## 55.2 Secrets

必须：

- keyring；
- memory redaction；
- Debug 不打印；
- panic report 不打印。

---

## 55.3 Config Permissions

尽可能将 config directory 设置为当前用户可访问。

Private key 不复制到 TunnelWarden directory。

只保存原路径。

---

## 55.4 No shell interpolation

任何 host/path/config 不得构造：

```text
cmd /c ...
powershell ...
sh -c ...
```

来执行 SSH。

核心路径全部 native Rust。

---

# 56. Update Check

只做：

```text
current version
↓
GitHub Releases
↓
latest version
```

不发送：

- machine ID；
- hostname；
- tunnel count；
- usage data。

Update check 可关闭。

v1 可以先提供：

```text
Download release page
```

不要求实现静默 auto-update。

---

# 57. Testing Strategy

可靠性不是人工点几次 GUI。

必须建立自动化测试。

---

# 58. State Machine Unit Tests

状态机必须纯函数化到足以测试：

```rust
transition(state, event) -> next_state + effects
```

至少覆盖：

- Start；
- Stop；
- connect success；
- connect timeout；
- auth failure；
- host key failure；
- ping failure；
- reconnect；
- retry success；
- config changed；
- stop during backoff；
- stop during connect；
- stop during authentication；
- stop during relay；
- old generation callback。

---

# 59. Property-based Tests

使用：

```text
proptest
```

测试 invariant。

例如随机事件序列：

```text
Start
NetworkDown
Retry
Stop
NetworkUp
OldConnectSuccess
Start
...
```

必须始终保证：

```text
DesiredState::Stopped
⇒ old generation cannot become Healthy
```

---

# 60. Integration SSH Environment

CI 使用 Docker / containerized OpenSSH。

构建：

```text
client test
    │
    ├── ssh-server-a
    ├── bastion
    └── target
```

测试：

- key auth；
- password auth；
- jump；
- local；
- remote；
- dynamic。

---

# 61. Dynamic SOCKS Test

自动启动 TunnelWarden core：

```text
127.0.0.1:1080
```

通过 SOCKS5 请求 test HTTP service。

验证：

```text
response correct
```

同时测试 domain ATYP，确认本地 resolver 不参与。

---

# 62. Port Conflict Test

流程：

1. test process bind `127.0.0.1:1080`；
2. Start Tunnel；
3. Tunnel → Blocked/PortConflict；
4. UI/core snapshot 明确；
5. release test listener；
6. Retry；
7. Tunnel Healthy。

---

# 63. Stop-during-reconnect Test

核心 regression test。

```text
Tunnel Healthy
↓
break SSH
↓
Tunnel Reconnecting
↓
Stop
↓
wait longer than retry backoff
↓
assert 1080 free
↓
assert no SSH session
↓
assert state Stopped
```

这是防止 NyaTerm 类型 bug 的强制测试。

---

# 64. Generation Regression Test

模拟：

```text
generation 10 connect future
↓
Stop
↓
generation 11
↓
old future returns Success
```

断言：

```text
state != Healthy
no listener created
no session retained
```

---

# 65. Server Restart Test

```text
Healthy
↓
restart sshd
↓
Degraded
↓
Reconnecting
↓
Healthy
```

Tunnel config 不变化。

Dynamic listener 地址不变化。

---

# 66. Network Blackhole Test

不是简单 TCP close，而要模拟 packet blackhole。

验证 keepalive 能够识别：

```text
half-open connection
```

而不是永远认为 TCP socket 存在。

---

# 67. Jump Chain Tests

至少：

### 1 hop

```text
client → target
```

### 2 hop

```text
client → bastion → target
```

### 3 hop

```text
client → bastion A → bastion B → target
```

同时测试：

- 中间 hop 认证失败；
- 中间 hop reboot；
- final target reboot。

---

# 68. Remote Forward Test

远端监听：

```text
remote:18080
```

通过远端连接访问本地 test service。

Stop 后：

```text
remote forwarding removed
```

---

# 69. Soak Test

Release Candidate 必须：

```text
24 hours
```

持续运行。

场景：

- Dynamic SOCKS5；
- sustained periodic traffic；
- 每小时主动切断 SSH；
- 自动重连；
- 每次恢复后继续流量。

监控：

- memory；
- task count；
- file handles；
- sockets；
- reconnect count。

不得出现线性增长。

---

# 70. 1000 Reconnect Stress

自动执行：

```text
connect
disconnect
reconnect
```

1000 次。

验收：

- 无 panic；
- 无死锁；
- 无 stale listener；
- 内存无明显持续增长；
- task 数恢复到基线。

---

# 71. UI Tests

利用 GPUI Kit 提供的 UI testing 能力测试：

- Sidebar navigation；
- New Tunnel dialog；
- mode switch；
- blocked state；
- reconnecting state；
- host key prompt；
- logs filter；
- settings。

重点不是像素截图，而是：

- state；
- focus；
- accessibility；
- interaction。

---

# 72. 五项绝对验收标准

这五项是项目的最低生命线。

## Gate 1

**连续运行 24h，1080 不假死。**

---

## Gate 2

Wi-Fi / 网络断开后恢复：

**Tunnel 自动恢复。**

---

## Gate 3

SSH Server 重启：

**Tunnel 自动恢复。**

---

## Gate 4

点击 Stop：

**1080 必须立即释放。**

测试必须实际：

```text
bind 127.0.0.1:1080
```

确认成功，而不是只看 UI。

---

## Gate 5

强制结束 TunnelWarden：

**不得存在残留 SSH 子进程。**

由于无外部 ssh process，此项应天然成立；同时确认端口被 OS 释放。

---

# 73. v1.0 Full Release Gates

除了上述五项，全部要求：

- `-L` passed；
- `-R` passed；
- `-D` passed；
- SOCKS5 remote DNS passed；
- multi-hop passed；
- SSH config import passed；
- traffic stats passed；
- system tray passed；
- run-at-startup passed；
- host key verification passed；
- password/private-key/agent auth passed；
- config import/export passed；
- single-instance passed；
- 1000 reconnect stress passed；
- 24h soak passed；
- cargo test passed；
- clippy warnings = 0；
- rustfmt clean；
- cargo audit reviewed。

**缺任何一项，不发布 v1.0。**

---

# 74. Internal Development Work Packages

这些只是实现顺序，不意味着削减 v1 功能。

## WP0 — Foundation

- Cargo workspace；
- domain model；
- tracing；
- config schema；
- Tokio bridge；
- GPUI Kit app shell；
- theme；
- CI。

---

## WP1 — SSH Engine

- direct connect；
- known_hosts；
- private key；
- password；
- agent；
- ping；
- structured errors；
- jump chain。

---

## WP2 — Forwarding

- relay；
- Local；
- Dynamic SOCKS5；
- Remote；
- traffic counters。

---

## WP3 — Supervisor

- lifecycle；
- generation；
- cancellation tree；
- reconnect；
- backoff；
- health；
- Stop semantics；
- single instance。

---

## WP4 — GUI

- Overview；
- Jumpers；
- Tunnels；
- Groups；
- Editors；
- Logs；
- Settings；
- dialogs。

---

## WP5 — Desktop Integration

- tray；
- startup；
- network notification；
- config folder；
- keyring；
- notifications；
- update check。

---

## WP6 — Import & Diagnostics

- ssh_config；
- config import/export；
- test connection；
- deterministic diagnostics。

---

## WP7 — Hardening

- chaos tests；
- reconnect stress；
- soak；
- performance；
- leak inspection；
- Windows packaging。

只有 WP0–WP7 全部完成才称 v1。

---

# 75. Windows Packaging

提供：

### Portable

```text
TunnelWarden-x86_64-windows.zip
```

### Installer

可使用：

- WiX；
- Inno Setup；

二者选一。

安装不要求管理员权限作为默认路径。

---

# 76. CI

GitHub Actions 至少：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
cargo audit
cargo deny check
```

Windows 是 required runner。

Linux runner 用于 core integration test。

---

# 77. Dependency Policy

原则：

- SSH / crypto 不自己造轮子；
- Tunnel lifecycle 自己掌握；
- UI 不引入 WebView；
- 不引入 Node/npm runtime；
- 不引入 Electron；
- 不引入数据库；
- 不引入商业 SDK；
- 不引入 telemetry SDK。

新增 dependency 必须说明：

1. 为什么需要；
2. 是否活跃维护；
3. license；
4. 是否引入 native DLL；
5. 是否能被小型自有实现替代。

---

# 78. Suggested Dependencies

参考，不要求逐字固定版本：

```toml
[dependencies]
gpui-kit = "0.6"

tokio = { version = "1", features = [
    "rt-multi-thread",
    "net",
    "io-util",
    "time",
    "sync",
    "macros",
] }

tokio-util = { version = "0.7", features = ["rt"] }

russh = { version = "0.63", features = ["ring"] }

serde = { version = "1", features = ["derive"] }
toml = "1"

thiserror = "2"
anyhow = "1"

tracing = "0.1"
tracing-subscriber = "0.3"

uuid = { version = "1", features = ["v4", "serde"] }

ssh2-config = "0.7"

tray-icon = "0.25"

# select suitable keyring backend
keyring = "4"
```

SOCKS library根据 integration prototype 后确定。

---

# 79. Code Quality Rules for Agents

`AGENTS.md` 必须写入：

### Rule 1

不要为了通过编译删除状态。

### Rule 2

不得把 Supervisor 简化成：

```rust
loop {
    if error {
        sleep();
        reconnect();
    }
}
```

### Rule 3

任何新增 async task 必须回答：

```text
Who owns it?
How is it cancelled?
What generation does it belong to?
Who joins it?
```

### Rule 4

任何 socket 必须回答：

```text
Who owns it?
When is it dropped?
Can Stop guarantee release?
```

### Rule 5

UI 不得推断 core health。

只能渲染 Core Snapshot。

### Rule 6

禁止通过 process existence 判断 connection health。

### Rule 7

禁止引入隐藏遥测、账户、License、Pro gating。

---

# 80. Runtime Snapshot

Core 定期/事件驱动产生：

```rust
pub struct TunnelSnapshot {
    pub id: TunnelId,

    pub desired_state: DesiredState,
    pub runtime_state: RuntimeStateView,
    pub listener_state: ListenerStateView,

    pub generation: u64,

    pub latency_ms: Option<u32>,

    pub upload_bps: u64,
    pub download_bps: u64,

    pub upload_total: u64,
    pub download_total: u64,

    pub active_connections: u32,

    pub connected_since: Option<DateTime<Utc>>,

    pub last_error: Option<ErrorSummary>,
}
```

UI 只消费 Snapshot。

---

# 81. Event Journal

为调试复杂状态机，每个 Tunnel 保留最近：

```text
200 transitions
```

内存 journal。

例如：

```text
13:20:01 Stopped → Starting
13:20:01 Starting → Connecting
13:20:02 Connecting → Authenticating
13:20:02 Authenticating → Healthy
13:44:17 Healthy → Degraded
13:44:27 Degraded → Reconnecting
13:44:29 Reconnecting → Healthy
```

“Copy Diagnostic” 中加入这个 journal。

这对解决偶发 reconnect 问题非常关键。

---

# 82. State Transition Audit

所有 transition 必须经过同一个函数：

```rust
runtime.transition(next_state, cause)
```

严禁：

```rust
runtime.state = ...
```

散落在不同模块。

Transition 自动记录：

- old；
- new；
- time；
- cause；
- generation。

---

# 83. Runtime State Diagram

```text
                         ┌────────────┐
                         │  Stopped   │
                         └─────┬──────┘
                               │ Start
                               ▼
                         ┌────────────┐
                         │  Starting  │
                         └─────┬──────┘
                               │
                               ▼
                         ┌────────────┐
                         │ Connecting │◄───────────────┐
                         └─────┬──────┘                │
                               │                       │
                               ▼                       │
                       ┌────────────────┐              │
                       │ Authenticating │              │
                       └───────┬────────┘              │
                               │                       │
                               ▼                       │
                    ┌──────────────────────┐           │
                    │ EstablishingForward  │           │
                    └──────────┬───────────┘           │
                               │                       │
                               ▼                       │
                         ┌────────────┐                │
                    ┌───►│  Healthy   │                │
                    │    └──────┬─────┘                │
                    │           │ health issue          │
                    │           ▼                       │
                    │    ┌────────────┐                 │
                    │    │  Degraded  │                 │
                    │    └──────┬─────┘                 │
                    │           │ threshold exceeded    │
                    │           ▼                       │
                    │    ┌──────────────┐               │
                    └────│ Reconnecting │───────────────┘
                         └──────────────┘
                               │
                      permanent/config error
                               ▼
                         ┌────────────┐
                         │  Blocked   │
                         └────────────┘

Any active state
      │ Stop
      ▼
┌────────────┐
│  Stopping  │
└─────┬──────┘
      ▼
┌────────────┐
│  Stopped   │
└────────────┘
```

---

# 84. 必须避免的状态机错误

## 错误 A

```text
SSH socket exists → Healthy
```

禁止。

---

## 错误 B

```text
reconnect task 自己修改 UI
```

禁止。

---

## 错误 C

Stop 只更新 UI：

```text
status = stopped
```

但后台 task 仍运行。

禁止。

---

## 错误 D

配置更新后旧 task 可以提交结果。

禁止。

---

## 错误 E

发生 auth failure 后每秒重试密码。

禁止。

---

## 错误 F

listener bind 成功后立即显示 Connected。

禁止。

---

# 85. Product Copy Guidelines

文案保持极简。

推荐：

```text
Healthy
Reconnecting
Blocked
Stopped
Starting
```

不要：

```text
Amazing!
Oops!
Something went wrong!
```

错误必须具体。

差：

```text
Connection failed.
```

好：

```text
Authentication rejected by gpu.example.com:22.
Public key ~/.ssh/id_ed25519 was not accepted.
```

---

# 86. Keyboard Shortcuts

建议：

```text
Ctrl+1 Overview
Ctrl+2 Jumpers
Ctrl+3 Tunnels
Ctrl+4 Logs

Ctrl+, Settings

Ctrl+K Command palette / quick navigation

Ctrl+F Search current page

Ctrl+R Retry selected tunnel

Ctrl+Shift+R Restart selected tunnel
```

Command Palette 只用于本应用 action，不扩张为复杂插件系统。

---

# 87. Command Palette

支持：

```text
Start Tunnel: Lab SOCKS
Stop Tunnel: Lab SOCKS
Restart Tunnel: Lab SOCKS
Open Logs
New Tunnel
New Jumper
Check for Updates
```

GPUI Kit 已适合做此类 desktop action surface。

---

# 88. Accessibility

必须：

- focus ring；
- keyboard navigation；
- meaningful accessible names；
- state not color-only；
- modal focus trap；
- Escape close dialog；
- destructive action明确。

---

# 89. Internationalization

v1：

- English；
- 简体中文。

字符串不得散落硬编码。

目录：

```text
locales/
├── en.toml
└── zh-CN.toml
```

Runtime error technical fields保持原始英文 code，但 UI message 本地化。

---

# 90. No Feature Creep

以下明确不做：

- Terminal；
- SFTP；
- RDP；
- VNC；
- Docker；
- server monitoring；
- GPU monitoring；
- AI Agent；
- MCP；
- cloud sync；
- plugin system；
- team collaboration；
- mobile client。

因为 TunnelWarden 的核心价值就是：

> **SSH Tunnel Supervisor。**

---

# 91. README 产品承诺

README 第一屏应该非常简单：

```text
TunnelWarden

A native Rust GUI that keeps SSH tunnels alive.

• Local, Remote and SOCKS5 forwarding
• Multi-hop Jump Hosts
• Automatic reconnect
• Real SSH health checks
• Live traffic and latency
• No accounts
• No telemetry
• No Pro tier
• Apache-2.0
```

---

# 92. Definition of Done

一个 feature 只有同时满足以下条件才算完成：

1. Domain model；
2. Core implementation；
3. Structured errors；
4. cancellation；
5. generation safety；
6. logs；
7. UI；
8. unit test；
9. integration test；
10. error-path test；
11. documentation。

“Happy path 能跑”不算 Done。

---

# 93. 最终工程目标

TunnelWarden 最终必须做到：

```text
User clicks Start
↓
TunnelWarden owns the port
↓
TunnelWarden owns the SSH session
↓
TunnelWarden knows whether that session is truly alive
↓
Network fails
↓
TunnelWarden knows why
↓
TunnelWarden reports it honestly
↓
TunnelWarden reconnects deterministically
↓
Traffic resumes
```

而不是：

```text
there is still a process
↓
therefore probably connected
```

---

# 94. 最关键的成功标准

对于最初催生本项目的场景：

```text
Dynamic SOCKS5
127.0.0.1:1080
↓
SSH
↓
Lab Server
↓
Internet
```

TunnelWarden 必须保证：

### 正常

```text
Healthy · 36 ms
127.0.0.1:1080
```

### 服务器断开

```text
Reconnecting
Listener reserved: 127.0.0.1:1080
SSH unavailable
Retrying in 4.8s
```

### 恢复

```text
Healthy · 41 ms
```

### 用户 Stop

```text
Stopped
```

此刻：

```text
bind(127.0.0.1:1080)
```

必须立即能够被其他进程成功执行。

这就是 TunnelWarden 是否成功的最终判据。

---

# 95. Technical References

实现时优先阅读官方/一手资料：

- GPUI Kit  
  https://github.com/longbridge/gpui-kit  
  https://gpui-kit.com

- `russh`  
  https://github.com/Eugeny/russh  
  https://docs.rs/russh/latest/russh/

- `russh::client::connect_stream`  
  https://docs.rs/russh/latest/russh/client/fn.connect_stream.html

- `russh::client::Handle::channel_open_direct_tcpip`  
  https://docs.rs/russh/latest/russh/client/struct.Handle.html

- `russh::client::Config`  
  https://docs.rs/russh/latest/russh/client/struct.Config.html

- `ssh2-config`  
  https://docs.rs/ssh2-config/latest/ssh2_config/

- `tray-icon`  
  https://docs.rs/tray-icon/latest/tray_icon/

- Loris Tunnel（仅作为产品功能与交互参考）  
  https://github.com/RangerWolf/loris-tunnel-app

---

# 96. 给 Coding Agent 的最终约束

执行本 SPEC 时：

> **不要自行缩减范围。**

尤其不得把以下任何项目推迟到“future”：

- Local Forward；
- Remote Forward；
- Dynamic SOCKS5；
- Jump Host；
- Multi-hop；
- `ssh_config` import；
- traffic monitor；
- latency；
- logs；
- tray；
- auto run；
- auto start；
- import/export；
- connection test；
- deterministic diagnostics；
- state-machine hardening；
- reconnect stress tests。

可以分 PR、分工作包实现，但 **v1.0 的定义就是本文档的完整实现。**

同时：

> **不要为了快速完成 GUI 而弱化 Tunnel Runtime。**

本项目最值钱的部分不是 Zed 风格界面，也不是 `russh` 能连上服务器，而是：

> **一个具有严格资源所有权、可取消任务树、generation 防陈旧结果、错误分类、健康检查和可验证 Stop 语义的 SSH Tunnel Supervisor。**

这部分必须优先保证正确性。
