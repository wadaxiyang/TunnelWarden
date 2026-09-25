# 09 · 内存归因、性能预算与可比测量

## 1. 先纠正判断依据

Rust 不保证应用内存低，GPUI 不保证在所有 Windows 场景低于其他 GUI 技术。项目已有的 Windows 记录显示，完整应用窗口的量级接近最小 GPUI Kit 窗口。当前没有本次独立测量，不能把“主观感觉没优势”直接解释为仍有线性内存泄漏，更不能承诺换一种主题就减少几十 MB。[S27](13_SOURCE_INDEX.md#s27)

本轮目标按顺序是消除无限增长与资源残留、减少无效工作、控制负载扩展成本、最后公平比较 Loris。界面完善与内存最低不是同一个任务，但二者可以通过虚拟化、按需创建和事件驱动兼顾。

## 2. 仓库已经记录了什么

以下全部是 `docs/verification.md` 的**仓库自记结果**，不是本次执行。未获取原始 CSV、机器完整配置及所有采样细节。数值单位沿用原记录的 MB 标记，没有在本次核对十进制/二进制换算；新测量统一保存 bytes，并在展示时明确 MiB。

| 场景 | 仓库记录 | 能支持的有限判断 |
|---|---|---|
| core 1000 次重连测试进程 | Private Bytes 约 2.0–2.5 MB，handles 141–143 | core fixture 在该测试中的占用低；**不是 GUI 进程占用** |
| 完整应用 30 次窗口关闭/重开的一次记录 | WS 61.1→60.2 MB；Private 86.0→113.5 MB，峰值 137.7 MB | 未呈持续 WS/handle 线性增长，不能仅取最小值当固定占用 |
| 从未打开窗口的后台托盘启动 | WS 48.5 MB，Private 56.9 MB，37 threads | fresh-background 基线，与打开后关闭不是同一状态 |
| 最小 GPUI 单标签窗口 30 次循环 | WS 63.5 MB，Private 113.0 MB，48 threads，GPU local 31.5 MB | 该机器上的框架/窗口常驻成本不能忽略 |
| 完整应用 60 次窗口循环 | WS 59.5→62.8 MB；Private 107.7→116.2 MB；handles 599→590 | 该次记录未出现持续线性增长，仍需用统一协议复测 |
| 关闭完整窗口后 5/25 秒 | WS 62.4 MB；Private 82.5 MB | 关闭窗口不意味着 renderer 全部内存归还 OS |
| 最小窗口关闭后 25 秒 | WS 59.7 MB；Private 81.4 MB | 框架常驻行为与应用业务内存需区分 |
| 1000 条 stopped 定义，启动配置共享优化 | 后台约 53.6/61.7 MB 降到约 52.1/60.2 MB | 这是**已经实施**的重复配置优化，不应再报为未修复 |

记录还说明：原先约每次窗口循环增长 23 MB Private Bytes 的 Windows drop-target 问题，已通过 vendor patch 修复；托盘 presentation 去重也已完成。不得删除这些修复，也不得把它们包装成本次新实现。[S27](13_SOURCE_INDEX.md#s27)、[S21](13_SOURCE_INDEX.md#s21)、[S28](13_SOURCE_INDEX.md#s28)

单一 Tokio runtime 已明确 `worker_threads(2)`。操作系统看到的 37–52 个线程含框架、驱动/渲染、平台服务和其他执行线程，不等于应用开了 52 个 Tokio worker。[S20](13_SOURCE_INDEX.md#s20)

## 3. 应优先归因的成本

| 项目 | 当前证据层次 | 工作方式 |
|---|---|---|
| 框架/窗口/GPU 常驻成本 | 仓库记录支持其存在 | 重测 minimal window 与 full app，不能精确相减成“业务只占 X MB” |
| 持续全量快照克隆与日志发布 | 源码有相关路径 | 记录每秒分配量/唤醒次数和实体 invalidation；有收益证据再改 |
| 长编辑表单渲染所有可选 hosts/groups | 源码明确 | 搜索选择器与虚拟列表，避免 512 个 Add 按钮 |
| 串行 channel-open / health await | 控制路径明确 | 降低连接排队延迟，不宣传必然降低空闲 RAM |
| 多隧道并发握手/解密 | 有单项限制但需总预算 | 引入全局 admission，测峰值与响应时间 |
| 会话/图像/字体缓存 | 没有本次剖析证据 | 不凭猜测关闭正确缓存或改 allocator |
| WS 和 Private 的差异 | 必须分指标观察 | 不把一个数字下降解释为总内存下降 |

原则是先看到开销，再改最小路径。不要只凭 Rust 类型名中的 `Arc`、`Vec`、`Box` 判定“这是内存泄漏”。有限上限内的缓存、allocator retention 和真正持续增长是不同现象。

## 4. 有界运行预算

以下为首轮建议值，**不是已测最佳配置**。原有静态定义上限仍为 hosts 512、tunnels 1024、groups 128、chain 16；配置数量不等于承诺 1024 条隧道可同时满负荷运行。

| 运行资源 | 建议起点 | 过载策略 |
|---|---:|---|
| 并发 SSH chain 建立 | 4 | 有界等待，显示 Queued，可 Stop |
| 同时 CPU 密钥解码/解密 | 2 | 在全局 semaphore 之前不派发无限 blocking job |
| SSH session 总预算 | 256 | 启动前按整条 chain 预留，不足时排队/拒绝 |
| 同时 active relay | 全局 512、每隧道 64 | 明确拒绝/背压，保留取消路径 |
| channel-open in flight | 全局 32、每隧道 4 | 从接收起计算 deadline，过期清理 |
| 每隧道候选等待连接 | 上限 64 或更小 | 满时快速失败，不无界扩容 |
| 流量样本 | 每隧道 120 | 覆盖最旧样本，只为启用监控的对象分配 |
| 日志 | 500 条基线 | 淘汰可观察；复杂日志对象另设总字节预算 |
| 安全审批 | 维持有界队列 | 去重需精确身份，满时明确提示而非自动信任 |

会话预算不能让每条链先拿一部分 permits 再等待其余部分，否则多链可能互相占用导致无法推进。使用整链原子预留或等价无循环等待方案，失败/取消归还尚未使用的部分。

当前 relay 为双方向各 16 KiB。在建议全局 512 条 active relay 上，仅这部分缓冲理论量为 `512 × 2 × 16 KiB = 16 MiB`，还不含 SSH 数据结构、Tokio task、TCP buffer 和 GPU。预算是推算，不是分配器实测。不要把 SSH 协议 window credit 直接当作同等大小物理 buffer。

若样本设计成 32-byte 固定结构，1024×120 的原始样本载荷为 3.75 MiB；实际布局、对齐、索引与对象开销需要 `size_of` 和 profiler 确认。不要按每条曲线创建一份重复历史。

## 5. 具体优化顺序

### 5.1 先停止做无效工作

Core 保留真实健康维护，UI 仅在可见信息变化时更新。流量历史使用统一 1 Hz 采样，不给每行启动独立 timer。关闭窗口后解绑 GUI 订阅，保留 application-owned runtime。

快照携带 revision 和稳定 ID。日志只在新事件出现时发布；RTT 更新不重新复制全量配置，不触发无变化的托盘菜单重建。避免每次 render 把所有 500 条日志的所有字段反复 lowercase 及重复筛选；可以在数据版本或搜索词变化时更新检索结果。

上述缓存必须有容量、失效条件和 owner，不把“减少计算”变成多份永不释放的数据副本。

### 5.2 再控制峰值

限制握手、agent、key decode、channel-open、relay 的全局并发。测试启动 100 条定义时应看到受控排队，而不是瞬间创建所有高成本任务。普通 Stop 和审批命令不能等待所有这些任务完成。

不做未经测量的共享 SSH session 池。共享 session 会改变故障域、凭据/信任隔离和 Remote registration 所有权；首轮保留每条 Tunnel 自己的 chain，先让其正确、可测。

### 5.3 最后看框架边界

先比较 minimal window 与 full app，再考虑减少不需要的资源、渲染路径和依赖 feature。不要仅因 binary 大就断言运行内存高。若框架下限决定了无法显著低于 Loris，应如实接受这一事实或另做技术决策，不在本轮未经授权迁移 UI 技术栈。

## 6. 公平比较协议

### 6.1 固定条件

每次比较记录 Windows build、CPU/RAM、GPU/driver、屏幕分辨率/DPI、主题、显示器数量、应用版本/commit、编译 profile、二进制 hash、配置数量、SSH 服务端及网络条件。TW baseline、TW revised 和 Loris 选择固定 release 版本，不能 debug 对 release。

每组至少五次独立启动，交替运行顺序，前一个应用完整退出。SSH endpoints、认证、跳数、连接数、负载相同。对照使用单独测试配置和凭据，不修改用户真实隧道。

统计应用可归因的所有进程，而不是只取主进程；若某应用有子进程，把范围写清。GPU dedicated/shared 分开记；共享资源不能在多个进程之间重复相加成一个夸大总数。来源入口参考 [E09](13_SOURCE_INDEX.md#e09)，比较方案本身是本项目设计。

### 6.2 场景表

| 编号 | 场景 | 必要说明 |
|---|---|---|
| M00 | fresh tray，未创建过窗口，0 隧道 | 启动后固定预热与采样窗口 |
| M01 | 首次主窗口，0 隧道 | 同窗口尺寸/DPI/主题 |
| M02 | 打开后关闭，25 秒与 60 秒 | 不与 M00 混为一谈 |
| M03 | 10 条空闲隧道 | 三种模式分别及混合场景 |
| M04 | 固定 active relay 负载 | 相同吞吐、流数、包大小、方向 |
| M05 | 2/3 跳链 | 协议 session 数和业务流数均记录 |
| M06 | 网络黑洞/恢复/服务端重启 | 看峰值、恢复时间与残留 |
| M07 | 60 次窗口关闭/重开 | 第 1 次和预热后趋势分开 |
| M08 | 1000 条 stopped 配置 | 同时测搜索、选择、编辑、滚动 |
| M09 | 24 小时长稳 | 业务流验证、周期故障及资源趋势 |

不能在 Loris 不支持某个测试条件时仍宣称完全等价。记录不适用项，核心比较选双方都支持的共同场景，TW 特有测试单列。

### 6.3 原始采集

常规资源每秒采样，长稳可降到每分钟，同时记录关键事件时刻。CSV 至少包含 timestamp、run_id、phase、pid/process role、working_set_bytes、private_bytes、handles、threads、TCP 连接数、CPU 时间及可取得的 GPU dedicated/shared。

额外记录控制命令的 received/accepted/completed 时刻、连接 admission/open 时刻、frame time、测试流实际字节数与错误。采样器本身作为独立进程记录，不算入被测应用；不要在 GUI 热路径同步写每个数据包日志。

保留原始 CSV、stdout/stderr、exit code、配置摘要/脱敏 fixture、可执行 hash。当前仓库已有 probe 脚本，可先检查实际参数与实现后扩展；不能沿用一个只读 Working Set 的脚本却把结果命名为 Private Bytes。

## 7. 验收门槛

这些是建议的首轮预算，应在 WP0 基线测量后固定并由维护者评审；不能为了让测试通过而事后随意移动。

| 项目 | 起始门槛 |
|---|---|
| 普通控制命令 accepted | 正常/受控过载场景 p95 ≤ 100ms；极端调度噪声单列 |
| Stop | 共同清理预算内完成，初始建议 3s；需报告强制清理和未确认远端情况 |
| Quit | 全局预算初始建议 5s，不是 N 条隧道各等 5s |
| 60Hz 窗口操作 | 主要交互 frame p95 目标 <16.7ms；实际机器达不到时必须给 trace 而非编造结果 |
| 隐藏窗口 | 无持续 UI 动画/无效高频 render，隧道维护不受影响 |
| 稳态 Private Bytes | 预热后早/晚稳定窗口差不超过 max(10 MiB, 10% 基线)，且无无法解释的持续线性增长 |
| handles/tasks/connections | 循环后回到稳定范围，无随 generation 持续积累 |
| 同机 revised 对 baseline | 同场景内存/CPU >10% 回退须解释并评审；新监控负载单列开关对照 |
| revised 对 Loris | 只有共同场景数据齐全才给百分比；没有数据就写未测 |

稳定窗口应排除首次 shader/font/allocator 预热，但必须同时保留初始峰值，不能选择性删除不利样本。超过阈值是调查信号，不自动等于泄漏；低于阈值也不自动证明没有泄漏。结合 heap/handle owner 的存活证据判断。

## 8. 禁止的“优化”

不得调用 EmptyWorkingSet 或周期性 Working Set trimming 来美化数字；不得关闭安全校验、保留端口却谎报停止、删除监控功能来完成其验收；不得把 UI 移到隐藏 keeper window 假装它已被释放。

不得把所有异步状态包进一把 Arc<Mutex<_>>，不得无限 spawn 再指望 semaphore 在任务内部自然限流。不得把 `shutdown_timeout` 当作结束 blocking job；它只改变等待行为，具体约束见 [E06](13_SOURCE_INDEX.md#e06)。

不得宣称“同样是 Rust，所以肯定更快”，也不得从当前框架基线推导任何未测的 Loris 数字。最终性能报告需要数据，不需要技术栈优越性口号。
