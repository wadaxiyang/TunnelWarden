# 13 · 证据索引与复查入口

审计日期为 **2026-09-25**。TunnelWarden 的全部源码链接固定到提交 `71bf07c4c2185e8901166e2fbb2ccb8894effc78`。外部文档为该日访问的内容，后续执行时要重新核对依赖版本。

文档中的 S 编号指用户项目源码，E 编号指外部一手资料。源文件的函数名是定位依据；没有编造未核验的行号。只要执行时 HEAD 不同，先检查相关文件 diff，不能机械套用结论。

## 读取范围说明

这是一轮针对控制流、数据流、UI 接线和资源边界的静态审阅。没有逐行审阅 Cargo.lock 的所有传递依赖、vendor 全部实现、全部集成测试或原规格所有后续章节。仓库测试记录不替代独立复测。

<a id="s01"></a>
## S01 · 产品原始规格

[TunnelWarden_SPEC.md](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/TunnelWarden_SPEC.md)

本次读取第 1–650 行，覆盖定位、v1 核心范围、认证、转发、跳板、导入、流量、技术栈。未把后续未读章节当作逐行审计证据。

<a id="s02"></a>
## S02 · 项目 Agent 约束

[AGENTS.md](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/AGENTS.md)

完整读取。生命周期、资源所有权、GPUI、内存、完整 v1 范围约束。

<a id="s03"></a>
## S03 · 管理器与配置应用

[crates/tunnel-core/src/manager.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/manager.rs)

完整分段读取。run、persist_config、replace_config、start、stop、refresh、build_hops、运行参数比较及文件内测试。

<a id="s04"></a>
## S04 · 形式状态机

[crates/tunnel-core/src/state_machine.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/state_machine.rs)

读取第 1–400 行。transition 及相关状态转换；没有声称完整覆盖该文件余下测试。

<a id="s05"></a>
## S05 · 本地监听器生命周期

[crates/tunnel-core/src/runtime.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/runtime.rs)

完整读取。ListenerRuntime 实际使用 StateMachine，但只驱动监听器阶段。

<a id="s06"></a>
## S06 · Local/Dynamic supervisor

[crates/tunnel-core/src/supervisor.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/supervisor.rs)

完整读取。连接、初始探测、退避、Blocked、取消与 listener 保留。

<a id="s07"></a>
## S07 · Local/Dynamic worker

[crates/tunnel-core/src/local_forward.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/local_forward.rs)

完整读取。accept、SOCKS、channel-open 队列、健康探测与连接任务清理。

<a id="s08"></a>
## S08 · Remote supervisor

[crates/tunnel-core/src/remote_supervisor.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/remote_supervisor.rs)

完整读取。独立 session token、注册、重连与退出。

<a id="s09"></a>
## S09 · Remote worker

[crates/tunnel-core/src/remote_forward.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-core/src/remote_forward.rs)

完整读取。远端注册、接收 forwarded channels、本地目标连接与停止。

<a id="s10"></a>
## S10 · SSH 客户端适配层

[crates/ssh-engine/src/client.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/ssh-engine/src/client.rs)

完整分段读取。connect_stream、认证、host-key approval、ping、agent、私钥读取和 Drop。

<a id="s11"></a>
## S11 · 多跳链

[crates/ssh-engine/src/chain.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/ssh-engine/src/chain.rs)

完整读取。多级 direct-tcpip、逐跳认证、ping_all 与逆序 disconnect。

<a id="s12"></a>
## S12 · 主机密钥与反向通道校验

[crates/ssh-engine/src/host_key.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/ssh-engine/src/host_key.rs)

完整读取。Strict、Bypass、unknown/changed、信任保存、forwarded-tcpip admission。

<a id="s13"></a>
## S13 · 配置 schema 与域转换

[crates/config-store/src/schema.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/config-store/src/schema.rs)

完整分段读取。v1 schema、AppSettings、数量限制、路径展开与双向转换。

<a id="s14"></a>
## S14 · 配置读写与备份

[crates/config-store/src/store.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/config-store/src/store.rs)

完整读取。save、tmp、rename、prune_backups、export 及文件内测试。

<a id="s15"></a>
## S15 · OpenSSH 导入器

[crates/config-store/src/ssh_import.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/config-store/src/ssh_import.rs)

完整读取。read_lines、Include、匹配、标量映射、预览警告。

<a id="s16"></a>
## S16 · 系统凭据存储

[crates/config-store/src/secrets.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/config-store/src/secrets.rs)

完整读取。服务名、引用校验、16 KiB 上限、读写删除。

<a id="s17"></a>
## S17 · 当前界面实现

[crates/app/src/workspace.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/workspace.rs)

分段读取第 1–2310 行，覆盖生产页面、编辑/保存流程及部分 UI 测试；未读取文件末尾全部测试。

<a id="s18"></a>
## S18 · 编辑器模型

[crates/app/src/editor.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/editor.rs)

完整读取。HostEditor、TunnelEditor、SecretUpdate 与随机 SecretRef。

<a id="s19"></a>
## S19 · 应用与窗口入口

[crates/app/src/main.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/main.rs)

完整读取。窗口、托盘动作、配置订阅、再次打开窗口。

<a id="s20"></a>
## S20 · 应用运行时所有者

[crates/app/src/runtime_host.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/runtime_host.rs)

完整读取。单一 Tokio runtime、2 个 worker、thread.join 与平台服务。

