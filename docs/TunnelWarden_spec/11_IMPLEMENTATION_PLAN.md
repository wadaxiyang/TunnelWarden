# 11 · 实施顺序与改动边界

## 1. 执行原则

本计划把全部整改拆成可评审的工作包，不把核心功能推到一个没有日期的“以后”。所有原 v1 能力仍是同一次公开发布的门槛。每包可以拆为多个小 PR，但不能把未接线功能写进已完成列表。

开始前核对 HEAD 与基线 `71bf07c4c2185e8901166e2fbb2ccb8894effc78` 的差异。如果相关问题已修复，追加验证而不是强行重做。不要覆盖根 AGENTS.md 或历史验证记录。

原则是先给确定缺陷补回归，随后统一状态和事务契约，最后让 UI 接真实数据。不是先把所有文件拆开，再希望功能自然正确。

## 2. 工作包关系

```text
WP0 确定性数据/地址/运行意图修复
 └─ WP1 生命周期 + 控制面 + 配置完成事件契约
     ├─ WP2 SSH/安全/导入语义与认证
     ├─ WP3 流量与诊断数据闭环
     └─ WP4 UI 设计系统和异步编辑边界
          └─ WP5 页面流程与功能完整性
               └─ WP6 真实平台、性能、长稳、发布签收
```

WP2/3/4 可以在 WP1 的稳定接口确定后分别开发。不要让三个分支分别发明一套 TunnelSnapshot、错误格式或 ConfigRevision，再到最后靠适配器补丁拼起来。可以共用 fixtures 开发视觉，但生产路径不得保留假的 Healthy/流量数据。

## 3. WP0 · 先修确定的小范围缺陷

**主要登记项**　TW-001、002、003、007、019。TW-003 在这里建立明确运行意图，WP1 再纳入完整权威 reducer。

**目标文件**　`config-store/src/store.rs`、`schema.rs`、`tunnel-core/src/manager.rs`、`tunnel-domain/src/validation.rs`、`app/src/editor.rs` 及相应 tests。

1. 为 post-commit pruning 失败补回归，区分提交前失败与已提交警告，消除误删除新 SecretRef。
2. 同目录唯一 tmp、明确写锁/恢复规则，补崩溃后下一次保存测试；不要先删原配置再 rename。
3. 将 auto_start 限定为应用启动策略；手动停止后无关保存不得重启。不能只用 active map 是否存在替代 Desired。
4. Local/Dynamic 用 IpAddr + SocketAddr::new；覆盖 IPv6 完整启动路径。
5. 所有保存/测试/应用路径采用校验后返回的 canonical 配置，补 `~` 路径保存与重载一致性。

**禁止夹带**　重画所有 UI、升级全部依赖、共享 session 池、改 SSH 协议、为了简单禁用自动启动或 IPv6。

**退出条件**　T001/002/003/007/019 有故障/修复证据；原有普通保存、导入、端口释放与三种转发测试未回退。若先实现最小 SaveOutcome，提交点语义必须与 05 一致，后续增加 OperationId 不能再改变其含义。

## 4. WP1 · 统一真实生命周期与控制面

**主要登记项**　TW-004、005、006、011，复核 TW-003；为 TW-018/020 提供公共契约。TW-022 的稳定成功证据接口在此冻结，具体依赖修补可在 WP2。

**目标文件**　`tunnel-domain/src/state.rs`、`tunnel-core/src/state_machine.rs`、`runtime.rs`、`manager.rs`、`supervisor.rs`、`remote_supervisor.rs`、`app/src/runtime_host.rs`；路径如 HEAD 变化先定位现有模块。

先定义 Snapshot、Desired、generation/attempt、owner ID、OperationId/ConfigRevision 和 typed events，再逐步将原 supervisor 接入同一 reducer。保持现有 relay 和 SSH 功能，用适配过渡，不一次重写所有数据面。

把 manager 内耗时 await 变成受持有和追踪的 job completion。为 Stop、Quit 和提示响应保留响应能力；公平消费事件，防止持续命令洪水使状态不更新。

统一 StopAll 的先取消后共同回收、Remote cancel 的剩余时限、Quit 的全局预算。started blocking jobs 单独追踪，不能声称 abort 已销毁。减少 RuntimeHost::Drop 中不可控同步等待，确保退出策略对未完成工作给出可恢复语义。

Blocked、Stopped、Stopping 的合法动作由公共映射提供，托盘和主表不再使用互相矛盾的 Toggle 逻辑。资源清理完成事件不能因 generation 旧就失去回收资格。

**退出条件**　T004/005/006/011；真实 SSH 集成下每次 Healthy 都有当前代事实；快速 Start/Stop/Restart、保存、网络恢复交错无旧代回写；所有任务 owner 与 join/cancel 路径可画出一张图。

## 5. WP2 · 协议、安全与导入

**主要登记项**　TW-008、009、010、012、013、016、021、022、023、024、025、026。

**目标文件**　`ssh-engine/src/client.rs`、`chain.rs`、`host_key.rs`、`forwarding/src/socks5.rs`、`tunnel-core/src/local_forward.rs`、`remote_forward.rs`、`config-store/src/ssh_import.rs`、`secrets.rs`、schema/manager 相关入口。

