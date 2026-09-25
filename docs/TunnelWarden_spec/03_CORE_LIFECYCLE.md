# 03 · 权威状态机、控制面与资源生命周期整改

关联 [TW-003](02_DEFECT_REGISTER.md#tw-003), [TW-004](02_DEFECT_REGISTER.md#tw-004), [TW-005](02_DEFECT_REGISTER.md#tw-005), [TW-006](02_DEFECT_REGISTER.md#tw-006), [TW-010](02_DEFECT_REGISTER.md#tw-010), [TW-011](02_DEFECT_REGISTER.md#tw-011), [TW-012](02_DEFECT_REGISTER.md#tw-012), [TW-020](02_DEFECT_REGISTER.md#tw-020), [TW-021](02_DEFECT_REGISTER.md#tw-021), [TW-022](02_DEFECT_REGISTER.md#tw-022), [TW-023](02_DEFECT_REGISTER.md#tw-023)。现状证据为 [S03](13_SOURCE_INDEX.md#s03)、[S04](13_SOURCE_INDEX.md#s04)、[S05](13_SOURCE_INDEX.md#s05)、[S06](13_SOURCE_INDEX.md#s06)、[S08](13_SOURCE_INDEX.md#s08)、[S20](13_SOURCE_INDEX.md#s20)。

## 1. 采用的架构决定

**保留 TunnelManager 和既有 workers；让一个 reducer 成为真实生命周期的唯一写入口。** 不新建通用 actor 框架，不引入数据库，也不同时保留两份能独立决定 Healthy 的公开状态。

管理器持有每个 TunnelId 的 `TunnelSlot`，负责意图、版本、转移、资源 owner handle 和快照。真实 `TunnelRuntime`/worker 持有 listener、SSH chain、registration、relay tasks 等 I/O 资源，执行 effect 后只报告带标识的事实，不另行维护第二套权威 SupervisorState。

物理资源可以位于管理器拥有的任务内，但不能因此脱离管理器所有权。`TunnelSlot → OwnedRuntime/JoinSet → concrete I/O` 是可追踪的所有权链。Stopped 配置只保留小型元数据；不要为 1024 条未运行配置无条件创建 1024 个定时器和 Tokio 任务。

## 2. 目标模型

以下是契约示意，不要求照抄类型名，也不是已编译代码。

```rust
struct TunnelSnapshot {
    id: TunnelId,
    config_revision: ConfigRevision,
    generation: Generation,
    attempt: AttemptId,
    desired: DesiredState,
    phase: RuntimePhase,
    listener: ListenerObservation,
    remote_binding: Option<RemoteBindingObservation>,
    health: HealthObservation,
    next_retry_at: Option<MonotonicDeadline>,
    last_error: Option<FailureView>,
    counters: TrafficSummary,
    allowed_actions: ActionSet,
}
```

必须独立保存以下维度，不能塞进一个 enum 后互相丢失。

| 维度 | 约定 |
|---|---|
| DesiredState | 用户本次会话意图 Running/Stopped，不能从 auto_start 或 active map 推断 |
| RuntimePhase | Stopped、Queued、Binding、Connecting、AwaitingHostKey、Authenticating、Registering、Running、Degraded、Backoff、Blocked、Stopping |
| Listener | Unbound、Bound(actual address)、Releasing；Remote 模式注明本机是目标，不伪造本地监听 |
| RemoteBinding | requested address/port、实际分配端口、registration owner/attempt；独立于健康状态保存 |
| Health | Unknown、AwaitingProbe、Healthy、Degraded、Failed；带最后成功时间及逐跳结果 |
| Generation | 手动重启、影响连接的配置变更、Stop 等使旧生命周期结果失效 |
| AttemptId | 同一 desired lifecycle 中每轮连接递增，重连后旧 probe 不能污染当前链 |
| RuntimeOwnerId | 唯一资源实例，专门用于清理确认，不能混同 config revision |
| ConfigRevision | 已提交配置版本，用于拒绝旧保存和旧测试结果 |

`Healthy` 的最低事实为：当前 generation/attempt、SSH 链各跳已认证、转发器/registration 就绪、一次有效 session probe、owner 仍存活且未被取消。端口绑定、任务存在、TCP connect 成功都不能单独满足。

Local/Dynamic 的“转发器就绪”不等于所有目标服务均可达。`TargetReachability` 是另外的可选诊断结果；SOCKS 没有固定目标，不能自动访问第三方网站当健康检查。

## 3. 事件与 effect

工作线程必须回传 `tunnel_id + generation + attempt + owner_id`，具体事件至少包含 ListenerBound、ChainAuthenticated、TrustRequired、AuthenticationRequired、ForwardReady、ProbeCompleted、AttemptFailed、CleanupCompleted、RuntimeExited。

管理器快速执行以下流程。

```text
接受命令/完成事件
  → 验证 ID / revision / generation / owner
  → reducer.transition(event)
  → 更新小型权威模型
  → 产生有界 effects
  → 安排 owned I/O job 或向现有 worker 发指令
  → 返回控制事件循环
```

不能在这个流程中等待整个 SSH 连接、远端取消注册、keyring、磁盘备份或 N 个 Stop。`spawn_blocking(...).await` 仍然占住当前 actor；需要保存 job handle 并在完成事件到达时处理。

普通状态快照允许合并为最新值。重要生命周期事件走单独的有序有界 journal；不能根据 250 ms 快照之间的 enum discriminant 变化重建完整历史。

## 4. 命令语义表

| 当前情况 | 命令 | 必须结果 |
|---|---|---|
| Stopped | Start | desired=Running，创建新 owner/generation；按预算进入 Queued 或启动 |
| 已要求 Running | Start | 幂等确认；不再启动第二套 listener/chain |
| Running/Backoff/Blocked/审批中 | Stop | desired=Stopped，作废旧结果，取消审批/重试，进入 Stopping，最终释放全部本地资源 |
| Stopped | Stop | 幂等确认，不增加任务 |
| Stopping | Start | 记录“清理后启动”；不能在旧 listener 未释放时抢跑 |
| Stopping，已排定重启 | Stop | 清除 pending restart；最终必须 Stopped，不能因旧 Restart 再次运行 |
| Running | Restart | desired 保持 Running；旧 owner 完成清理后开始新代 |
| Blocked | Retry | 明确用户重试，允许重新构建；已知密钥变化不得因此自动受信 |
| Backoff | Retry now | 在当前资源约束内提前一次重试，不新建并行重连循环 |
| 手动 Stopped | 任意配置保存 | 仍为 Stopped，即使 auto_start=true |
| Running，仅改名称/分组/备注/主题 | Save | 更新显示/定义，不断开链，不重置累计流量 |
| Running，改端点/认证/链/策略 | Save | 按已提交 revision 安排一次重启，保留用户当前意图 |
| 导入/新建配置 | Save | 默认只保存；run_now 独立授权，auto_start 仅影响下次启动初始化 |
| Backoff 且网络变化 | NetworkRecovered | 合并重复事件，可提前一次尝试；不能复活手动停止的隧道 |

所有 UI、托盘、分组、快捷键共用同一个 action resolver。管理器再次检查命令合法性，不能只靠禁用按钮防竞态。

## 5. 过期结果与清理确认不是同一规则

旧代 `Healthy`、认证成功或新连接资源不能写入当前模型。携带资源的迟到结果必须交回 owner 清理，不能仅仅“忽略消息”后遗忘 socket。

`CleanupCompleted` 则根据正在清理的 **RuntimeOwnerId + StopOperationId** 接收。用户 Stop 时 generation 已前进，不代表旧 owner 的清理回执也应被丢弃。否则会卡在 Stopping 或错误地允许第二个 listener 提前启动。

至少验证以下竞争序列。

```text
Start(g1) → Stop(g2) → 迟到的 Authenticated(g1)
Start(g1,a1) → Backoff → StartAttempt(a2) → 迟到 Probe(a1)
Restart → Stopping → Stop → CleanupCompleted
Save(rev10) → Stop → SaveApplied(rev10)
编辑 host → 新 fingerprint prompt → 旧窗口批准旧 prompt
删除 TunnelId A → 重建同名但新 ID B → A 的回调到达
```

同名不等于同 ID，attempt 不等于 generation，保存完成不等于要求运行。

## 6. Listener 和转发资源所有权

本地 listener 由当前 runtime owner 独占。Local/Dynamic 短暂重连只替换 SSH attempt，**不释放 listener**；这时拒绝新业务连接并给出有界失败，不无限等待，也不假装有转发能力。

明确 Stop 释放 listener。手动 Restart/影响端点的配置变更可以选择完整释放后重建；本方案不承诺这类显式重启期间端口无间隙。若未来要保留同端点 listener，必须实现独立 dispatcher 的所有权转交测试，而不是两个 worker 同时借用。

Remote 的 listener 在服务端。客户端只有 registration lease、SSH transport 与本地目标连接。停止后不能把“本地任务取消”描述成“服务端已确认关闭”，尤其是在网络黑洞中。

## 7. Stop 与 Quit 的统一期限

采用唯一 `StopContext`，含绝对 deadline、operation id、原因、是否应用退出。禁止层层新建完整 3/5 秒导致总时限累加。

建议保留 **正常停止总预算 3 秒** 作为首轮工程目标，所有阶段从剩余时间中分配；若实测不足，必须整体调整而不是局部偷偷延长。

1. 同步关闭 admission，标记 Stopping，取消 pending opens/probes/用户审批，通知所有 relays。
2. Remote 在 transport 仍可用时尽力 cancel 注册，使用剩余预算的一小部分；不允许它吃掉整个总预算。
3. 取消 attempt/transport，逆序清理跳板，回收任务。Local listener 明确 drop。
4. 到 deadline 后 abort **可取消 async** tasks 并完成 join；记录 forced cleanup 与剩余诊断，不把超时当普通成功。
5. 只有资源释放/强制收束达到约定后，命令结果进入 Completed。UI 接受命令时就显示 Stopping，而不是伪造已经 Stopped。

`StopAll` 先给所有目标发取消，再并行收束，共用全局 deadline。不得逐个等待三秒。Quit 的核心清理预算可单独取 5 秒作为起点，但仍是全局预算，不是每条隧道各五秒。

如果服务端确认不可获得，关闭本地 SSH transport 后显示“本地已停止；远端释放未确认”，让服务端按连接失效处理。这不是许可保留本地 relay，也不是声明远端端口已经实测释放。

## 8. 阻塞 job 的特殊边界

已开始的 keyring/文件同步任务不能靠 Tokio abort 终止，`shutdown_timeout` 也不是杀线程。[E06](13_SOURCE_INDEX.md#e06)

因此保存事务要有可恢复提交点，阻塞 job 数量有界，并由 owner 保存状态。正常退出先禁止新写入、等待当前短事务结束，再销毁 runtime；超过整体预算时显示或记录“系统 I/O 未结束/提交状态待恢复”。不能为消除界面卡顿创建无人跟踪的后台写入。极端 OS I/O 卡死下的强制进程退出依赖 05 的崩溃恢复，不承诺文件系统外部操作已经取消。

## 9. 退避、健康和网络恢复

保留指数退避、jitter、稳定后 reset、Blocked 不自动无限重试的原则。稳定计时应从当前 attempt 达到真正 Healthy 开始，而不是从 TCP socket 建立时开始。

认证拒绝、未知/变化密钥等待处理、无效配置、缺失凭据进入可操作的 Blocked。网络断开、连接超时和适当的传输故障进入 Backoff。目标服务单次拒绝不应无条件重启整条健康 SSH 链。

`next_retry_at` 用单调时钟绝对期限。界面按这个期限显示剩余秒数，不把配置 delay 一直重复显示。网络接口事件需要合并/去抖并保留审计原因；不要把每次接口通知当互联网已经可达。睡眠恢复后做一次受预算约束的有效性探测，不能改为永久每 100 ms ping。

## 10. 分步迁移

先给现有真实 supervisor 增加事件 envelope 和 owner tracking，保留数据面与现有测试；接入权威 reducer；迁移健康、失败与退避事件；最后删除重复公开状态和从 active map 推断意图的代码。每一步都要求 Stop、真实转发和重连测试通过。

最终检查所有 Start/Stop/Restart/Config/Network/Prompt 入口都走同一 transition。代码中不能出现 UI 根据任务存在自行写 Healthy，或 worker 与 manager 各自维护不同的 desired。
