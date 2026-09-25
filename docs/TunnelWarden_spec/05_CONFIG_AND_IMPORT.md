# 05 · 配置提交、凭据一致性、崩溃恢复与导入

主关联 [TW-001](02_DEFECT_REGISTER.md#tw-001), [TW-002](02_DEFECT_REGISTER.md#tw-002), [TW-003](02_DEFECT_REGISTER.md#tw-003), [TW-008](02_DEFECT_REGISTER.md#tw-008), [TW-016](02_DEFECT_REGISTER.md#tw-016), [TW-018](02_DEFECT_REGISTER.md#tw-018), [TW-019](02_DEFECT_REGISTER.md#tw-019), [TW-024](02_DEFECT_REGISTER.md#tw-024)。证据为 [S03](13_SOURCE_INDEX.md#s03)、[S13](13_SOURCE_INDEX.md#s13)、[S14](13_SOURCE_INDEX.md#s14)、[S15](13_SOURCE_INDEX.md#s15)、[S16](13_SOURCE_INDEX.md#s16)、[S17](13_SOURCE_INDEX.md#s17)、[S18](13_SOURCE_INDEX.md#s18)。

## 1. 修复目标

任何时刻，用户必须能区分“没有保存”“保存已提交但附带维护警告”“提交结果需恢复确认”。磁盘配置、内存 canonical 配置、系统凭据引用和系统启动设置不能因为同一个笼统 Result<(), Error> 进入相互矛盾的状态。

配置仍使用 TOML，不引入数据库。保留导出不包含明文密码、文件长度限制、数量上限和备份机制。

## 2. CanonicalConfig 是唯一有效配置

所有输入统一走 `RawDocument/Draft → validate_and_normalize → CanonicalConfig`，包括 GUI 保存、文件导入、ssh_config 导入、草稿测试与启动重载。

校验函数必须返回最终采用的对象，不可只检查 Err 然后继续使用原始草稿。规范化至少处理路径展开、IP/端口角色、ID 引用、跳链、策略与持续时间边界。保留原始显示路径时，明确其与 runtime path 的区别。

UI 通过 config watch 接收已提交的 canonical snapshot，不在回调中把自己早先克隆的 DomainConfig 当新真相。保存命令携带 base_revision，版本落后则返回 Conflict 及当前 revision，不能 silently last-write-wins。

## 3. 提交协议

建议的数据契约如下，只是模型，不是现有 API。

```rust
enum SaveOutcome {
    NotCommitted { error: SaveError },
    Committed { receipt: SaveReceipt, warnings: Vec<MaintenanceWarning> },
    Indeterminate { operation_id: OperationId, recovery_required: bool },
}
struct SaveReceipt {
    operation_id: OperationId,
    committed_revision: ConfigRevision,
    canonical_config: CanonicalConfig,
}
```

返回值不能把“提交后 pruning 失败”表示为 NotCommitted。warnings 必须有上限，UI 能查看详情；不是无限堆积的字符串。

### 事务步骤

| 步骤 | 动作 | 失败/崩溃策略 |
|---|---|---|
| A | 在单写者/配置目录锁下检查 base_revision，验证并规范化输入 | 不修改磁盘和 runtime |
| B | 需要新密码时写入全新随机 SecretRef，旧引用保持不动 | 提交前失败可删除这个明确新建的引用；不能删除原引用 |
| C | 在同目录创建唯一 tmp，限制序列化大小，写入并 sync | 未提交；重启后依据事务记录/命名与写锁识别孤立 tmp |
| D | 按已有配置制作可恢复备份 | 失败则不替换主配置；保留必要凭据引用 |
| E | 原子替换主配置，确定提交 revision | **提交点**；之后不能再走“删除新凭据”的失败回滚 |
| F | 将 canonical snapshot 应用到内存，按当前 DesiredState 计算 runtime effects | 已保存但运行应用失败要单独显示，不谎称保存失败 |
| G | 清理旧备份、回收无引用凭据、同步系统 Run 设置 | 非致命维护结果；失败可重试，不能回滚已提交主配置 |

文件系统的原子替换和掉电持久化不是同一保证。平台实现需要明确实际 API、同卷限制、flush 行为和 Windows sharing violation 重试。不要为了“兼容 Windows”退化成先删除 config.toml 再写新文件；也不要笼统声称 std::fs::rename 在 Windows 一定不能覆盖。

在 E 与 F 之间进程退出时，下一次启动应读取已提交文件，恢复成一致的 canonical 配置。worker panic 或结果通道丢失导致无法知道是否提交时，归类 Indeterminate，先核对磁盘 revision/内容，绝不能盲删 SecretRef。

## 4. 系统登录启动设置

原逻辑把注册表修改和文件保存编成一段容易误回滚的操作。新方案把“用户期望设置”保存在已提交配置，把“OS 实际应用结果”作为独立状态。提交后调用幂等 reconciler；失败显示“配置已保存，登录启动设置未应用”，允许重试。

这避免必须对 keyring、文件系统和注册表进行并不存在的跨系统原子事务。退出/重开后继续对账，不是启动时默默把错误丢到 stderr。导入外部配置默认不修改系统设置。

## 5. 崩溃恢复与多实例写锁

临时文件使用本程序专用前缀 + transaction ID，且在主文件目录内。所有写入获得配置目录级锁；Windows GUI 的 Local 命名 mutex 不是跨会话文件事务锁的替代。

启动恢复顺序为：锁定配置目录；读取并验证主文件；识别本程序已结束事务遗留；主文件有效时优先采用主文件；主文件无效时提供明确的备份恢复预览；验证候选再恢复；最后清理确认孤立的临时文件。不能看到 tmp 就覆盖主文件，也不能删除用户目录所有 .tmp。

主文件缺失、损坏、未来 schema、不支持字段、只读目录、磁盘满、备份清理失败、替换 sharing violation、进程被终止都必须有测试。正常已有主配置时恢复流程不能令应用自动重建空配置并覆盖用户数据。

## 6. 凭据保留与回收

维护本程序创建的引用登记，至少能关联事务与当前/保留备份引用。需要保留的集合为：当前配置、保留备份、尚未完成的写事务、仍在清理且需要该引用的运行/诊断任务。只回收已确认不在集合中的本程序条目。

提供 Keep、Replace、Remove 三态。Remove 后规范化配置不能留下悬空引用。替换已提交但清理旧凭据失败是警告；不能为清理失败撤销新密码。恢复旧备份时凭据可能已不存在，必须显示“需要重新输入”，不能声称导出文件能跨机器带回密码。

导出分两种明确语义：普通可分享配置默认移除本机凭据绑定并提示重新关联；本机备份可保留 opaque refs，但说明它们不是秘密值、不是跨机可移植凭据。两个模式均不导出密码/口令。

## 7. 外部配置导入计划

导入分为 Parse、Normalize、Diff、Policy review、Confirm、Commit、可选 Run，不把七步混成一个确认按钮。

`ImportPlan` 展示新增/修改/删除的主机、分组、隧道，ID 冲突策略，引用缺失，目标监听范围，host-key policy，凭据来源，立即运行与应用设置变化。支持 merge 和 replace，replace 必须显示将删除的当前对象及影响运行中的隧道。

默认策略：不立即启动，不修改登录启动，不接受导入文件声称的暴露授权，不静默启用 Bypass，不自动关联本机 SecretRef。可先把条目导入为“待配置”；不要用假凭据字符串绕过 schema 验证。为未绑定凭据定义合法的 draft/incomplete 状态，或者暂不提交该条目并清楚列出阻塞原因。

受信本机备份恢复可以由用户选择保留引用和运行偏好，但仍需展示差异。确认时带 base_revision，预览与提交之间配置变化则重新生成 Diff，不拿过期预览强行覆盖。

## 8. OpenSSH 导入

当前 parser 有限支持是已有功能，不应删掉。但其可映射子集必须准确，无法映射时要明示，尤其不能把含 ProxyJump 的最终主机直接当成无需跳板的直连配置。[S15](13_SOURCE_INDEX.md#s15)

OpenSSH 手册明确相对 Include 的基准与条件作用域，ServerAliveInterval 的 0 值代表不发送对应保活消息。修复这两类语义偏差，不能用 max(1) 静默改变含义。[E05](13_SOURCE_INDEX.md#e05)

### 求值策略

采用成熟 parser 的适配层，或经过夹具覆盖的自有 parser；选用库前先验证 Windows 路径、Include 和原始来源能力，不能仅为了依赖名称符合旧 spec 就盲换。

解析结果带文件、行号、原字段和值、目标字段以及 Unsupported/Conflict/Unsafe 等诊断。Host/Match 的条件作用域和 Include 展开不是简单全局文本拼接；重复 Include 的上下文与递归环要分开处理。按实际读取量限制字节数，限制文件数量/递归深度，拒绝非普通文件，外部/网络路径需要明确授权。

scalar first-wins 与列表累加分别实现。多个 IdentityFile、IdentitiesOnly、HostKeyAlias、UserKnownHostsFile 不支持完整语义时，不得静默丢弃然后给出绿色“等价导入”。ProxyJump 需解析 alias、user、port、多跳，解析后检测循环/缺失和最大 16 跳，预览整条链。

产品进程绝不执行 ProxyCommand、LocalCommand、Match exec、shell 展开或系统 ssh。可为可信测试夹具使用 OpenSSH 作为外部比较基准；这不等于允许产品为用户未知文件执行这些命令。

### 必备夹具

包含具体 Host 与通配 Host、否定模式、Include 相对/绝对/嵌套/重复/循环、带空格 Windows key path、Port=22/Port = 22、未闭合引号、多个 IdentityFile、保活 0、未知 User、缺失跳板、HostKeyAlias、多个 known_hosts、Unsupported directive 和超长文件。

## 9. Schema 演进

新增规范化、信任策略、inactivity 或 credential state 需要 schema v2 时，提供显式 v1→v2 迁移、迁移前备份和 round-trip 测试。未来 schema 只读报错，不擅自覆盖。原来 local/remote 字段的含义不变；新类型通过适配层映射。

旧二进制回退可能无法读取 v2。文档应指导恢复兼容备份，而不是声称更换 exe 即可无条件回滚。不能把未来字段全部忽略来“提高兼容性”，从而误忽略安全策略。

## 10. UI 保存事务

Editor 保存时只提交自己的 EditSessionId 与 base_revision。成功后更新核心配置缓存；只关闭仍属于该 EditSessionId 的编辑器。用户后来打开的其他编辑器不受影响。保存失败保留草稿并定位字段/事务阶段。

重复保存合并/拒绝有明确结果；全局只允许一个配置写事务处于 commit 序列，多个窗口草稿可并存但需冲突处理。取消编辑器不是取消一个已经到提交点的事务；UI 必须区分“关闭草稿”和“撤回尚未提交的保存请求”。
