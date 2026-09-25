# TunnelWarden 整改文档包

**基线提交**　`71bf07c4c2185e8901166e2fbb2ccb8894effc78`  
**日期**　2026-09-25  
**交付性质**　基于源码的审阅结论、修复规格、UI 重构规格与验收方案。**不是已实施的补丁，也不是已通过的测试报告。**

## 先看结论

TunnelWarden 已经有真实 SSH、三种 TCP 转发、多跳、严格主机密钥检查、资源上限和实际测试基础，不应推倒重写。当前最重要的缺陷是配置提交一致性、手动停止语义、控制面响应、IPv6 地址处理以及 UI/托盘状态接线。[S03](13_SOURCE_INDEX.md#s03)、[S06](13_SOURCE_INDEX.md#s06)、[S09](13_SOURCE_INDEX.md#s09)、[S10](13_SOURCE_INDEX.md#s10)、[S14](13_SOURCE_INDEX.md#s14)、[S24](13_SOURCE_INDEX.md#s24)

UI 的主要问题不是缺一个好看的颜色，而是仍以零散表单、长按钮列表和简单状态文字组织产品。整改方向是 **Loris 的任务组织方式 + 原创 Zed 风格的原生控制台**，以“隧道列表、当前状态、链路详情、诊断与操作”作为视觉中心。保留 GPUI Kit，不引入 WebView，也不抄 Loris 的品牌和代码。[S01](13_SOURCE_INDEX.md#s01)、[S17](13_SOURCE_INDEX.md#s17)、[E01](13_SOURCE_INDEX.md#e01)、[E02](13_SOURCE_INDEX.md#e02)

内存没有证据支持“改完 UI 就显著低于 Loris”。仓库记录表明，完整窗口已经接近其最小 GPUI 窗口的占用量级；业务层应先消除增长与无效工作，再按同机、同场景、同进程范围比较。[S27](13_SOURCE_INDEX.md#s27)

## 包含什么

| 文档 | 用途 |
|---|---|
| [01 · 审计基线](01_AUDIT_BASELINE.md) | 已检查什么、哪些能力要保留、哪些结论没有实测支持 |
| [02 · 26 项登记册](02_DEFECT_REGISTER.md) | 每项的证据、触发条件、影响、修复与回归 |
| [03 · 核心生命周期](03_CORE_LIFECYCLE.md) | 唯一状态机、generation/attempt、Stop/Restart、重连与关闭 |
| [04 · SSH / 转发 / 安全](04_SSH_FORWARDING_SECURITY.md) | 信任、认证、端点、SOCKS、反向转发、并发与超时 |
| [05 · 配置与导入](05_CONFIG_AND_IMPORT.md) | 提交点、凭据事务、崩溃恢复、规范化与安全导入 |
| [06 · UI 设计系统](06_UI_DESIGN_SYSTEM.md) | 布局、颜色、字级、尺寸、组件、渲染边界 |
| [07 · 页面与交互](07_UI_PAGES_AND_INTERACTIONS.md) | 各页线框、全部状态动作、表单、弹窗和键盘规则 |
| [08 · 功能补齐](08_FEATURE_COMPLETION.md) | 现有能力与 v1 缺口、Connection Test、监控、CRUD、命令导入 |
| [09 · 内存与性能](09_MEMORY_AND_PERFORMANCE.md) | 已有记录、归因、预算、同机对照及禁止的“优化” |
| [10 · 测试与发布](10_TESTS_AND_RELEASE.md) | 26 项回归和跨模块测试、人工测试、长稳与发布门槛 |
| [11 · 实施顺序](11_IMPLEMENTATION_PLAN.md) | 小批次改动、依赖关系、目标文件与每包退出条件 |
| [12 · Agent 执行提示词](12_AGENT_PROMPTS.md) | 总入口及逐工作包可直接使用的指令 |
| [13 · 证据索引](13_SOURCE_INDEX.md) | 固定提交源码链接与外部一手资料 |
| [14 · 验收清单](14_ACCEPTANCE_CHECKLIST.md) | 逐项签收模板；初始均未验收 |

## 优先处理

先处理 [TW-001](02_DEFECT_REGISTER.md#tw-001) 的提交后误回滚，再处理 [TW-002](02_DEFECT_REGISTER.md#tw-002)、[TW-003](02_DEFECT_REGISTER.md#tw-003)、[TW-007](02_DEFECT_REGISTER.md#tw-007)、[TW-019](02_DEFECT_REGISTER.md#tw-019)，用确定的回归保护原行为。随后将真实 supervisor 与权威状态机接通，解决控制面等待、Blocked 操作和统一停止期限。UI 可以按已冻结的快照契约并行开发，但不能自己推断连接健康。

26 项中，14 项是静态代码路径问题，1 项是架构偏差，7 项是功能/接线缺口，4 项是需要运行测试确定影响的条件风险。优先级不是漏洞利用评级，不能把它们宣传成“26 个已证实的安全漏洞”。

## 交给开发代理的方法

先读本文件、01、02、11、12，再读当前仓库 AGENTS.md 和各工作包要求的源文件。从 WP0 开始，每包提交实际 diff、执行命令、真实测试结果、未完成项和资源影响。本文中的 Rust/事件示意是设计契约，不是已经编译过的库 API 示例。

推荐将本目录放入 `docs/remediation/`。不要用它覆盖根 AGENTS.md、原规格或历史验证记录。执行时若 HEAD 已变化，先核对每一项是否仍成立，保留已正确修复的代码，避免回退现有 Windows drop-target 修复。

## 不变的边界

一次公开 v1 必须包含 Local、Remote、Dynamic SOCKS5、多跳、导入、健康、流量、托盘及可靠性验证。工作包是开发顺序，不是删减功能的理由。不增加终端、SFTP、Docker、AI Debug、云同步、账号、遥测或付费开关。[S01](13_SOURCE_INDEX.md#s01)、[S02](13_SOURCE_INDEX.md#s02)

本次没有修改远端仓库，没有运行 Windows GUI、cargo test、cargo audit 或长稳，也没有测量 Loris。后续验收结果必须由实际执行填入，不得把本包中的目标数值或步骤填写为“已经通过”。