按风险拆为小 PR：监听授权与安全导入；超时与有界并发；Keyboard Interactive；known_hosts/信任上下文；OpenSSH 语义；凭据生命周期；依赖 ping 与 TCP 选项。不要把这些变成一个无法评审的 5000 行“SSH 重构”。

本包的安全策略必须在 core 生效，不只弹 UI warning。外部导入不得继承本机 consent、自动运行或复用私有凭据引用。事务后处理遵守 05。

保留 SOCKS remote DNS、半关闭、Remote registration 校验和逐跳认证。全局 budgets 必须在创建高成本任务之前 admission。实现 KI 的多轮、echo、取消与零日志泄露，而不只是把枚举分支改成一个成功返回。

对于 TW-022 先写固定版本回执消失的测试，然后确定最小上游修复/本地补丁；不能仅加 `is_closed()` 就宣称一定收到 pong。对于 TW-026 直接测试 socket 选项，不把可能的小包性能影响当既成测量结果。

**退出条件**　对应 T008/009/010/012/013/016/021/022/023/024/025/026 完成或有充分证据的条件风险排除；安全负面 fixture 通过；已实现三种数据面无降级。

## 6. WP3 · 监控与诊断闭环

**主要登记项**　TW-014、020。

**目标文件**　`forwarding/src/relay.rs`、`tunnel-core` 中计数 owner、采样器、日志与 snapshot 模块；app 仅准备只读 view models。

稳定累计计数与 Attempt 解耦；一套 1 Hz sampler、有界 120 样本、真实时间间隔、全局汇总不重复计 hop。实现监控开关，active guard 取消归零。

错误由 string-only/discriminant-only 改为阶段、主体、代号、原因、可重试性、时间与安全上下文。状态快照只保留最新值；关键事件另有有界 journal，避免短暂失败在 250ms 刷新间被完全吞掉。

**退出条件**　T014/020，三种模式方向正确；重连累计不倒退；关闭监控无历史计时器/绘图空转；错误能从 UI 回溯真实失败阶段。保持日志和样本容量可计算。

## 7. WP4 · UI 基础与编辑一致性

**主要登记项**　TW-015、018；落实 06 的设计系统。

**目标文件**　`app/src/workspace.rs` 拆出有限的 `theme.rs`、`metrics.rs`、`ui/`、`view_models/`、`editors/` 等模块；仅在责任清楚时拆，禁止每函数一个文件。

建立 AppShell、主题、Table/Picker/Inspector、Overlay/focus 和 operation feedback。页面不自建 runtime、不拥有生产隧道。InputState 只创建一次，稳定 key，按可见状态更新。

接入 canonical config watch、EditSessionId、OperationId、WindowEpoch 与 dirty guard。保存、测试和导入回执不会清空后打开的编辑器；冲突不 last-write-wins。

**退出条件**　T015/018；双主题实际切换不重连 SSH；0/1000 项表格和 16 跳编辑可用；主表、Blocked、Unknown 弹窗三类实际截图先验收，避免整个 UI 做完才发现方向不对。

## 8. WP5 · 页面和完整功能

**主要登记项**　TW-017，复核 TW-006/013/014/015/016/020/023/024 的用户流程。包含本轮建议新增的命令导入。

按 07/08 完成 Tunnel/Host CRUD、搜索筛选、详情、Test Connection、分组移动/排序、监控图、组合日志与导出、Settings、安全导入和原生托盘动作。没有数据的页面给真实空状态，不给假演示成功。

工作流验证从用户入口走到底：草稿 → 校验 → 测试/保存 → 实际连接 → 故障 → 诊断 → Stop。不能只在组件测试注入 ManagerSnapshot，然后宣布后台功能已接入。

**退出条件**　T017 与全部功能矩阵行有可操作证据；两种主题和键盘流程可完成；主机被引用时删除有明确方案；导入预览真正列出敏感差异；没有死按钮、未读设置或表面支持的认证方式。

## 9. WP6 · 同机基准与发布签收

**所有登记项的最终验证**，重点复核条件风险和平台行为。

按 09/10 执行 release benchmark、窗口循环、物理网络/登录、24h 长稳、依赖审计。记录失败与未执行项，不删坏样本。Loris 对照使用固定真实版本，同机共同场景；未测则不写比较结论。

将最新证据追加到验证记录，并保持旧记录不被改写。运行 14 的清单评审，确认本轮修改后的 commit 与被测 binary 一致。

**退出条件**　原 v1 完整范围、关键 bug、UI、安全、资源、依赖和真实平台门槛同时通过。尚未完成的 gate 保持开放，不把 release 名称当完成证据。

## 10. 每个 PR 的固定附件

| 附件 | 内容 |
|---|---|
| 改动说明 | 问题 ID、实际源文件、设计变化、不改变什么 |
| 回归证据 | 命令、退出码、fixture、失败前与通过后差异 |
| 资源说明 | owner、取消、join、容量、重复运行/峰值影响 |
| UI 证据 | 实际截图/交互结果；无 UI 改动则注明 |
| 兼容性 | 配置迁移、旧数据、凭据、备份与回滚 |
| 未完成项 | 原因、影响、release gate；不能用“后续优化”掩盖必要功能 |

回滚不仅是 `git revert`。若配置 schema 已升级，必须说明旧 binary 是否能读取；不能在仍使用新配置/SecretRef 的情况下回滚并删除必要凭据。
