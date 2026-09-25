# 10 · 回归测试、互通、长稳与发布门槛

## 1. 测试证据边界

本包没有执行任何 Windows/Cargo/GUI/长稳测试。仓库历史验证保留在 [S27](13_SOURCE_INDEX.md#s27)，不能在复制本规格时改成新的通过日期。

下面的测试函数名均是**建议新增或重构后的名字**，不是声称仓库已存在这些测试。执行时优先复用已有 SSH fixture、端口释放、多跳、SOCKS、UI interaction 测试，避免新起一套不验证产品接线的平行测试系统。

每项回归至少包含“修复前会失败的证据”与“修复后通过的证据”。有风险项未能在指定条件下复现时，应记录条件和排除范围，不能把所有风险都强行宣称修复成功。

## 2. 26 项回归索引

| 测试 | 登记项 | 建议测试名 | 关键断言 |
|---|---|---|---|
| T001 | [TW-001](02_DEFECT_REGISTER.md#tw-001) | `save_prune_failure_keeps_committed_secret` | 注入备份清理 AccessDenied；确认 save 返回 committed warning、新配置与新凭据均可读，内存采用同一 revision；另测真正的提交前失败仍清理新凭据。 |
| T002 | [TW-002](02_DEFECT_REGISTER.md#tw-002) | `crash_temp_does_not_block_next_save` | 在 write/sync/rename 前后分别终止子测试进程，重启后主配置为旧版或完整新版，下一次保存可成功。 |
| T003 | [TW-003](02_DEFECT_REGISTER.md#tw-003) | `manual_stop_survives_unrelated_save` | A 手动停止后连续进行五类无关保存，A 始终 Stopped 且端口可被测试监听器绑定；A 在下一次真正应用启动时仍按 auto_start 启动。 |
| T004 | [TW-004](02_DEFECT_REGISTER.md#tw-004) | `slow_config_job_does_not_block_other_stop` | 注入永不立即返回的配置 worker，另一隧道的 Stop 仍被确认；大量 stop 用同一时限，不出现 N×单隧道时限；状态事件不被持续命令洪水饿死。 |
| T005 | [TW-005](02_DEFECT_REGISTER.md#tw-005) | `only_current_owner_can_report_healthy` | 对真实连接接线测试：每次 Healthy 必须能追溯到当前代的认证、forward readiness、ping 与 live owner；旧代 Ready/Health 事件只能释放资源不能改新状态。 |
| T006 | [TW-006](02_DEFECT_REGISTER.md#tw-006) | `blocked_tray_retry_and_stop_are_effective` | 有 listener 的 Blocked 与无 listener 的 Blocked 各自测试；托盘 Retry 真正进入新 attempt；Stop 最终释放端口。 |
| T007 | [TW-007](02_DEFECT_REGISTER.md#tw-007) | `ipv6_loopback_bind_works_end_to_end` | 在 IPv6 可用主机上真实绑定 ::1，完成 SOCKS/Local 数据交换并 Stop；缺少 IPv6 的 CI 显式 skip 并记录原因，不伪称通过。 |
| T008 | [TW-008](02_DEFECT_REGISTER.md#tw-008) | `import_requires_explicit_security_approval` | 危险导入夹具在用户分项同意之前不能连接、不能绑定外网、不能读取旧凭据、不能写 Run key；安全导入仍保留定义且提示待配置项。 |
| T009 | [TW-009](02_DEFECT_REGISTER.md#tw-009) | `non_loopback_exposure_is_checked_in_core` | 覆盖 127.0.0.1、127.0.0.2、LAN IPv4、0.0.0.0、::1、:: 和具体 IPv6；确认只能由显式授权使暴露配置进入运行。 |
| T010 | [TW-010](02_DEFECT_REGISTER.md#tw-010) | `slow_channel_open_does_not_block_admission_or_health` | 慢目标与快 echo 目标并发；快目标不应被迫排在完整慢超时之后；排队超时有确定错误；原有流持续可用。 |
| T011 | [TW-011](02_DEFECT_REGISTER.md#tw-011) | `shutdown_obeys_shared_budget_and_tracks_blocking_jobs` | 远端拒绝/不响应 cancel、三跳断线、keyring 慢调用、主窗口关闭后退出；记录强制清理次数并断言本地端口释放。 |
| T012 | [TW-012](02_DEFECT_REGISTER.md#tw-012) | `handshake_timeout_is_not_prompt_wait_timeout` | 8 秒网络预算下，对端不发 banner 或拖延 KEX 不得等 120 秒；正常 prompt 可等待约定的人机交互时限；Cancel 与 Stop 都中止握手。 |
| T013 | [TW-013](02_DEFECT_REGISTER.md#tw-013) | `keyboard_interactive_multi_round_cancel_and_echo` | 多轮挑战、echo=false 输入、取消、过期响应、多跳上下文；日志和导出中不出现应答。 |
| T014 | [TW-014](02_DEFECT_REGISTER.md#tw-014) | `traffic_counters_survive_reconnect_and_toggle` | 已知字节数双向传输、half-close、重连、停止、监控关闭与重开；累计不倒退，采样按真实时间间隔计算。 |
| T015 | [TW-015](02_DEFECT_REGISTER.md#tw-015) | `theme_preference_is_applied_without_tunnel_restart` | 浅深主题与系统切换、输入/选择/错误/禁用/焦点状态、图表和托盘可读性；主题变更不增加连接 attempt。 |
| T016 | [TW-016](02_DEFECT_REGISTER.md#tw-016) | `openssh_import_preserves_supported_semantics` | 固定夹具验证 Include 路径基准、条件作用域、标量 first-wins、列表项、0 值与警告；测试用可信夹具可与 OpenSSH 比较，产品不能 shell out。 |
| T017 | [TW-017](02_DEFECT_REGISTER.md#tw-017) | `crud_search_duplicate_and_connection_test_are_wired` | 被引用主机的删除不会留下悬空链；复制不继承立即启动；草稿测试不覆盖现有配置、不占用生产隧道 listener；筛选后选中 ID 稳定。 |
| T018 | [TW-018](02_DEFECT_REGISTER.md#tw-018) | `stale_save_does_not_close_new_editor` | A 慢保存→B 新编辑→A 完成；B 仍存在且内容不变。覆盖主窗口关闭重开、取消对话框、重复点击和保存失败。 |
| T019 | [TW-019](02_DEFECT_REGISTER.md#tw-019) | `save_and_reload_use_same_canonical_key_path` | 同一草稿在保存后运行、重载后运行、导入后测试得到相同实际 key path；还覆盖 Windows ~\ 路径与空格路径。 |
| T020 | [TW-020](02_DEFECT_REGISTER.md#tw-020) | `error_events_keep_stage_and_attempt_details` | 同状态不同错误、短暂中间状态、环形覆盖、订阅落后、敏感输入和过长远端文本；重要事件可追溯且不泄密。 |
| T021 | [TW-021](02_DEFECT_REGISTER.md#tw-021) | `global_resource_budget_bounds_peak_admission` | 启动/重连风暴、512 主机/1024 配置、配额耗尽/释放、公平性和取消；资源高水位不超过配置预算，队列不无限增长。 |
| T022 | [TW-022](02_DEFECT_REGISTER.md#tw-022) | `ping_reply_drop_is_not_success` | server 在 ping 与回执之间断开；不得发布成功 RTT 或当前代 Healthy。合法 reply、合法 negative global reply 与真正断线分别测试。 |
| T023 | [TW-023](02_DEFECT_REGISTER.md#tw-023) | `host_key_prompt_is_bound_to_run_hop_and_identity` | 并发同主机提示、不同端口/不同 fingerprint、Stop 后批准、窗口重开、保存安全策略变更；过期审批永远无效。 |
| T024 | [TW-024](02_DEFECT_REGISTER.md#tw-024) | `secret_clear_and_gc_preserve_retained_references` | 更换/清除/保留、提交前后失败、恢复旧备份、并发慢连接；日志无密码，回收不删仍被引用的凭据。 |
| T025 | [TW-025](02_DEFECT_REGISTER.md#tw-025) | `inactivity_policy_does_not_kill_default_idle_tunnel` | 长时间无业务但有健康回执不误停；真正传输无响应按约定进入退避；用户设定 idle close 只影响相应层级。 |
| T026 | [TW-026](02_DEFECT_REGISTER.md#tw-026) | `first_hop_tcp_nodelay_matches_configuration` | 真实 socket 选项查询、小包 RTT 与大块吞吐 A/B；设置失败作为结构化错误/警告处理，不引入 busy loop。 |

## 3. 测试分层

### 3.1 纯模型与状态转换

对 reducer 做表驱动与事件序列测试。覆盖 Start/Stop/Restart/ConfigChanged 交错、Stopping 期间 Start 再 Stop、删除再创建同名不同 ID、错误归类、重连退避 reset、过时代号和 owner 回收。

最重要的断言是：用户 Stop 的意图不被 auto_start 或旧事件覆盖；Stopped 无 owner/listener；Healthy 必须有当前尝试的认证、forward、ping 和 live owner；旧 CleanupCompleted 仍能正确回收对应旧 owner，但不能改变新代状态。

模型测试不能替代实际 SSH 集成。用 fake clock 验证退避和 deadline，真实测试验证实际 socket 释放；不要为了让真实 OS socket 测试快，把所有时间都假装已经经过。

### 3.2 配置、凭据与崩溃一致性

用可注入文件/keyring adapter，在每个步骤前后故障：新凭据写入、tmp 创建、部分写入、sync、backup、rename、prune、系统启动设置、内存应用。判断 NotCommitted/Committed/Indeterminate 分类与实际磁盘内容相符。

强制终止测试只能针对新建临时 profile 的子测试进程，绝不对用户现有应用/真实配置做破坏性实验。每次重启验证完整旧版或新版、引用可读取、下一次能保存、备份仍可恢复。

覆盖磁盘满、权限拒绝、损坏 TOML、不支持版本、长路径/中文路径、过大文件、残留 tmp、两个实例争写、过期 base_revision。若 Windows 替换 API 行为依赖文件系统，记录 NTFS 等实际环境；不要拿 Linux 测试通过当 Windows 原子替换证据。

GC 测试要包含当前配置不再引用、但保留备份或尚在运行 owner 仍引用的凭据。不能仅断言“旧 secret 删除了”就判轮换正确。

### 3.3 本地 SOCKS / relay

覆盖 IPv4、IPv6、DOMAIN、255-byte 上限、空域名、非 UTF-8、错误版本、RSV、零端口、无 NO AUTH、BIND/UDP 拒绝、截断输入、慢客户端和队列满。

测试 DNS 请求不在本机提前解析；测试 channel 成功前不回 SOCKS success；测试未知网络错误不伪造 TTL expired。

使用双向独立数据源验证 half-close：一端写完 EOF 后另一端剩余大响应仍能完整接收。取消、写零、连接关闭和错误都要使 active_connections 回落到零。不要用一个全关的小 echo 测试代替半关闭。

### 3.4 真实 SSH / 多跳 / Remote

真实 fixture 至少涵盖单跳、2 跳、3 跳，以及第 n 跳 DNS/TCP/认证/key mismatch 失败；验证错误指向正确 hop。大量 Host 定义不代表每项都应连接。

Remote 测试覆盖固定端口、port 0、拒绝绑定、拒绝/不回复取消、forwarded 地址不匹配、IPv6、wildcard/GatewayPorts 策略和本机目标拒绝连接。停止后检查客户端资源与服务器端 listener；无法确认服务器状态的分支必须显示未确认，不能算“远端一定释放”。

断网测试不仅关闭 server，也应包含无 FIN/RST 的黑洞、KEX 卡住、ping 回复通道关闭、channel-open 不响应和间歇丢包。这样才覆盖超时路径，不能只测容易退出的正常断线。

当运行环境不支持 IPv6 时记录明确 skip 原因；发布必须另有真实 IPv6 环境通过，不可长期用 skip 代替支持承诺。

### 3.5 认证与信任

Password、加密/非加密私钥、Agent 多身份/无身份、Windows OpenSSH pipe、Pageant fallback 和多轮 Keyboard Interactive 都要覆盖。测试 echo=false 不在 UI 文本导出、日志和错误格式化中泄露响应。

Unknown、Changed、证书不支持、known_hosts 多文件冲突、Hash host、自定义端口、IPv6、信任保存失败和审批取消分别验证。固定网络预算的无 prompt 握手不能拖成 120 秒；真正出现 prompt 才进入交互预算。

导入一个带 Bypass、外部监听、auto_start、Run-at-login 和旧 SecretRef 的配置。确认任何敏感行为不会在普通预览/保存之前发生，且外部 consent 字段不能直接当本机授权。

### 3.6 OpenSSH 兼容

fixture 明确标注用户配置还是系统配置路径基准。覆盖 conditional Include、相同文件在不同上下文被引用、循环 Include、通配/否定 Host、标量 first-wins、多个 IdentityFile、ServerAliveInterval=0、未映射 ProxyJump/HostKeyAlias/known-hosts 设置。

对支持范围可在隔离测试环境用已安装 OpenSSH 生成对照结果，但产品不得通过执行命令进行导入。Match exec、ProxyCommand、命令替换永不执行；未支持项必须产生可见警告/阻断，不能静默误解释。

## 4. UI 测试与真实截图

保留已有 GPUI Kit headless 交互测试，扩展到实际数据契约和动作。单靠“按钮存在/能点击”不能验证保存成功、隧道停止或视觉美观。

必须测试：Blocked 行和托盘的 Retry/Stop、dirty guard、保存 A 后不关闭 B、ConfigRevision 冲突、组件 ID 稳定、虚拟列表筛选后操作对象不变、弹窗默认安全焦点、Keyboard Interactive 回应、导入差异及 post-commit warning。

截图来自真实 Windows release 应用，至少覆盖 07 末尾列出的状态。固定 fixture 与字体/DPI；两种主题、默认与窄窗口、长文本都要检查。100/125/150/200% DPI 用真实系统设置，不能只缩放截图图片。

键盘完整走通新建、编辑、测试、保存、停止和导入。检查 tooltip、可访问名称、焦点圈、输入密码可访问处理和 native tray；不能从 headless `aria_label` 存在直接声称 Windows 辅助技术已全面兼容。

## 5. 真实平台门槛

| 场景 | 需要的证据 |
|---|---|
| Wi-Fi 禁用/恢复 | 状态迁移、保留监听、恢复新连接数据、Stop 释放 |
| VPN/接口切换 | 提示合并/去抖，无重连风暴，错误/恢复归类正确 |
| 睡眠/唤醒 | 超时与流量间隔合理，不假报巨大 1 秒吞吐 |
| 服务器重启/rekey | 真实数据恢复，新会话可信，旧通道清理 |
| Windows 实际登录 | 启动路径正确、后台托盘出现、autostart 生效 |
| Explorer/托盘变化 | 无悬空回调；异常可恢复或明确报告 |
| 关闭/重开窗口 60 次 | UI owner/订阅释放、状态仍来自最新 core |
| 多会话/配置争用 | 配置锁行为和错误可见，不覆盖对方更改 |

用户手动停止后再经历上述网络/保存事件，必须保持停止。登录后重新运行 auto_start 是新的应用启动策略，不与本次运行手动 Stop 混淆。

## 6. 长稳

24 小时实际长稳为完整 v1 发布门槛，72 小时为建议扩展，不用几分钟 smoke 或 1000 次快速循环冒充 24 小时。

持续使用受控 TCP/SOCKS 业务流，每分钟验证数据，每小时注入一次可恢复断线或服务端重启，另有持续长连接与短连接混合。记录恢复用时、累计 bytes、成功/失败请求、generation、active tasks、handles、WS/Private 与 GPU。

24 小时测试至少覆盖主要 Dynamic 场景，Local/Remote 的故障与取消有独立足够时长测试，混合场景再验证全局预算和公平性。时间不足只能标未完成，不降低发布门槛。

所有日志保存在专用 run_id 目录；失败时保留现场摘要，不能自动删掉失败运行只留下成功样本。

## 7. 可复用的执行命令

在对应工具链和 Windows 环境执行，先检查当前仓库结构。命令是操作入口，不是本次输出。

```powershell
git rev-parse HEAD
rustc --version --verbose
cargo --version
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked -p tunnelwarden
cargo tree --locked --target x86_64-pc-windows-msvc
cargo audit --json
```

`cargo audit` 需要实际安装并记录版本、数据库 commit/更新时间及完整报告。无法联网更新数据库时明确报告所用日期，不能把旧报告写为最新结果。

原有记录中的 `TUNNELWARDEN_RECONNECT_CYCLES` 与 probe 可用于重测；先读脚本或 `Get-Help` 验证参数，不构造没有确认存在的开关。对每条命令保存 stdout/stderr、退出码和时间，不仅截图终端最后一行。

## 8. 依赖和发布

历史记录虽写“0 reported vulnerabilities”，同时记录了 unmaintained 条目与 glib unsoundness 警告，并说明部分不在 Windows 依赖树。本次未重跑，不能概括为“所有依赖完全安全”。[S27](13_SOURCE_INDEX.md#s27)

重新审计 Cargo.lock 的 Windows 实际依赖路径、可达性、维护状态和锁定版本。保留每个例外的范围与理由，禁止全局忽略 advisory。russh ping 修补与 GPUI vendor 修复要记录上游版本、补丁、测试和撤销条件。

Release 交付记录 commit、工具链、目标、Cargo.lock hash、binary hash、许可证/第三方声明和已知限制。调试符号可独立留档供崩溃分析。签名/分发策略按项目能力处理，未签名就直说，不能宣传 Windows 信任已解决。

最终 gate 必须同时满足关键缺陷关闭、原 v1 功能全部实接、真实平台与长稳完成、性能证据齐全、UI 状态和键盘流程通过。任何一个 README、mock 演示或单元测试通过都不能单独解除发布 gate。