<a id="s21"></a>
## S21 · 托盘

[crates/app/src/tray.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/tray.rs)

完整读取。presentation 去重、25 条菜单、动作投递与状态映射。

<a id="s22"></a>
## S22 · 网络变更回调

[crates/app/src/network.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/network.rs)

完整读取。有限 try_send、系统注销与注销失败的安全保留分支。

<a id="s23"></a>
## S23 · Windows 单实例

[crates/app/src/instance.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/src/instance.rs)

完整读取。命名对象、唤醒、native handle 释放。未做 Windows ACL/跨会话实测。

<a id="s24"></a>
## S24 · 域级校验

[crates/tunnel-domain/src/validation.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/tunnel-domain/src/validation.rs)

完整读取。IPv4/IPv6、端点、跳板、分组、重连参数。

<a id="s25"></a>
## S25 · SOCKS5 解析器

[crates/forwarding/src/socks5.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/forwarding/src/socks5.rs)

完整读取。NO AUTH、CONNECT、地址类型、错误回复及文件内测试。

<a id="s26"></a>
## S26 · 双向 relay 与计数器

[crates/forwarding/src/relay.rs](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/forwarding/src/relay.rs)

完整读取。双方向 16 KiB 缓冲、half-close、atomic counters、RAII active count。

<a id="s27"></a>
## S27 · 仓库验证记录

[docs/verification.md](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/docs/verification.md)

完整读取。仅作为仓库记录引用，不是本次执行结果；没有获取其原始 CSV 或完整运行日志。

<a id="s28"></a>
## S28 · workspace 依赖固定

[Cargo.toml](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/Cargo.toml)

完整读取。russh =0.63.3、Tokio、Windows GPUI vendor patch。

<a id="s29"></a>
## S29 · App 依赖固定

[crates/app/Cargo.toml](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/crates/app/Cargo.toml)

完整读取。gpui-kit =0.6.6、Windows 依赖、test-support。

<a id="s30"></a>
## S30 · 当前 README

[README.md](https://github.com/wadaxiyang/TunnelWarden/blob/71bf07c4c2185e8901166e2fbb2ccb8894effc78/README.md)

完整读取。开发状态、已实现能力与尚未完成的发布门槛。

# 外部一手资料

<a id="e01"></a>
## E01 · Loris Tunnel 官方仓库 README

[打开来源](https://github.com/RangerWolf/loris-tunnel-app)

仅据公开 README 对照工作流：三种转发、多跳、复制、连接测试、SSH 命令导入、延迟显示等。不据此推断其内存、稳定性或每项功能无缺陷。

<a id="e02"></a>
## E02 · GPUI Kit 官方仓库

[打开来源](https://github.com/longbridge/gpui-kit)

组件能力与 facade 方向；工程以 TunnelWarden 锁定的 0.6.6 为准，不自动追 main。

<a id="e03"></a>
## E03 · GPUI Kit Table 文档

[打开来源](https://gpui-kit.com/component/table/)

虚拟化表格、选择、列与排序的能力参考。具体 API 需以固定版本示例编译验证。

<a id="e04"></a>
## E04 · GPUI Kit Theme 文档

[打开来源](https://gpui-kit.com/component/theme/)

主题语义角色与应用级主题接入参考。

<a id="e05"></a>
## E05 · OpenBSD/OpenSSH ssh_config 手册

[打开来源](https://man.openbsd.org/ssh_config)

重点核对 Include 的路径基准/条件作用域，以及 ServerAliveInterval=0。本文提出的安全导入策略是本项目设计，不冒充 OpenSSH 原生行为。

<a id="e06"></a>
## E06 · Tokio spawn_blocking 官方文档

[打开来源](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

已经开始的 blocking job 不能靠 abort 终止；shutdown_timeout 仅停止等待，不等于任务消失。检索页为 1.53.1，不以此替代本项目 Cargo.lock 的版本审计。

<a id="e07"></a>
## E07 · RFC 1928

[打开来源](https://www.rfc-editor.org/rfc/rfc1928.html)

SOCKS5 CONNECT、地址类型和回复语义；本项目明确不实现 BIND/UDP ASSOCIATE。

<a id="e08"></a>
## E08 · RFC 4254

[打开来源](https://www.rfc-editor.org/rfc/rfc4254.html)

SSH channel、EOF、direct/forwarded TCP/IP 与取消远端转发的协议依据。

<a id="e09"></a>
## E09 · Microsoft 进程内存采集文档

[打开来源](https://learn.microsoft.com/en-us/windows/win32/psapi/collecting-memory-usage-information-for-a-process)

进程内存采集入口；跨应用比较方案与验收预算是本项目建议，不是微软给出的性能承诺。

<a id="e10"></a>
## E10 · russh v0.63.3 Handle 实现

[打开来源](https://github.com/Eugeny/russh/blob/v0.63.3/russh/src/client/mod.rs)

通过 GitHub 读取第 780–1100 行，核对 send_ping、channel_open_direct_tcpip、tcpip_forward 与 connect。send_ping 丢弃 receiver.await 错误；该风险尚未在本次运行复现。

<a id="e11"></a>
## E11 · Zed 的 GPUI ownership 说明

[打开来源](https://zed.dev/blog/gpui-ownership)

用于理解实体所有权与 UI 更新边界，不作为 0.6.6 的可直接编译 API。
