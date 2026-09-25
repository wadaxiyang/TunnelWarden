# 14 · 最终验收清单

**本清单初始状态全部未验收。** 文档写得完整、源码中已有测试或历史记录写 passed，都不能自动给这次整改打勾。

## 1. 验收对象

| 字段 | 值 |
|---|---|
| 审阅基线 | `71bf07c4c2185e8901166e2fbb2ccb8894effc78` |
| 整改 HEAD | 待填 |
| 被测 binary / SHA256 | 待填 |
| Cargo.lock / 工具链 / target | 待填 |
| Windows / GPU driver / DPI | 待填 |
| 原始测试与采样目录 | 待填 |
| 实施者 / 复核者 / 日期 | 待填 |

状态可用“未验收、进行中、已验证、条件风险已排除、不适用”。D/A/G 项不能仅因某次没复现就自动排除；R 项排除必须写测试条件和排除范围。不适用项须说明与项目明确范围的关系，不能用来删减原 v1 核心能力。

## 2. 26 项逐项关闭

证据列填写整改 commit、测试路径/命令、日志或截图位置，而不是写“应该没问题”。优先级不是 CVSS，证据类型定义见 02。

| 勾选 | 测试 / 问题 | 等级 | 验收对象 | 状态 | 证据与复核 |
|---|---|---|---|---|---|
| ☐ | T001 / [TW-001](02_DEFECT_REGISTER.md#tw-001) | P0 · D | 配置已经提交，却按失败回滚新凭据 | 未验收 | 待填 |
| ☐ | T002 / [TW-002](02_DEFECT_REGISTER.md#tw-002) | P1 · D | 固定临时文件会在崩溃后阻止再次保存 | 未验收 | 待填 |
| ☐ | T003 / [TW-003](02_DEFECT_REGISTER.md#tw-003) | P1 · D | 保存无关配置会重新启动用户手动停止的隧道 | 未验收 | 待填 |
| ☐ | T004 / [TW-004](02_DEFECT_REGISTER.md#tw-004) | P1 · D | 管理器等待 I/O 与逐个 Stop，导致控制面排队 | 未验收 | 待填 |
| ☐ | T005 / [TW-005](02_DEFECT_REGISTER.md#tw-005) | P1 · A | 正式状态机没有成为完整 SSH 生命周期的唯一真相 | 未验收 | 待填 |
| ☐ | T006 / [TW-006](02_DEFECT_REGISTER.md#tw-006) | P1 · D | Blocked 状态的托盘重试无效，界面缺少直接停止入口 | 未验收 | 待填 |
| ☐ | T007 / [TW-007](02_DEFECT_REGISTER.md#tw-007) | P1 · D | IPv6 本地监听通过校验却无法启动 | 未验收 | 待填 |
| ☐ | T008 / [TW-008](02_DEFECT_REGISTER.md#tw-008) | P1 · D | 外部配置的危险设置缺少分项授权 | 未验收 | 待填 |
| ☐ | T009 / [TW-009](02_DEFECT_REGISTER.md#tw-009) | P1 · D | SOCKS 对外暴露警告只检查一个字符串 | 未验收 | 待填 |
| ☐ | T010 / [TW-010](02_DEFECT_REGISTER.md#tw-010) | P1 · D | channel-open 和健康探测在 worker 主循环中串行等待 | 未验收 | 待填 |
| ☐ | T011 / [TW-011](02_DEFECT_REGISTER.md#tw-011) | P1 · R | 停止时限不一致，退出仍可能卡在同步 join | 未验收 | 待填 |
| ☐ | T012 / [TW-012](02_DEFECT_REGISTER.md#tw-012) | P1 · D | 主机密钥确认把整个握手窗口扩展到至少 120 秒 | 未验收 | 待填 |
| ☐ | T013 / [TW-013](02_DEFECT_REGISTER.md#tw-013) | P1 · G | 界面允许选择 Keyboard Interactive，但核心确定不支持 | 未验收 | 待填 |
| ☐ | T014 / [TW-014](02_DEFECT_REGISTER.md#tw-014) | P1 · G | 流量计数没有接到应用，且 worker 重建会换计数器 | 未验收 | 待填 |
| ☐ | T015 / [TW-015](02_DEFECT_REGISTER.md#tw-015) | P2 · G | 主题配置没有形成实际产品主题与设置入口 | 未验收 | 待填 |
| ☐ | T016 / [TW-016](02_DEFECT_REGISTER.md#tw-016) | P1 · D | OpenSSH 导入有语义偏差与静默近似 | 未验收 | 待填 |
| ☐ | T017 / [TW-017](02_DEFECT_REGISTER.md#tw-017) | P2 · G | 基础管理工作流缺少删除、复制、搜索和保存前测试 | 未验收 | 待填 |
| ☐ | T018 / [TW-018](02_DEFECT_REGISTER.md#tw-018) | P1 · D | 保存回调会关闭后来打开的编辑器，导航也会丢弃草稿 | 未验收 | 待填 |
| ☐ | T019 / [TW-019](02_DEFECT_REGISTER.md#tw-019) | P1 · D | 校验生成了规范化配置，却继续运行原始配置 | 未验收 | 待填 |
| ☐ | T020 / [TW-020](02_DEFECT_REGISTER.md#tw-020) | P2 · D | 日志只记录状态大类，丢失诊断上下文与中间事件 | 未验收 | 待填 |
| ☐ | T021 / [TW-021](02_DEFECT_REGISTER.md#tw-021) | P1 · R | 局部上限不能构成全局资源预算，阻塞工作也需限流 | 未验收 | 待填 |
| ☐ | T022 / [TW-022](02_DEFECT_REGISTER.md#tw-022) | P1 · R | 固定 russh 版本的 ping 可能把回执取消当成功 | 未验收 | 待填 |
| ☐ | T023 / [TW-023](02_DEFECT_REGISTER.md#tw-023) | P2 · G | 信任提示缺少隧道/跳数/代号，临时信任作用域不清楚 | 未验收 | 待填 |
| ☐ | T024 / [TW-024](02_DEFECT_REGISTER.md#tw-024) | P2 · G | 凭据编辑没有完整的清除/保留/轮换和回收语义 | 未验收 | 待填 |
| ☐ | T025 / [TW-025](02_DEFECT_REGISTER.md#tw-025) | P2 · G | 原 v1 的 inactivity timeout 没有明确配置与运行语义 | 未验收 | 待填 |
| ☐ | T026 / [TW-026](02_DEFECT_REGISTER.md#tw-026) | P2 · R | 手动 connect_stream 路径的 TCP_NODELAY 需要实证核对 | 未验收 | 待填 |

## 3. 生命周期硬门槛

- [ ] 一套权威 desired/runtime/listener/health/generation/attempt 契约接通真实工作路径；没有第二套能独立宣布健康的状态。
- [ ] Stop 后没有旧 attempt 自动复活；无关保存/网络恢复不推翻手动停止；Stopping 中的最终用户意图正确。
- [ ] Stopped 没有未回收 listener/channel owner；有异常强制清理或远端未确认时如实记录。
- [ ] transient reconnect 保留本地 listener；恢复后新建数据流可通，未承诺旧 TCP 流无缝恢复。
- [ ] StopAll/Quit 使用共同预算，慢配置/keyring 不阻塞所有控制命令；blocking job 未被假称已 abort。
- [ ] 全部 task、订阅、timer、queue、cache、native handle 有可核对 owner/上限/结束路径。

## 4. 数据、安全与完整功能

| 勾选 | 能力 | 必须由什么证明 |
|---|---|---|
| ☐ | Local / Remote / Dynamic | 真正数据传输、错误、取消及 IPv6/DOMAIN 回归 |
| ☐ | 多跳 | 2/3 跳成功与中间失败，逐跳身份与错误 |
| ☐ | Password / Key / Agent | 真正认证及拒绝/无身份/口令错误 |
| ☐ | Keyboard Interactive | 多轮、echo、取消、脱敏 |
| ☐ | Host Key | Unknown/Changed 分离、完整提示、安全保存 |
| ☐ | 外部监听 | core 授权与 IPv4/IPv6/non-loopback 覆盖 |
| ☐ | 配置事务 | post-commit 维护失败不删有效凭据，crash 可恢复 |
| ☐ | 路径与版本 | canonical save/load/test 一致，revision 冲突正确 |
| ☐ | 凭据生命周期 | Keep/Replace/Remove，GC 保护备份和运行引用 |
| ☐ | Native import/export | 敏感差异、portable/local 区分，不执行导入授权 |
| ☐ | OpenSSH import | 支持语义正确，未映射项明确，不执行命令 |
| ☐ | SSH 命令导入（本轮增量） | -L/-R/-D 多项预览、安全解析、未知项处理 |
| ☐ | Connection Test | 隔离 owner，不污染生产健康，不抢生产端口 |
| ☐ | Delete / Duplicate / Search | 真正接线、ID 稳定、冲突清楚、删除先停 |
| ☐ | Groups | CRUD、移动、排序、组启停不破坏运行意图 |
| ☐ | Traffic | 方向、累计、速率、120 样本、开关、跨重连 |
| ☐ | RTT / Health | 当前 session 成功证据、逐跳、过期提示 |
| ☐ | Logs / Diagnostics | 阶段/主体/错误、序号、有界、组合筛选/导出 |
| ☐ | inactivity / retry | 明确语义和真实配置，默认空闲不误杀 |
| ☐ | Theme / Settings | System/Light/Dark 真实生效，设置无死字段 |
| ☐ | Tray / Startup | Blocked 动作、队列失败反馈、实际登录 |

“本轮增量”与原 v1 需求分开标注，只能由维护者明确决定该增量是否纳入本轮发布；不得以此为理由删掉原规格功能。若增量不做，在最终发布说明中明确它不在本次范围，而非称工作流完全对齐。

## 5. UI 验收

- [ ] 真实应用中主表、详情、编辑、日志、设置、导入均完成，不是只有首页好看。
- [ ] 深/浅主题、窄工作区、100/125/150/200% DPI、长名称/路径、0/1000 项均可用。
- [ ] Blocked 可 Stop，Reconnecting 的 listener/倒计时真实，Remote 绑定信息不会因 Degraded 消失。
- [ ] 安全弹窗默认取消，指纹可复制，旧 prompt 无效；错误不是只有一条底部红字。
- [ ] 脏草稿保护、保存 A 不关闭 B、旧 TestRun 不污染新草稿、revision conflict 完整。
- [ ] 键盘、焦点、tooltip、选择、虚拟列表稳定 ID 和可访问名称完成实际检查。
- [ ] 图表来自真实监控，无随机/假数据；生产无仅为演示的成功分支。
- [ ] 窗口关闭释放 UI owner；重新打开从最新 core/config 获取状态，无 hidden keeper tree。

## 6. 性能与平台证据

- [ ] 记录完整基线条件与 release binary hash，不用 debug/release 混比。
- [ ] 分别记录 WS、Private、handles、threads、CPU、GPU dedicated/shared；无 Working Set trimming。
- [ ] M00–M09 实际可执行场景按 09 完成；异常/不适用场景附理由与替代证据。
- [ ] 60 次窗口循环、重连压力、全局资源预算和大配置列表无持续未解释增长。
- [ ] 实际 IPv6、Wi-Fi/VPN、睡眠/恢复、服务器重启、Windows 登录通过。
- [ ] 24 小时真实长稳完成，数据正确性和资源日志齐全；smoke 没有冒充长稳。
- [ ] 比较 Loris 的结论仅使用同机共同场景完整数据；未测不写优势百分比。

## 7. 依赖与发布

- [ ] fmt/test/clippy/build 与当前 HEAD 一致；保存全部命令退出码和日志。
- [ ] 新鲜 cargo audit 与 Windows target 依赖树已检查；所有告警/例外可追溯，无全局一键忽略。
- [ ] russh/GPUI 最小补丁有版本、来源、回归与撤销条件；既有 vendor 修复未被回退。
- [ ] schema/凭据/备份回滚策略经过验证；发布包、许可证、hash、已知限制齐全。
- [ ] 维护者确认关键缺陷与原 v1 必要能力均关闭；没有用“以后优化”隐藏必需功能。

## 8. 最终签收摘要模板

```text
整改版本 / binary hash

确定缺陷关闭数：__ / 14
架构偏差关闭数：__ / 1
功能缺口关闭数：__ / 7
条件风险完成验证数：__ / 4

原 v1 必要功能：通过 / 未通过（列项）
UI：通过 / 未通过（证据）
24h 长稳：通过 / 未通过 / 未执行
Windows 真实平台：通过 / 未通过（列项）
依赖审计：结论与例外
性能：基线、修改版、Loris（未测则写未测）

尚未关闭的 gate：
发布决定及复核者：
```

没有足够证据时维持未验收，并明确缺的是代码、运行环境还是结果复核。本包到此结束；清单的空格只能由后续真实执行填入。
