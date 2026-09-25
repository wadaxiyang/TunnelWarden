# 12 · 可直接交给开发 Agent 的指令

## 使用方式

将本包放到仓库 `docs/remediation/` 或其他明确路径，按 11 的顺序一次执行一个工作包。以下指令是开发任务，不是本次已经完成的代码修改。

总入口之后附上所需工作包指令。不要把全部包同时交给多个无共享契约的代理改同一 manager/schema。

## A. 总入口

```text
你正在整改 TunnelWarden，一个 Windows 原生 SSH Tunnel GUI supervisor。
技术栈保持 Rust + Tokio + russh + GPUI Kit，不 shell out 到 ssh.exe/plink。

先读取根 AGENTS.md、TunnelWarden_SPEC.md 的相关部分，以及整改包
00_README、01_AUDIT_BASELINE、02_DEFECT_REGISTER、11_IMPLEMENTATION_PLAN。
审计基线为 71bf07c4c2185e8901166e2fbb2ccb8894effc78。
先比较当前 HEAD；已经修复的条目只补验证，不退回旧实现。

只执行本次指定的工作包。先确认真实生产调用路径，再写能覆盖它的回归，
然后最小化修改。不能把测试 fixture、假 Healthy、随机流量接进生产。

保留 Local、Remote、Dynamic SOCKS5、多跳、严格信任、remote DNS、半关闭、
取消清理、托盘与现有 Windows GPUI vendor 修复。原 v1 范围不可删减。
不要引入 WebView、终端、SFTP、数据库、云、AI Debug、账号或付费开关。
不要批量升级所有依赖，不要为编译通过删除 desired/runtime/listener/health/generation。

每个长寿命资源明确 owner、容量、取消和 join；不得 detached task、无界 channel、
全局粗锁或隐藏 keeper window。render 不做 I/O、创建长期实体/订阅或生成随机 ID。
实际 GPUI API 以锁定的 0.6.6 源码/示例为准，不照抄网上不同版本方法名。

文档中的 Rust 结构是设计契约，不代表已经可编译。按仓库实际类型落实。
未执行测试就写“未执行 + 原因”，不得伪造输出或沿用旧日期结果。
Windows、GUI、真实登录和 24h 未验收时保持 release gate 未关闭。

完成后按本文件 H 模板输出：实际 diff、问题 ID、测试命令/退出码/证据路径、
资源生命周期、兼容性/回滚、未完成项。不要只总结“已优化、已完善”。
```

## B. WP0 指令

```text
执行 WP0，读 05_CONFIG_AND_IMPORT、04 的端点部分以及 TW-001/002/003/007/019。

1. 追踪 ConfigStore::save 的 rename 与 prune_backups，以及 manager 的失败回滚。
   先加 post-commit pruning 失败测试。已提交时不得删除新 SecretRef。
2. 区分 NotCommitted/CommittedWithWarnings/无法确定提交；设计最小可用返回契约。
   不用笼统 Result 继续混淆提交与维护。保存失败/成功的 UI 文案对应真实状态。
3. 解决固定 tmp 在 crash 后阻断保存；唯一同目录临时文件、受控恢复与写锁，
   不允许先删除原配置。强杀测试只在临时 profile 子进程。
4. auto_start 只在应用启动形成 Desired；用户 Stop 后无关配置保存不得复活。
5. IPv6 bind 用 IpAddr + SocketAddr::new，而不是 format host:port 再 parse。
6. 实际采用 canonical normalization 返回对象，验证 ~ 路径立即连接与重载一致。

新增/复用 T001/002/003/007/019。保持三种转发和现有普通保存测试。
不要在此包改整套 UI、更新依赖或重写 SSH 数据面。
```

## C. WP1 指令

