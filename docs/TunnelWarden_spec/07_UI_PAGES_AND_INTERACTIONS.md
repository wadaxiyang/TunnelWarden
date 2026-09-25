# 07 · 页面、操作流程与状态交互

## 1. 导航与主工作区

保留 Overview、Jumpers、Tunnels、Groups、Logs、Settings 的产品内容。推荐将 Groups 变成隧道页的分组导航和管理面板，减少只为重命名组而切整页的成本；也可保留独立入口，但必须共用同一组操作。首次有配置时默认进入 Tunnels；空配置时展示可直接新建/导入的开始界面，不是只有统计数字的 Overview。

界面可保留英文首版，但文案集中管理，避免为了后续中文支持重写布局。`Jumpers` 页面中明确说明主机也可以是最终 SSH 目标，不能让用户误以为所有 SSH 主机都必须承担中转角色。

### 主界面线框（设计，不是当前截图）

```text
┌──────────────────────────── Native title bar ───────────────────────────┐
│ TunnelWarden    │ Tunnels        [ Search…           ] [Filter] [+ New] │
│                ├────────────────────────────────────────────────────────┤
│ Overview       │ All 8   Healthy 5   Needs attention 2   Stopped 1      │
│ Tunnels        ├───────────────┬─────────────────┬─────────────┬────────┤
│ Jumpers        │ Name / Mode   │ Route           │ State / RTT │ Action │
│ Logs           │ Lab proxy     │ 127…:1080 → SSH │ Healthy 38ms│ Stop ⋯ │
│                │ DB access     │ 127…:5432 → db  │ Reconnect   │ Stop ⋯ │
│ Groups         │ Web preview   │ Remote :8080    │ Blocked     │ Retry⋯ │
│  Research      │               │                 │             │        │
│  Development   ├─────────────────────────────────┴─────────────┴────────┤
│                │ Selected tunnel details / right inspector on wide UI │
│ Settings       │ Route · Health · Traffic · Recent errors · Actions     │
├────────────────┴───────────────────────────────────────────────────────┤
│ Core running     2 actions need attention           ↑ …      ↓ …        │
└────────────────────────────────────────────────────────────────────────┘
```

宽屏时详情在右侧；窄屏时进入详情面板，保留 Back 与选中项。不是同时堆表格、底部详情和右侧详情三份相同信息。

## 2. 隧道列表

