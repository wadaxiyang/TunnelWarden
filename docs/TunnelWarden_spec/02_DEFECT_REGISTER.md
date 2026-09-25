# 02 · 缺陷、风险与功能缺口登记册

基线 `71bf07c4c2185e8901166e2fbb2ccb8894effc78`。这里的 **P0/P1/P2 是整改和发布优先级，不是 CVSS，也不是已确认可被远程利用的漏洞评级**。

P0 先修，避免配置/凭据不一致。P1 涉及控制、信任、连接正确性或核心功能。P2 仍属于本轮完整交付，不能以优先级较低为由从 v1 删除。

本册共 26 项。D=14，A=1，G=7，R=4。D 的含义是代码路径足以确定问题，不代表本次已经执行了复现。

| 证据级别 | 含义 |
|---|---|
| D | 确定的代码路径问题（本次未动态复现） |
| A | 确定的架构偏差 |
| G | 确定的功能/接线缺口 |
| R | 条件风险，影响程度需运行验证 |

## 索引

| 编号 | 优先级 | 证据 | 事项 |
|---|---|---|---|
| [TW-001](#tw-001) | P0 | D | 配置已经提交，却按失败回滚新凭据 |
| [TW-002](#tw-002) | P1 | D | 固定临时文件会在崩溃后阻止再次保存 |
| [TW-003](#tw-003) | P1 | D | 保存无关配置会重新启动用户手动停止的隧道 |
| [TW-004](#tw-004) | P1 | D | 管理器等待 I/O 与逐个 Stop，导致控制面排队 |
| [TW-005](#tw-005) | P1 | A | 正式状态机没有成为完整 SSH 生命周期的唯一真相 |
| [TW-006](#tw-006) | P1 | D | Blocked 状态的托盘重试无效，界面缺少直接停止入口 |
| [TW-007](#tw-007) | P1 | D | IPv6 本地监听通过校验却无法启动 |
| [TW-008](#tw-008) | P1 | D | 外部配置的危险设置缺少分项授权 |
| [TW-009](#tw-009) | P1 | D | SOCKS 对外暴露警告只检查一个字符串 |
| [TW-010](#tw-010) | P1 | D | channel-open 和健康探测在 worker 主循环中串行等待 |
| [TW-011](#tw-011) | P1 | R | 停止时限不一致，退出仍可能卡在同步 join |
| [TW-012](#tw-012) | P1 | D | 主机密钥确认把整个握手窗口扩展到至少 120 秒 |
| [TW-013](#tw-013) | P1 | G | 界面允许选择 Keyboard Interactive，但核心确定不支持 |
| [TW-014](#tw-014) | P1 | G | 流量计数没有接到应用，且 worker 重建会换计数器 |
| [TW-015](#tw-015) | P2 | G | 主题配置没有形成实际产品主题与设置入口 |
| [TW-016](#tw-016) | P1 | D | OpenSSH 导入有语义偏差与静默近似 |
| [TW-017](#tw-017) | P2 | G | 基础管理工作流缺少删除、复制、搜索和保存前测试 |
| [TW-018](#tw-018) | P1 | D | 保存回调会关闭后来打开的编辑器，导航也会丢弃草稿 |
| [TW-019](#tw-019) | P1 | D | 校验生成了规范化配置，却继续运行原始配置 |
| [TW-020](#tw-020) | P2 | D | 日志只记录状态大类，丢失诊断上下文与中间事件 |
| [TW-021](#tw-021) | P1 | R | 局部上限不能构成全局资源预算，阻塞工作也需限流 |
| [TW-022](#tw-022) | P1 | R | 固定 russh 版本的 ping 可能把回执取消当成功 |
| [TW-023](#tw-023) | P2 | G | 信任提示缺少隧道/跳数/代号，临时信任作用域不清楚 |
| [TW-024](#tw-024) | P2 | G | 凭据编辑没有完整的清除/保留/轮换和回收语义 |
| [TW-025](#tw-025) | P2 | G | 原 v1 的 inactivity timeout 没有明确配置与运行语义 |
| [TW-026](#tw-026) | P2 | R | 手动 connect_stream 路径的 TCP_NODELAY 需要实证核对 |


<a id="tw-001"></a>
## TW-001 · 配置已经提交，却按失败回滚新凭据

**P0 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S14](13_SOURCE_INDEX.md#s14)、[S03](13_SOURCE_INDEX.md#s03)。ConfigStore::save → rename → prune_backups；TunnelManager::persist_config 的 Err 分支。

**触发条件**　保存更换密码/私钥口令的配置；主配置 rename 已成功，但删除旧备份失败。save 随后返回 Err，上层仍删除刚保存的新 SecretRef，并尝试回滚登录启动设置。

**影响与边界**　磁盘中的新配置可能引用已被删除的凭据，内存仍认为旧配置有效。普通“保存失败”提示掩盖了已经发生的提交，用户无法据提示安全重试。

**整改要求**　引入带提交状态的 SaveReceipt。提交点前失败才能回滚新凭据；提交后备份清理/目录持久化异常返回 CommittedWithWarnings。若无法确认提交结果，先重读并对账，绝不能无条件删除 SecretRef。

**必须增加的回归**　注入备份清理 AccessDenied；确认 save 返回 committed warning、新配置与新凭据均可读，内存采用同一 revision；另测真正的提交前失败仍清理新凭据。

**验收关联**　T001；见 05 的事务状态表。

---

<a id="tw-002"></a>
## TW-002 · 固定临时文件会在崩溃后阻止再次保存

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S14](13_SOURCE_INDEX.md#s14)。ConfigStore::save 的 config.toml.tmp / create_new 与 TempCleanup。

**触发条件**　进程在临时文件已创建而清理尚未执行时被终止；下一次启动正常读取 config.toml，下一次保存再次 create_new 同名 tmp。

**影响与边界**　即使主配置完全正常，后续保存也可能持续失败，直到人工删除残留文件。Drop 清理不能处理进程终止。

**整改要求**　同目录使用带事务 ID 的唯一临时文件；加配置写锁和启动恢复流程。只清理确认属于本程序、未被活跃事务使用的孤立临时文件，不删除任意 .tmp，也不直接信任残留 tmp。

**必须增加的回归**　在 write/sync/rename 前后分别终止子测试进程，重启后主配置为旧版或完整新版，下一次保存可成功。

**验收关联**　T002；恢复路径必须覆盖损坏文件与正常主配置两种情况。

---

<a id="tw-003"></a>
## TW-003 · 保存无关配置会重新启动用户手动停止的隧道

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S03](13_SOURCE_INDEX.md#s03)、[S01](13_SOURCE_INDEX.md#s01)。TunnelManager::replace_config 的 auto_start || old_running.contains(id)。

**触发条件**　隧道 A 设置 auto_start=true 并正在运行；用户 Stop A；随后仅修改分组名称、其他主机备注或应用设置。

**影响与边界**　replace_config 再次把 A 放入启动集合。用户本次运行意图被启动偏好覆盖，可能重新占端口并发起用户并未要求的连接。

**整改要求**　auto_start 只在应用启动初始化时转换为运行意图。每个 TunnelId 维护独立 DesiredState；配置应用保留当前意图，新建/导入的立即运行必须由独立 run_now 决策授权。

**必须增加的回归**　A 手动停止后连续进行五类无关保存，A 始终 Stopped 且端口可被测试监听器绑定；A 在下一次真正应用启动时仍按 auto_start 启动。

**验收关联**　T003；不能只把条件改成 old_running，它也不是完整的 DesiredState。

---

<a id="tw-004"></a>
## TW-004 · 管理器等待 I/O 与逐个 Stop，导致控制面排队

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S03](13_SOURCE_INDEX.md#s03)、[S20](13_SOURCE_INDEX.md#s20)。TunnelManager::run 内部的 stop().await、persist_config().await、spawn_blocking(...).await。

**触发条件**　凭据库/磁盘操作慢，或对多个慢退出隧道执行 StopAll；期间再发 Stop、处理 host-key prompt、读取状态变化。

**影响与边界**　Tokio 线程可能仍在运行网络任务，但管理器这个控制 actor 被占用，无法及时处理后续命令和快照。StopAll 的等待还可随隧道数量串行累加。

**整改要求**　把耗时工作变成由 owner 持有的 job，管理器只接收带 operation/revision 的完成事件；StopAll 先同时取消，再用共同 deadline 回收。为退出/停止保留高优先级路径，并对命令返回 Accepted/Busy/Rejected。

**必须增加的回归**　注入永不立即返回的配置 worker，另一隧道的 Stop 仍被确认；大量 stop 用同一时限，不出现 N×单隧道时限；状态事件不被持续命令洪水饿死。

**验收关联**　T004；见 03、05。

---

<a id="tw-005"></a>
## TW-005 · 正式状态机没有成为完整 SSH 生命周期的唯一真相

**P1 · 确定的架构偏差**

**证据与定位**　[S04](13_SOURCE_INDEX.md#s04)、[S05](13_SOURCE_INDEX.md#s05)、[S06](13_SOURCE_INDEX.md#s06)、[S08](13_SOURCE_INDEX.md#s08)、[S03](13_SOURCE_INDEX.md#s03)。StateMachine/Lifecycle 与 SupervisorState 并行；ListenerRuntime 只提交监听阶段事件。

**触发条件**　读取实际生产路径：listener 使用带 desired/generation 的状态机，SSH 连接、健康、重连则通过另一套 SupervisorState 发布。

**影响与边界**　形式模型中的不变量并未完整约束实际 SSH 工作路径；UI 又拿不到 desired、generation、稳定的 listener/remote binding。不能据此断言已经发生“旧任务复活”，但这是必须修正的架构偏差。

**整改要求**　保留一个权威 transition/reducer，将真实 worker 改为带 generation/attempt/owner 的事件和资源执行器。统一快照，删除第二份可独立决定健康的公开状态，而不是删除原有状态维度。

**必须增加的回归**　对真实连接接线测试：每次 Healthy 必须能追溯到当前代的认证、forward readiness、ping 与 live owner；旧代 Ready/Health 事件只能释放资源不能改新状态。

**验收关联**　T005；见 03 的迁移方案，禁止一次性重写所有数据面。

---

<a id="tw-006"></a>
## TW-006 · Blocked 状态的托盘重试无效，界面缺少直接停止入口

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S19](13_SOURCE_INDEX.md#s19)、[S21](13_SOURCE_INDEX.md#s21)、[S17](13_SOURCE_INDEX.md#s17)、[S03](13_SOURCE_INDEX.md#s03)。Tray Toggle 把 Blocked 视为未运行并发送 StartTunnel；manager.start 在 active 已存在时直接返回。

**触发条件**　让实际运行的隧道因认证/host-key 问题进入 Blocked；点击托盘该隧道。其本地 listener 还可能被保留。

**影响与边界**　用户看到点击没有效果；主列表此时只有 Retry，也不能在该行直接 Stop 释放端口。分组 StopAll 的存在不能替代单隧道操作。

**整改要求**　托盘、列表、详情统一调用状态动作解析器。Blocked 可 Retry，也必须可 Stop；显式区分 active owner 与 healthy。对 try_send 失败给出可见反馈，不静默丢命令。

**必须增加的回归**　有 listener 的 Blocked 与无 listener 的 Blocked 各自测试；托盘 Retry 真正进入新 attempt；Stop 最终释放端口。

**验收关联**　T006；统一 action matrix，见 07。

---

<a id="tw-007"></a>
## TW-007 · IPv6 本地监听通过校验却无法启动

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S24](13_SOURCE_INDEX.md#s24)、[S03](13_SOURCE_INDEX.md#s03)。TunnelConfig::validate 接受 IpAddr；manager.start 用 format!("{}:{}", host, port).parse::<SocketAddr>()。

**触发条件**　Local 或 Dynamic 的 local.host 设置为 ::1，端口为 1080。

**影响与边界**　IpAddr 校验成功，但 ::1:1080 不是期望的带方括号 socket address 表示。改写成 [::1] 又不能通过当前 IpAddr 校验。

**整改要求**　在域转换时保存或解析 IpAddr，运行时用 SocketAddr::new(ip, port)；序列化保留裸 IP，显示层才添加方括号。同步测试远端端点与 IPv4-mapped IPv6 的规范化。

**必须增加的回归**　在 IPv6 可用主机上真实绑定 ::1，完成 SOCKS/Local 数据交换并 Stop；缺少 IPv6 的 CI 显式 skip 并记录原因，不伪称通过。

**验收关联**　T007。

---

<a id="tw-008"></a>
## TW-008 · 外部配置的危险设置缺少分项授权

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S17](13_SOURCE_INDEX.md#s17)、[S13](13_SOURCE_INDEX.md#s13)、[S03](13_SOURCE_INDEX.md#s03)、[S16](13_SOURCE_INDEX.md#s16)。ImportConfig → import_preview → confirm_import → persist_config → replace_config。

**触发条件**　导入配置包含 Bypass、非回环监听、auto_start、登录启动设置，以及在本机实际存在的 SecretRef 或本地私钥路径；现预览只提供主机摘要与隧道数量。

**影响与边界**　一次“替换配置”确认混合了导入数据、复用本机凭据、放宽安全策略和启动网络连接。前提是用户导入并确认该文件，不是无需交互的远程攻击；新机器也不能凭空恢复不存在的凭据。

**整改要求**　建立 ImportPlan。默认不启动、不修改系统启动项、Strict、凭据待映射；分别列出网络暴露、信任降级、凭据关联和立即运行。受信本机备份恢复可显式选择恢复策略，但仍展示差异。

**必须增加的回归**　危险导入夹具在用户分项同意之前不能连接、不能绑定外网、不能读取旧凭据、不能写 Run key；安全导入仍保留定义且提示待配置项。

**验收关联**　T008；见 04、05。

---

<a id="tw-009"></a>
## TW-009 · SOCKS 对外暴露警告只检查一个字符串

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S17](13_SOURCE_INDEX.md#s17)、[S25](13_SOURCE_INDEX.md#s25)、[S24](13_SOURCE_INDEX.md#s24)。render_editor 只在 Dynamic 且 local_host == "0.0.0.0" 时显示提示。

**触发条件**　监听具体 LAN IPv4 地址；或修复 IPv6 后监听 :: / 具体非回环 IPv6；也需要检查 Local 与 Remote 的暴露语义。

**影响与边界**　当前 SOCKS 为 NO AUTH。默认回环是安全基线，但用户改为可达的非回环地址后，其他设备可能使用该代理而没有充分提醒。没有证据表明默认配置已经对公网开放。

**整改要求**　按解析后的地址分类做核心 preflight，不靠 UI 字符串判断。所有非回环配置均标识范围；Dynamic 非回环必须明确承认无认证风险，导入默认不授权。不得顺便自动开放系统防火墙。

**必须增加的回归**　覆盖 127.0.0.1、127.0.0.2、LAN IPv4、0.0.0.0、::1、:: 和具体 IPv6；确认只能由显式授权使暴露配置进入运行。

**验收关联**　T009。

---

<a id="tw-010"></a>
## TW-010 · channel-open 和健康探测在 worker 主循环中串行等待

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S07](13_SOURCE_INDEX.md#s07)、[S09](13_SOURCE_INDEX.md#s09)、[S11](13_SOURCE_INDEX.md#s11)。LocalForwardWorker::run 的 open_rx 分支、health_tick 分支；request_channel 缺少整体排队期限。

**触发条件**　一个目标长时间不响应 channel-open，其他新连接陆续到达；或多跳 ping 顺序等待。

**影响与边界**　已有 relay 任务不一定停顿，但新连接 admission、后续 open 请求和部分控制处理被阻塞。单次 5 秒超时不等于排队请求 5 秒内完成。队列有界，不是无限内存泄漏。

**整改要求**　用有界并发的 in-flight future 集合处理 channel-open 和 probe；限制每隧道及全局并发。deadline 从连接被接收/请求进入系统时建立并贯穿排队。停止时取消并回收所有 in-flight 请求。

**必须增加的回归**　慢目标与快 echo 目标并发；快目标不应被迫排在完整慢超时之后；排队超时有确定错误；原有流持续可用。

**验收关联**　T010；不能用一个大 Mutex 包住整个 SshChain。

---

<a id="tw-011"></a>
## TW-011 · 停止时限不一致，退出仍可能卡在同步 join

**P1 · 条件风险，影响程度需运行验证**

**证据与定位**　[S03](13_SOURCE_INDEX.md#s03)、[S08](13_SOURCE_INDEX.md#s08)、[S09](13_SOURCE_INDEX.md#s09)、[S10](13_SOURCE_INDEX.md#s10)、[S11](13_SOURCE_INDEX.md#s11)、[S20](13_SOURCE_INDEX.md#s20)、[E06](13_SOURCE_INDEX.md#e06)。manager 的 3 秒 deadline；Remote cancel 最多 5 秒及后续 drain；RuntimeHost::drop 的 thread.join。

**触发条件**　远端不回应取消转发、多跳关闭缓慢、凭据/磁盘 blocking job 卡住，同时关闭应用。

**影响与边界**　代码存在 deadline 不一致，实际 forced cleanup 频率与退出延迟尚未实测。不能直接断言已有孤儿端口：现有 Drop/cancellation 确实会关闭本进程资源。

**整改要求**　传递同一 StopContext(deadline, operation_id)；先停止 admission，尽力撤销远端注册，再取消传输并 join，超时显式强制关闭。退出采用有状态异步流程；blocking job 的未结束状态必须如实记录，不能以 abort 假装终止。

**必须增加的回归**　远端拒绝/不响应 cancel、三跳断线、keyring 慢调用、主窗口关闭后退出；记录强制清理次数并断言本地端口释放。

**验收关联**　T011；见 03 的本地资源与远端监听确认边界。

---

<a id="tw-012"></a>
## TW-012 · 主机密钥确认把整个握手窗口扩展到至少 120 秒

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S10](13_SOURCE_INDEX.md#s10)、[S12](13_SOURCE_INDEX.md#s12)。finish_connect 的 handshake deadline 在存在 approval provider 时使用更长时限。

**触发条件**　应用提供 host-key approval，但对端停在 banner/KEX 阶段，尚未产生任何需要用户确认的 prompt。

**影响与边界**　用户设定的较短连接超时与实际等待不一致；“给用户读指纹的时间”影响了不该等待人的网络阶段。

**整改要求**　将 DNS/TCP/KEX、实际等待用户决策、认证分成独立阶段预算；只有真正进入 AwaitingHostKey 时才暂停/切换到交互预算。Stop 对所有阶段立即可见。

**必须增加的回归**　8 秒网络预算下，对端不发 banner 或拖延 KEX 不得等 120 秒；正常 prompt 可等待约定的人机交互时限；Cancel 与 Stop 都中止握手。

**验收关联**　T012。

---

<a id="tw-013"></a>
## TW-013 · 界面允许选择 Keyboard Interactive，但核心确定不支持

**P1 · 确定的功能/接线缺口**

**证据与定位**　[S17](13_SOURCE_INDEX.md#s17)、[S18](13_SOURCE_INDEX.md#s18)、[S03](13_SOURCE_INDEX.md#s03)、[S01](13_SOURCE_INDEX.md#s01)。认证编辑按钮与 build_hops 的 KeyboardInteractive 分支。

**触发条件**　保存使用 Keyboard Interactive 的主机并启动隧道。

**影响与边界**　配置保存看似成功，实际认证直接失败。这不是 SSH 服务器的问题，而且该能力在原 v1 范围中。

**整改要求**　完成多轮 prompt-response 的 SSH 适配、窗口交互、取消/超时和脱敏。实现合入前先禁用并标明不可用；这只是内部过渡，不能在公开 v1 中永久隐藏了事。

**必须增加的回归**　多轮挑战、echo=false 输入、取消、过期响应、多跳上下文；日志和导出中不出现应答。

**验收关联**　T013；见 04、08。

---

<a id="tw-014"></a>
## TW-014 · 流量计数没有接到应用，且 worker 重建会换计数器

**P1 · 确定的功能/接线缺口**

**证据与定位**　[S26](13_SOURCE_INDEX.md#s26)、[S07](13_SOURCE_INDEX.md#s07)、[S09](13_SOURCE_INDEX.md#s09)、[S03](13_SOURCE_INDEX.md#s03)、[S13](13_SOURCE_INDEX.md#s13)、[S17](13_SOURCE_INDEX.md#s17)、[S01](13_SOURCE_INDEX.md#s01)。TrafficCounters 与 worker.counters；ManagerSnapshot/TunnelView；traffic_monitor 设置。

**触发条件**　持续使用隧道并重连，再查看界面及设置。

**影响与边界**　数据面虽然计数，但 UI 没有完整吞吐、累计、活跃连接和 120 点历史；每次 worker 重新创建的计数器也不适合直接当累计值。

**整改要求**　把稳定 counters 归属到 TunnelId 的统计所有者并跨 attempt 保留；1 Hz 聚合，120 点定长历史，UI 仅接收采样。明确累计范围与关闭监控行为。

**必须增加的回归**　已知字节数双向传输、half-close、重连、停止、监控关闭与重开；累计不倒退，采样按真实时间间隔计算。

**验收关联**　T014；见 08、09。

---

<a id="tw-015"></a>
## TW-015 · 主题配置没有形成实际产品主题与设置入口

**P2 · 确定的功能/接线缺口**

**证据与定位**　[S13](13_SOURCE_INDEX.md#s13)、[S17](13_SOURCE_INDEX.md#s17)、[S19](13_SOURCE_INDEX.md#s19)、[S29](13_SOURCE_INDEX.md#s29)。AppSettings.theme 与 main/workspace 生产路径。

**触发条件**　切换或导入 theme 偏好，检查窗口与控件。

**影响与边界**　当前主要依赖框架默认主题与通用组件；没有完整的应用级语义色、密度、层级和主题切换闭合路径。此处是源码结构审阅，不是本次截图测评。

**整改要求**　建立 theme.rs、metrics.rs 与少量应用组合组件，接入 System/Light/Dark；保存偏好不得重启隧道。按 06、07 重构，而不是单独改背景颜色。

**必须增加的回归**　浅深主题与系统切换、输入/选择/错误/禁用/焦点状态、图表和托盘可读性；主题变更不增加连接 attempt。

**验收关联**　T015。

---

<a id="tw-016"></a>
## TW-016 · OpenSSH 导入有语义偏差与静默近似

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S15](13_SOURCE_INDEX.md#s15)、[E05](13_SOURCE_INDEX.md#e05)、[S01](13_SOURCE_INDEX.md#s01)。read_lines 的 Include 展开与目录基准；标量解析；ServerAliveInterval 映射。

**触发条件**　导入 Include 相对路径、条件 Include、重复 Include、多个 IdentityFile、显式 ServerAliveInterval 0 或复杂带引号配置。

**影响与边界**　自写解析器并不等价 OpenSSH。特别是以当前文件父目录处理相对 Include，以及把 0 提升到 1 的保活映射，会改变配置含义。ProxyJump 已有警告，不能谎称它已被完整导入。

**整改要求**　建立携带来源位置的解析/求值适配器；修复已支持字段的语义。无法安全映射的 directive 保留可操作警告，并阻止相关条目以“等价可用”状态导入。支持 ProxyJump 的解析和链构造，但不执行 ProxyCommand/Match exec。

**必须增加的回归**　固定夹具验证 Include 路径基准、条件作用域、标量 first-wins、列表项、0 值与警告；测试用可信夹具可与 OpenSSH 比较，产品不能 shell out。

**验收关联**　T016；见 05。

---

<a id="tw-017"></a>
## TW-017 · 基础管理工作流缺少删除、复制、搜索和保存前测试

**P2 · 确定的功能/接线缺口**

**证据与定位**　[S17](13_SOURCE_INDEX.md#s17)、[S03](13_SOURCE_INDEX.md#s03)、[S01](13_SOURCE_INDEX.md#s01)、[E01](13_SOURCE_INDEX.md#e01)。render_hosts、render_tunnels、编辑器、CoreCommand。

**触发条件**　日常需要删除/复制某条隧道、搜索大量配置，或检查未保存草稿的连通性。

**影响与边界**　已有分组增删改和分页，但主机/隧道的完整 CRUD、快速定位、诊断测试尚未形成用户可用的功能集。不能把底层 integration test 当成界面的 Test Connection。

**整改要求**　补齐带依赖检查的删除、复制新 ID、列表搜索筛选、草稿隔离测试、批量操作与上下文菜单；所有危险操作明确影响的运行隧道。

**必须增加的回归**　被引用主机的删除不会留下悬空链；复制不继承立即启动；草稿测试不覆盖现有配置、不占用生产隧道 listener；筛选后选中 ID 稳定。

**验收关联**　T017；见 07、08。

---

<a id="tw-018"></a>
## TW-018 · 保存回调会关闭后来打开的编辑器，导航也会丢弃草稿

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S17](13_SOURCE_INDEX.md#s17)。persist_config 的异步回调无条件清空 editor/group_editor/import_preview；select_page 无 dirty guard。

**触发条件**　A 保存尚未返回时切换页面并打开 B 编辑器，随后 A 的回调成功；或编辑中直接点击侧栏。

**影响与边界**　B 可被旧回调关闭；未保存内容没有退出确认。单个 _save_task 被覆盖的风险还需要并发点击测试，不能靠按钮零散 disabled 保证正确性。

**整改要求**　引入 EditSessionId、WindowEpoch、SaveOperationId 与配置 revision；回调只更新属于自己的编辑会话，配置以核心发布的 canonical snapshot 为准。统一 dirty-navigation guard 和全入口保存协调。

**必须增加的回归**　A 慢保存→B 新编辑→A 完成；B 仍存在且内容不变。覆盖主窗口关闭重开、取消对话框、重复点击和保存失败。

**验收关联**　T018。

---

<a id="tw-019"></a>
## TW-019 · 校验生成了规范化配置，却继续运行原始配置

**P1 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S13](13_SOURCE_INDEX.md#s13)、[S17](13_SOURCE_INDEX.md#s17)、[S18](13_SOURCE_INDEX.md#s18)、[S03](13_SOURCE_INDEX.md#s03)、[S10](13_SOURCE_INDEX.md#s10)。into_domain/expand_home；UI 保存仅检查转换结果是否 Err；manager.replace_config 使用传入的 DomainConfig。

**触发条件**　在私钥路径框输入 ~/.ssh/id_ed25519，保存后立刻启动，之后退出并重新打开再启动。

**影响与边界**　校验路径会展开 ~，但其结果没有作为本次运行配置；原始 PathBuf 仍可能按字面 ~ 打开。磁盘重载又会执行展开，产生“保存后不能用、重开后能用”的不一致。

**整改要求**　集中返回 CanonicalConfig/ValidatedConfig，并让保存、应用、测试和重载使用同一结果。UI 不自行替换为旧草稿副本；需要保留路径原始写法时，区分显示路径与运行路径。

**必须增加的回归**　同一草稿在保存后运行、重载后运行、导入后测试得到相同实际 key path；还覆盖 Windows ~\ 路径与空格路径。

**验收关联**　T019。

---

<a id="tw-020"></a>
## TW-020 · 日志只记录状态大类，丢失诊断上下文与中间事件

**P2 · 确定的代码路径问题（本次未动态复现）**

**证据与定位**　[S03](13_SOURCE_INDEX.md#s03)、[S17](13_SOURCE_INDEX.md#s17)。refresh 比较 enum discriminant；LogEvent 的 stage/message 为静态字符串；UI 仅显示通用事件。

**触发条件**　同为 Reconnecting 的连续失败原因/attempt 改变；一个很快的 Connecting→Blocked 转换发生在快照采样之间。

**影响与边界**　用户可能只看到通用“检查状态”或漏掉阶段经过，难以诊断第几跳、哪个端点、哪一步失败。现有日志有 500 条上限，这不是无界日志问题。

**整改要求**　状态快照仍可合并；另外从真实事件生成有序、结构化、脱敏的有界 journal，保留 tunnel/host/hop/attempt/generation/stage/error code。UI 增加隧道过滤、复制和本地导出。

**必须增加的回归**　同状态不同错误、短暂中间状态、环形覆盖、订阅落后、敏感输入和过长远端文本；重要事件可追溯且不泄密。

**验收关联**　T020。

---

<a id="tw-021"></a>
## TW-021 · 局部上限不能构成全局资源预算，阻塞工作也需限流

**P1 · 条件风险，影响程度需运行验证**

**证据与定位**　[S03](13_SOURCE_INDEX.md#s03)、[S07](13_SOURCE_INDEX.md#s07)、[S09](13_SOURCE_INDEX.md#s09)、[S10](13_SOURCE_INDEX.md#s10)、[S12](13_SOURCE_INDEX.md#s12)、[S13](13_SOURCE_INDEX.md#s13)、[S20](13_SOURCE_INDEX.md#s20)、[S26](13_SOURCE_INDEX.md#s26)、[E06](13_SOURCE_INDEX.md#e06)。1024 条配置、每隧道 64 连接、最多 16 跳；host-key 同步文件操作、私钥同步 decode 与 blocking credential load。

**触发条件**　大量隧道同时自启动或同时掉线，或多份高成本私钥同时解码；实际高水位取决于网络、认证方式和框架。

**影响与边界**　所有容器即使各自有界，聚合仍可能很大。现有证据不能证明 RAM 泄漏，但足以要求全局 admission、线程/任务/缓冲预算和过载可见性。

**整改要求**　为连接链、SSH session、active relays、pending opens、昂贵解码分别建立有限预算。跨多跳一次申请整条链的 session 配额，避免部分占有后互相等待。阻塞工作移出 async 热路径但继续限量、持有 handle。

**必须增加的回归**　启动/重连风暴、512 主机/1024 配置、配额耗尽/释放、公平性和取消；资源高水位不超过配置预算，队列不无限增长。

**验收关联**　T021；具体初始建议见 09，属于待校准预算。

---

<a id="tw-022"></a>
## TW-022 · 固定 russh 版本的 ping 可能把回执取消当成功

**P1 · 条件风险，影响程度需运行验证**

**证据与定位**　[S10](13_SOURCE_INDEX.md#s10)、[S06](13_SOURCE_INDEX.md#s06)、[S07](13_SOURCE_INDEX.md#s07)、[E10](13_SOURCE_INDEX.md#e10)。russh v0.63.3 Handle::send_ping 的 let _ = receiver.await; Ok(())。

**触发条件**　ping 已提交到 session，随后回执 sender 因 session 异常被丢弃；产品适配层将 Ok 转成 RTT。

**影响与边界**　上游源码确定忽略该错误，但具体竞争窗口的产品表现尚未动态复现。不能把所有现有 Healthy 都说成假健康；需要防止一次取消回执被当作成功样本。

**整改要求**　添加断线/取消回执回归；适配层检查代号、取消状态和 transport closed；若固定版本存在可复现误判，优先最小上游修复/受控补丁，正确传播 receiver 错误。仅 pre/post is_closed 检查不等于证明真正收到 pong。

**必须增加的回归**　server 在 ping 与回执之间断开；不得发布成功 RTT 或当前代 Healthy。合法 reply、合法 negative global reply 与真正断线分别测试。

**验收关联**　T022；升级必须带固定版本与同一回归，禁止盲升 main。

---

<a id="tw-023"></a>
## TW-023 · 信任提示缺少隧道/跳数/代号，临时信任作用域不清楚

**P2 · 确定的功能/接线缺口**

**证据与定位**　[S12](13_SOURCE_INDEX.md#s12)、[S03](13_SOURCE_INDEX.md#s03)、[S17](13_SOURCE_INDEX.md#s17)。HostKeyPrompt/View 只有 host、port、algorithm、fingerprint、id；TrustOnce 仅对本次验证返回 true。

**触发条件**　多个隧道同时要求确认，或已选择 Trust Once 的链随后重连；窗口关闭、Stop、编辑主机后旧提示仍可能在队列中。

**影响与边界**　用户难以判断正在批准哪一跳。“Once”究竟是一次握手还是本次运行未说明；当前代码不能直接解释为整个进程会话信任。

**整改要求**　给提示绑定 TunnelId、HostId、hop、generation、attempt、expiry，并由 reducer 使其过期。明确采用“本次隧道运行期间”或“一次连接”语义；本方案推荐前者且不覆盖任何已知 changed/revoked 判定。

**必须增加的回归**　并发同主机提示、不同端口/不同 fingerprint、Stop 后批准、窗口重开、保存安全策略变更；过期审批永远无效。

**验收关联**　T023；临时授权不得落入导出配置。

---

<a id="tw-024"></a>
## TW-024 · 凭据编辑没有完整的清除/保留/轮换和回收语义

**P2 · 确定的功能/接线缺口**

**证据与定位**　[S18](13_SOURCE_INDEX.md#s18)、[S16](13_SOURCE_INDEX.md#s16)、[S14](13_SOURCE_INDEX.md#s14)、[S03](13_SOURCE_INDEX.md#s03)。留空保留旧密码/口令；新密码分配新 SecretRef；备份继续引用旧配置。

**触发条件**　把加密私钥换成无口令私钥、清除已保存口令、频繁改密码、恢复较早配置备份。

**影响与边界**　留空只能保留，不能明确表达清除。旧引用的生命周期也没有可审计的回收规则。直接“每次保存删旧凭据”反而会破坏备份恢复。

**整改要求**　设计 Keep/Replace/Remove 三态；清除与密码空字符串不是同义词。回收时计算当前配置、保留备份、活跃事务及正在清理的 runtime 所引用集合；仅删除确认无引用的本程序条目。

**必须增加的回归**　更换/清除/保留、提交前后失败、恢复旧备份、并发慢连接；日志无密码，回收不删仍被引用的凭据。

**验收关联**　T024；UI 输入与第三方库不承诺全链路所有副本已零化。

---

<a id="tw-025"></a>
## TW-025 · 原 v1 的 inactivity timeout 没有明确配置与运行语义

**P2 · 确定的功能/接线缺口**

**证据与定位**　[S01](13_SOURCE_INDEX.md#s01)、[S13](13_SOURCE_INDEX.md#s13)、[S10](13_SOURCE_INDEX.md#s10)、[S17](13_SOURCE_INDEX.md#s17)。原规格 SSH Connection；当前 HostRecord/编辑器/client 配置未建立独立 inactivity 设置。

**触发条件**　用户需要区分建立连接超时、keepalive、健康探测和真正的空闲关闭行为。

**影响与边界**　目前既不能配置，也容易在补功能时混成一个“超时”，从而让长期空闲但健康的隧道被误停。

**整改要求**　明确 inactivity 的层级与默认值；默认不因业务空闲关闭长期隧道。将 SSH transport inactivity 与可选单条转发连接 idle policy 分开，v1 至少补齐原规格要求并说明与 ping 的关系。

**必须增加的回归**　长时间无业务但有健康回执不误停；真正传输无响应按约定进入退避；用户设定 idle close 只影响相应层级。

**验收关联**　T025。

---

<a id="tw-026"></a>
## TW-026 · 手动 connect_stream 路径的 TCP_NODELAY 需要实证核对

**P2 · 条件风险，影响程度需运行验证**

**证据与定位**　[S10](13_SOURCE_INDEX.md#s10)、[E10](13_SOURCE_INDEX.md#e10)。产品先 TcpStream::connect 再 client::connect_stream；config.nodelay=true；上游 connect 才显式 set_nodelay。

**触发条件**　短请求/交互型业务在存在小包和延迟确认的网络上运行。

**影响与边界**　本次已读的 TCP 建立路径没有看到实际 set_nodelay；不能根据 config 字段就认定 socket 选项生效。性能影响尚未测量，也不能把它当成当前内存占用的原因。

**整改要求**　在仍持有真实 TcpStream 时设置/读取 nodelay，并单测；多跳 ChannelStream 不伪装成 OS socket。Local/Remote 的接入 TCP 端是否设置，依据延迟与吞吐对照决定。

**必须增加的回归**　真实 socket 选项查询、小包 RTT 与大块吞吐 A/B；设置失败作为结构化错误/警告处理，不引入 busy loop。

**验收关联**　T026。