```text
执行 WP1，先读 03_CORE_LIFECYCLE、05 的异步提交契约与 07 的动作矩阵。

冻结统一 Snapshot、Desired、generation/attempt、owner、operation/revision 契约。
用一个 reducer 接入真实 supervisor；worker 只执行资源 effects 与上报 typed events。
不要保留两套能各自判断 Healthy 的公开状态；也不要删掉原正式状态模型。

manager 不在主命令循环等待慢配置/Stop；持有 job handles 并消费完成事件。
StopAll 先并发取消、共享剩余清理时间。Stop 后旧代任务只能清理，不能重新报告健康。
CleanupCompleted 根据旧 owner/stop operation 回收，不能因 generation 旧直接丢弃。
处理 Stopping 中 Start 又 Stop 的最终意图；保留 transient reconnect 的本地 listener。

Blocked 的主表/详情/托盘都能 Retry 和 Stop；命令满/关闭时有明确反馈。
定义共享 Stop/Quit 时限；started spawn_blocking 不可 abort 的事实必须反映在清理策略。
不以同步 thread.join 无限等待宣称“有界退出”。

完成 T004/005/006/011 并回归 T003；真实 SSH 路径验证 Healthy 的当前代事实。
提交 owner 图和逐状态 transition 测试，不用 mock 成功代替生产接线。
```

## D. WP2 指令

```text
执行 WP2，读 04_SSH_FORWARDING_SECURITY、05_CONFIG_AND_IMPORT 和对应登记项。
分成可独立评审的小修改，不把协议、存储、UI 混成一个巨型重构。

先落实 core ExposurePolicy 与 ImportPlan。外部 Bypass、非回环监听、auto_start、
Run-at-login、SecretRef 和 consent 不能未经明确确认变成实际行为。
保持 Strict；Changed 不复用 Unknown 普通同意流程。

将网络握手预算与真正的交互等待预算分开。channel-open 从入队计算 deadline，
有界并发执行，健康探测不阻塞全部新连接。加全局 session/relay/handshake/key-decode
预算，admission 在高成本任务创建之前；保留半关闭、remote DNS 和严格 Remote 校验。

实现完整多轮 Keyboard Interactive，echo 规则、prompt 归属、取消、超时和脱敏齐全。
OpenSSH 导入修正 Include scope/path、重复上下文、列表参数和 ServerAliveInterval=0；
不执行 Match exec、ProxyCommand、ssh -G 或 shell。无法映射的安全语义不能静默丢弃。

先复现 russh 0.63.3 send_ping 回执关闭风险，再做最小错误传播修补并记录上游版本。
直接检查第一跳 TCP socket 的 nodelay，不把 Config 字段当真实 setsockopt。
凭据 Keep/Replace/Remove 和 GC 保护备份/运行引用；inactivity 不误杀默认长期空闲连接。

按 11 的 WP2 测试列表执行。条件风险无法复现时给测试范围，不强行说修复了漏洞。
```

## E. WP3 / WP4 指令

### WP3 监控与事件

```text
执行 WP3，读 08 的计数定义、04 的诊断契约、09 的采样预算。

将 TrafficCounters 生命周期与 reconnect attempt 分离，跨自动重连累计不倒退。
明确 Remote 模式上下行方向。统一 1Hz sampler，按实际单调时间间隔求速率，
120 样本有界历史，关闭开关停止采样/绘图，不影响 relay 和健康。
全局汇总每 Tunnel 计一次，不按 hop 重复相加。

typed error/event journal 保留阶段、主体、hop、attempt、generation、时间、原因和
retryability，日志脱敏且有界。latest-state watch 不代替关键事件历史。
不按 packet 刷 UI，不无界 clone/caching，不新增数据库。

T014/020 通过；提交三种模式方向、重连、暂停监控和 active guard 释放证据。
```

### WP4 视觉基础与异步编辑