列顺序为名称/模式、转发规则、SSH 链摘要、状态/RTT、动作。可选流量列只在宽度足够时显示。关键规则要包含源端点和目的端点，不能像当前实现一样只显示 local 字段，让 Remote 的方向难以判断。[S17](13_SOURCE_INDEX.md#s17)

示例采用脱敏数据，不作为生产默认配置：

```text
Lab SOCKS      SOCKS5    本机 127.0.0.1:1080 → bastion → lab
Database       Local     本机 127.0.0.1:5432 → db.internal:5432
Preview        Remote    服务器 127.0.0.1:8080 → 本机 127.0.0.1:3000
```

搜索匹配名称、主机名、端口、分组、描述，结果排序稳定。筛选包括模式、状态、分组、自动启动。排序可按名称、状态、最后状态变化、已测 RTT；未测 RTT 放末尾，不当作零延迟。RTT 自动更新不应在用户正操作行按钮时频繁重排行；实时排序采用明确启用/暂停策略。

单击行选择并展示详情，双击可打开详情/编辑，二者选择一种并保持一致。多选批量操作明确显示影响数量；批量删除先列出需要停止的隧道并确认。选择集合按 ID 保存，筛选变化不误作用于其他行。

## 3. 所有入口共用动作矩阵

| 当前状态 | 主动作 | 次级动作 | 不允许的误导 |
|---|---|---|---|
| Stopped | Start | Edit、Test、Duplicate、Delete | 显示已绑定/连接成功 |
| Queued | Cancel | 查看等待原因 | 冒充正在认证 |
| Connecting / Authenticating | Stop | 查看阶段 | 用禁用的开关代替取消 |
| AwaitingHostKey / AwaitingChallenge | Review / Respond | Stop | 自动同意或把 prompt 当网络超时 |
| EstablishingForward | Stop | 查看进度 | 注册前显示 Remote 已可用 |
| Healthy | Stop | Restart、Diagnostics、Edit | 将应用目标可用性与 SSH 健康混为一谈 |
| Degraded | Stop | Diagnose、Retry/Restart（按 core 契约） | 丢弃现有绑定信息 |
| Backoff / Reconnecting | Retry now | Stop、Edit | 把完整 delay 当剩余倒计时 |
| Blocked | Retry / Resolve | **Stop**、Edit、Diagnostics | 托盘发无效 Start；隐藏仍占用的端口 |
| Stopping | 显示停止进度 | 可明确选择清理后 Start | 旧 task 尚未退出就显示 Stopped |

UI 不自行从这些状态推导资源释放完成。动作由 core 提供支持范围或由共享纯映射层解释。Stop、Retry、Restart 是不同命令，不能用一个 Toggle 根据不完整 bool 猜测。[TW-006](02_DEFECT_REGISTER.md#tw-006)

按钮点下后先显示命令 Accepted，再由状态事件显示完成；队列满、runtime 不可用、配置冲突须明确显示失败并保持可重试。不能收到 click 就先把状态改绿。

## 4. 隧道详情 Inspector

从上到下显示身份与动作、运行状态、链路、端点、监控、最近事件。默认只展开关键信息，不把完整调试结构全部铺开。

运行状态包括用户期望、当前阶段、最近成功时间、重连次数、下一次重试时间和阻断原因。诊断折叠区可显示 generation/attempt，普通用户不必先理解这些字段。

链路按“本机 → 第 1 跳 → … → 最终 SSH 主机”显示，每跳有认证/信任状态和到该跳的 session RTT。错误标在真正失败的一跳；第 3 跳失败不能把前两跳画成未连接。不要相加 RTT，参见 04。

端点区分别显示请求绑定地址、已知实际端口、目的服务地址和监听归属。本地 listener 保留时明确“端口仍保留，当前新连接不可转发”。Remote 服务器最终公网暴露范围未确认时直说“按服务器策略生效，未核验实际暴露范围”。

流量区显示本次应用运行中的累计上下行、当前速率、活跃连接和最近 120 秒历史。暂停监控显示暂停，不填零曲线。最近错误显示阶段、原因、时间和可执行建议，能跳转到带筛选条件的 Logs。

## 5. Overview

Overview 是处理问题的入口，不是装饰性仪表板。顶部最多四个紧凑指标：正在运行、健康、需处理、已停止；下面优先列出等待信任、认证错误、端口占用和持续重连。每条可直接打开对应详情。

全局速率只在真实计数接通后显示；没有运行隧道时显示“暂无传输”。可展示最近状态变化列表，但来自有界事件日志，不以随机图表填空。点击指标跳转到对应筛选后的 Tunnels。

## 6. Jumpers / SSH Hosts

列表显示名称、user@host:port、认证方式、信任策略、被多少条隧道引用、最近一次连接测试及时间。Bypass 必须有长期可见的风险标识，不仅在编辑时出现一次提示。

创建/编辑可从列表进入。删除被引用主机时，先展示引用清单，提供替换引用或取消；不能静默删除造成多条配置失效。Duplicate 复制主机定义，密码仍按明确引用策略处理，不读取明文来复制。

Test Connection 针对当前草稿或保存配置，结果有 TestRunId 和时间。旧测试成功不能变成所有生产隧道 Healthy。取消测试必须取消其连接 owner。完整 Test 行为在 08。

## 7. Tunnel Editor

采用独立 draft，不实时改写已保存配置。编辑标题是“New Tunnel”或明确名称，保存按钮与取消按钮固定在面板底部。

```text
New Tunnel                                             [Close]
Name                 [ Lab proxy                         ]
Group                [ Research                         v]
Mode                 [ Local ] [ Remote ] [ SOCKS5 ]

Listen on this PC    [127.0.0.1          ] [1080           ]
SSH route            [Bastion] → [Lab]       [Add host…  v]
                      Move / Remove / Set final host

[ ] Start automatically when TunnelWarden starts
[ ] Start now after saving                         (new only)

Advanced ▸  Retry policy / timeout / description
──────────── validation or operation feedback ────────────────
[Test connection]                         [Cancel] [Save]
```

Local 显示本地监听和远端目的；Remote 显示远端监听和本机目的；SOCKS5 不显示无意义的固定目的字段。切换模式保留用户草稿但重新校验，不默默把 Remote 的源/目的解释反过来。

主机选择用可搜索弹层，而不是为每台主机渲染一个 Add 按钮。已选主机有序展示，提供拖动及上下移动按钮，标明最后一个为最终目标。超过 16 跳明确阻止。分组用 Select，自动启动用 Switch/Checkbox，三种模式用 segmented control。

高级项包括现有 retry policy、连接/探测相关说明和描述；不要擅自给每条隧道复制一份本应属于主机的参数。字段来源和覆盖优先级必须与 canonical 配置一致。

输入错误贴近字段；依赖性错误显示概要与定位。提交失败保留草稿。保存已提交但维护失败显示黄色维护提示，不回退表单到旧值。

## 8. Host Editor 与敏感输入

基本项为名称、主机、端口、用户名；认证方式用 Select 或紧凑分段选择。只展示选中认证方式所需字段。

Private Key 提供原生文件选择；口令使用 Keep/Replace/Remove 选择，再展示输入。Password 同理。Agent 展示自动选择方式及可选自定义 socket/pipe。Keyboard Interactive 展示其会在连接时请求响应，不预先收集一个“万能密码”。

安全高级项默认 Strict。选择 Bypass 要解释风险和作用范围，保存后列表仍显示危险标识。普通重试不能自动打开 Bypass。连接超时以秒为常用显示单位，必要时支持精度；数据层仍明确保存单位，避免毫秒/秒混乱。

输入内容不进入日志/提示词/遥测。窗口关闭或切换认证方式要正确释放不用的敏感草稿；不要为“方便恢复”永久保存所有曾输入密码。

## 9. 主机密钥与交互认证弹窗

弹窗显示隧道名称、跳数、连接地址、身份别名（若支持）、算法和可复制完整指纹。说明这是新主机还是密钥变更。焦点默认在 Cancel，回车不会自动信任；Changed 流程不能复用普通 Unknown 的一键保存。

动作可为“本次运行信任”“信任并保存”“取消连接”，具体信任范围必须与 core 一致。多条待确认项有可见队列；不弹出一连串无法定位的窗，也不把不同指纹合并。

Keyboard Interactive 每轮显示对应 echo 规则的输入，支持屏幕阅读和键盘提交；Stop、代号变化、超时使旧窗失效。服务端说明按不可信文本处理，不渲染可执行 HTML，不自动打开链接。

## 10. Logs

支持级别、来源、隧道/主机和文本**组合**筛选，默认展示本地日期时间并可切换 UTC。行包含时间、级别、主体、阶段、简短原因，详情展示完整安全错误链。

保留有界日志，加入事件序号和“较早事件已淘汰”提示。自动跟随仅在用户已在底部时生效，手动向上滚动暂停跟随。筛选/分页后若起始位置越界，应回到合法范围，而不是显示 `26–0 of 0`。

导出遵守长度、文件覆盖确认和脱敏约束。流式导出当前可访问的日志，不承诺能导出已经被淘汰的历史。上传云端不属于功能。

## 11. Settings 与导入

Settings 分为 Appearance、Startup & Tray、Connection Defaults、Diagnostics & Data。System/Light/Dark 要真实接通；Traffic Monitor 开关要实际控制采样/历史与呈现。登录启动显示“期望设置”和“系统应用状态”，注册表失败不能假装成功。

Native 配置与 OpenSSH 导入共用“选择文件 → 解析结果 → 冲突处理 → 安全差异 → 确认 → 保存结果”。安全差异必须单列 Bypass、非回环监听、SecretRef、auto_start 和登录启动，不能藏在折叠三条警告中。具体策略见 05。

遇到不支持指令可以继续导入可安全表达的部分，但被舍弃的认证/信任/路径/跳板语义须逐项显式确认。不能标为 Ready 后偷偷改为直连。

## 12. 托盘和窗口

保留当前 presentation 去重与最多 25 条入口；大配置通过“更多隧道”打开主窗，不构建千项原生菜单。[S21](13_SOURCE_INDEX.md#s21)

每项根据共享动作契约提供 Start、Stop、Retry，Blocked 不能发送无效 Start。显示需处理数量，区分健康与运行意图。托盘请求失败要能通知用户或记录可见错误。

关闭到托盘必须真正释放窗口级实体，不能保留大隐藏树。重新打开从 manager 获取最新 canonical 配置与快照。未保存草稿可询问是否放弃；允许取消关闭。应用 Quit 的核心清理由应用 owner 完成，不依赖窗口仍存在。

## 13. 键盘与异步竞态

| 快捷键 | 作用与边界 |
|---|---|
| Ctrl+N | 新建隧道；表单有脏数据时先保护草稿 |
| Ctrl+Shift+N | 新建主机 |
| Ctrl+F | 当前列表/日志搜索 |
| Ctrl+K | 命令面板；只展示当前合法动作 |
| Enter | 打开选中项详情；在确认框遵循安全默认焦点 |
| Ctrl+E / F2 | 编辑 / 重命名选中项 |
| Ctrl+D | Duplicate，不自动启动 |
| Delete | 请求删除并确认，不直接删除活动资源 |
| Ctrl+S | 仅在编辑上下文保存 |
| Escape | 先关闭最上层弹窗/菜单，再处理草稿；不直接杀隧道 |

保存与测试结果携带 operation_id、edit_session_id、base_revision、window_epoch。保存 A 期间打开 B，A 的回执只能更新 canonical store 和 A 的反馈，不能关闭 B、清空其输入或误绑定其测试结果。[TW-018](02_DEFECT_REGISTER.md#tw-018)

创建新编辑器、换页、关闭窗口、保存中再编辑分别测试。离开脏表单要有 Save/Discard/Cancel，Save 尚未确认不能丢弃 draft。后台配置发生变化则提示冲突，可重新载入或显式合并。

## 14. 必须交付的界面状态

实际截图至少覆盖空配置、健康列表、多跳详情、重连保留端口、Blocked 可 Stop、Unknown 信任、Changed 阻断、草稿校验失败、提交后维护警告、安全导入差异、流量开/关、日志筛选。两种主题均要可用，不能只把首页做漂亮。

每个页面必须有“无数据”“无搜索结果”“core 不可用”“保存中/失败”“内容较长”的完整处理。空状态给一个主要入口，不同时放六个同级大按钮。
