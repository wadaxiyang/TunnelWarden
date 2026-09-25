# 01 · 审计基线、证据边界与总判断

## 1. 对象

仓库为 `wadaxiyang/TunnelWarden`，本次固定审阅 `71bf07c4c2185e8901166e2fbb2ccb8894effc78`。核心依赖固定为 `russh =0.63.3` 与 `gpui-kit =0.6.6`；应用自建一个 Tokio runtime，设置两个 worker。不要用操作系统看到的全部线程数倒推“Tokio 起了几十个 worker”。[S28](13_SOURCE_INDEX.md#s28)、[S29](13_SOURCE_INDEX.md#s29)、[S20](13_SOURCE_INDEX.md#s20)

源码通过 GitHub 连接器逐文件读取。完整范围和分段范围见 [证据索引](13_SOURCE_INDEX.md)。未获得 Windows 可运行桌面环境，未独立获取验证记录中的原始资源 CSV；没有做依赖全量审计、模糊测试、实际网络攻击测试或当前界面的像素级截图评审。

## 2. 实际结构

```text
main / DesktopShell
  ├─ Workspace              页面、草稿、快照订阅
  ├─ TrayController         菜单、状态摘要、动作
  ├─ InstanceGuard          再次启动唤醒窗口
  └─ RuntimeHost            单独应用级运行时，不依附窗口
       └─ TunnelManager     配置、命令、任务集合、快照、审批、日志
            ├─ LocalForwardSupervisor / RemoteForwardSupervisor
            │    ├─ LocalForwardWorker / RemoteForwardWorker
            │    ├─ SshChain → DirectSshSession → russh
            │    └─ relay / SOCKS5 / counters
            └─ ListenerRuntime → StateMachine
```

关键偏差不是“完全没有状态机”，而是 StateMachine 目前主要驱动监听器生命周期，连接/健康/退避另用 SupervisorState。已有 generation 模型不能直接视作完整 SSH 生产路径的形式保证。[S03](13_SOURCE_INDEX.md#s03)、[S04](13_SOURCE_INDEX.md#s04)、[S05](13_SOURCE_INDEX.md#s05)、[S06](13_SOURCE_INDEX.md#s06)、[S08](13_SOURCE_INDEX.md#s08)

## 3. 已做对的部分，不许整改时破坏

| 当前能力 | 证据 | 整改中应保持 |
|---|---|---|
| 应用级 runtime 独立于窗口 | [S19](13_SOURCE_INDEX.md#s19)、[S20](13_SOURCE_INDEX.md#s20) | 关闭窗口到托盘后网络仍工作；不能靠隐藏 keeper window 保活 |
| 真实 russh 三种转发与多跳 | [S06](13_SOURCE_INDEX.md#s06)、[S07](13_SOURCE_INDEX.md#s07)、[S08](13_SOURCE_INDEX.md#s08)、[S09](13_SOURCE_INDEX.md#s09)、[S10](13_SOURCE_INDEX.md#s10)、[S11](13_SOURCE_INDEX.md#s11) | 复用已有数据面；不能退回 mock 或外部 ssh.exe |
| Strict 主机密钥路径 | [S12](13_SOURCE_INDEX.md#s12) | 未知密钥经确认，变化密钥阻断；不因认证失败自动 Bypass |
| 有界队列、连接与日志 | [S03](13_SOURCE_INDEX.md#s03)、[S07](13_SOURCE_INDEX.md#s07)、[S09](13_SOURCE_INDEX.md#s09)、[S12](13_SOURCE_INDEX.md#s12) | 保留限额，再补全局预算；不把已有有界结构说成无界泄漏 |
| 本地 listener 在短时重连期间保留 | [S06](13_SOURCE_INDEX.md#s06)、[S07](13_SOURCE_INDEX.md#s07) | 普通网络重连不能让别的进程抢走本地端口 |
| relay half-close 与固定缓冲 | [S26](13_SOURCE_INDEX.md#s26) | 一方向 EOF 不应错误截断另一方向回包；不为减少代码改成粗暴同时关闭 |
| SOCKS 域名转交远端解析 | [S25](13_SOURCE_INDEX.md#s25)、[S07](13_SOURCE_INDEX.md#s07) | 不在本地悄悄解析 domain；成功回复要在 channel 建立后 |
| 密码在系统 keyring 中，配置只保留引用 | [S16](13_SOURCE_INDEX.md#s16)、[S18](13_SOURCE_INDEX.md#s18)、[S13](13_SOURCE_INDEX.md#s13)、[S14](13_SOURCE_INDEX.md#s14) | 不向 TOML、日志、截图测试夹具写真实密码 |
| 托盘 presentation 去重 | [S21](13_SOURCE_INDEX.md#s21) | 已避免每次 RTT 更新都重建菜单，不要把这当作尚未修复的缺陷 |
| 已有真实测试及 Windows 资源修复记录 | [S27](13_SOURCE_INDEX.md#s27) | 不删回归；特别是 vendor drop-target 修复需保留并单独追踪 |

## 4. 四类结论的阅读方式

**确定代码路径问题**可以由调用顺序或类型处理直接推出，但本次没有动态复现。例如提交成功后清理报错再删除凭据，以及 `::1` 拼为无方括号 socket 字符串。

**架构偏差**指实际数据流没有落实原约束。它是整改理由，不等于已经证明某个竞争条件必然发生。

**功能缺口**指源码没有完整接线或实现。例如可选择但必然被拒绝的 Keyboard Interactive，和未发布到 GUI 的流量计数器。

**条件风险**需要真实网络或故障注入才能知道影响频率、持续时间与严重度。包括退出卡住、全局资源峰值、ping 回执取消和 TCP_NODELAY 生效情况。

## 5. 当前产品判断

后端已有可保留的工程基础，不适合整个推倒。短板集中在异常路径、配置一致性和控制语义。UI 则尚未把这些能力组织成能快速判断、定位、操作的工作台。仅仅换色、加阴影或把按钮变圆，无法解决没有详情、没有完整诊断、没有搜索/复制/测试、Blocked 难以操作的问题。[S17](13_SOURCE_INDEX.md#s17)、[S18](13_SOURCE_INDEX.md#s18)、[S03](13_SOURCE_INDEX.md#s03)

项目应继续使用现有 Rust + GPUI Kit。是否更省内存必须靠对照，而不是语言推论。已有记录没有支持“持续线性泄漏仍未解决”这一结论，也没有支持“已在同条件下胜过 Loris”这一结论。[S27](13_SOURCE_INDEX.md#s27)

## 6. 本轮禁止作出的结论

不得说“整个项目逻辑已经正确”“Rust 所以无安全漏洞”“全部测试已由本次通过”“已实测 UI 比 Loris 好”“整改必然降低到 30 MB”“所有显示 Healthy 的状态都是假的”。这些结论超出了已取得的证据。

可说的结论是：本包识别了 26 个有定位、有修复要求、有验证出口的整改事项，并给出了保持完整 v1 范围的实施路径。