```text
执行 WP4，读 06_UI_DESIGN_SYSTEM 与 07。保持 GPUI Kit 0.6.6，不换框架。

按给定 token/metrics 建原创 Zed 风格 AppShell、虚拟 TunnelTable、Inspector、
搜索 HostPicker、Overlay/focus 与 OperationFeedback。用原有组件，不把选择器全做按钮。
先完成主表、Blocked 详情、Host-key dialog 三个实际界面截图，确认信息层次后再扩页。

深/浅/System 真实接入，主题切换不重启隧道。render 只描述 UI；InputState、
FocusHandle、订阅和滚动状态在 owner 创建，不在 render 重建。

实现 config watch、EditSessionId/OperationId/WindowEpoch、dirty guard 与 revision conflict。
保存 A 后不关闭 B，不丢 B 草稿；旧 TestRun 结果不污染新 draft。
隐藏窗口释放 view/subscription，不添加 keeper window，不 trim Working Set。

执行 T015/018 和稳定 ID、长列表、窄窗口、键盘焦点测试。
截图必须来自实际应用；无法运行 Windows 就标未验收，不用网页稿冒充。
```

## F. WP5 指令

```text
执行 WP5，按 07 的每个页面和 08 的每个功能矩阵行补齐真实用户流程。

Tunnel/Host 的 Delete、Duplicate、Search、Test Connection，Group 移动/拖动排序，
逐跳 Inspector、真实流量、Logs 组合筛选与本地导出，Settings 和安全 Import 都要接 core。
测试 draft 使用独立 owner，不抢生产 listener，不把测试成功当生产 Healthy。
Duplicate 新 ID、默认不自动运行；Delete 先正确停止，不先藏 UI 再留下运行任务。

加入建议的安全 SSH 命令导入：只解析 -L/-R/-D/-J 等受支持语义并预览，绝不执行。
所有未知安全/认证语义展示警告或阻断；引用和端口冲突明确解决。

所有状态有正确动作，Blocked 可 Stop，queue full 有反馈，原生托盘与主界面一致。
不留死按钮、不留仅存储却不生效的设置、不用假数据填图。

执行 T017 及相关功能接线回归，按 07/10 提交双主题真实状态截图和键盘流程证据。
```

## G. WP6 指令

```text
执行 WP6，读 09_MEMORY_AND_PERFORMANCE、10_TESTS_AND_RELEASE 和 14_ACCEPTANCE_CHECKLIST。

固定 current commit、Cargo.lock、工具链和 release binary hash。保存 fmt/test/clippy/build/
audit 的原始输出、退出码与时间；audit 记录数据库版本及所有告警，不笼统忽略传递依赖。

在受控 Windows 环境执行真实 IPv6、Wi-Fi/VPN/睡眠恢复、登录、托盘和60次窗口循环。
执行24小时实际长稳，保存业务数据正确性与资源采样；smoke不等于24小时。

资源分开测 Working Set、Private Bytes、handles、threads、GPU dedicated/shared、CPU。
分别测 fresh tray、打开窗口、打开后关闭、多隧道空闲、满载、故障恢复、大配置。
保留最小 GPUI window 对照。要比较 Loris 就记录真实版本并同机同场景统计归属进程；
没有安装/测试就写未测，不编造更省内存的结论。

逐项填写14验收表。没有证据的格子不能打勾。
所有必要能力与gate通过才关闭v1门槛，否则交付事实、剩余风险与尚缺的验证。
```

## H. 每包结果模板

```text
工作包 / 当前 HEAD / 被测 binary hash

1. 实际修复
   TW-ID；源文件/关键函数；修改前触发；修改后语义。
2. 实际测试
   命令；环境；退出码；证据文件路径；未执行项与原因。
3. 生命周期与资源
   owner；取消/join；容量与过载；停止/重连/窗口关闭后的归属。
4. 配置与兼容
   schema/revision；凭据与备份；提交点；回滚方式。
5. UI 证据（有 UI 改动时）
   实际状态截图；主题/DPI；键盘/焦点；真实数据接线。
6. 尚未关闭
   TW-ID或feature gate；证据不足或未实现的具体原因。
```

不能用“全部测试通过”掩盖只跑一个包，也不能用“目前没有明显问题”替代未运行的 Windows/长稳验证。最有用的交付是可复查的 diff 与证据，不是很长的自我评价。
