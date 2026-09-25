# 04 · SSH、转发与安全边界整改

本章既包含确定修复，也包含加固和待测条件风险。登记依据见 02，不能把本章每条建议都宣传成当前已有的可利用漏洞。主要证据为 [S07](13_SOURCE_INDEX.md#s07)、[S09](13_SOURCE_INDEX.md#s09)、[S10](13_SOURCE_INDEX.md#s10)、[S11](13_SOURCE_INDEX.md#s11)、[S12](13_SOURCE_INDEX.md#s12)、[S16](13_SOURCE_INDEX.md#s16)、[S24](13_SOURCE_INDEX.md#s24)、[S25](13_SOURCE_INDEX.md#s25)、[S26](13_SOURCE_INDEX.md#s26)。

## 1. 威胁边界

保护对象为本机凭据、私钥使用权限、监听范围、用户运行意图、连接数据和应用可用性。输入来源包括 SSH 服务器/网络、SOCKS 客户端、用户导入的配置、系统 keyring/文件错误与 UI 并发操作。

默认用户可以信任其自己显式设置的目标，但**导入数据不是运行授权**。同一用户已能完全控制进程和 keyring 的场景不应被夸大成新增远程漏洞。反过来，恶意网络或配置不能因为 Rust 类型安全就被忽略。

## 2. 端点与监听范围

### 2.1 结构化地址

Local/Dynamic 的本地 bind 采用 `IpAddr + u16`，通过 `SocketAddr::new` 构造。序列化使用裸 IP，展示/命令预览对 IPv6 加方括号。SSH 目标主机及 Local 的远端目标允许主机名；不能把目标主机名错误限制为本机 IP。

Remote 的本地字段是本机侧目标服务，远端字段才是服务端 listener。建议为业务层引入明确的 `LocalBind`、`RemoteBind`、`ForwardTarget` 类型或命名访问器，避免继续让 local/remote 字段名称制造方向混淆。持久化 schema 可经迁移保持兼容，不能直接颠倒旧文件含义。

端口 0 的语义必须按角色定义：当前 Remote bind 可请求服务端分配端口，目标服务端口不可为 0。不要仅因解析成 u16 就接受所有端点。Local/Dynamic 是否支持随机本地端口应明确；本轮按现有规格保持固定非零端口即可，不额外暗加支持。

### 2.2 ExposurePolicy

地址先规范化再判断 `is_loopback`，覆盖整个回环范围及 IPv6。非回环不等于必然公网可达，但意味着暴露范围扩大；提示措辞应准确。

| 场景 | 默认 | 用户必须知道 |
|---|---|---|
| Dynamic 回环 | 允许 | 仅本机，SOCKS NO AUTH |
| Dynamic 非回环 | 未授权不启动 | 同网络可达设备可能使用无认证代理；必须显式确认 |
| Local 非回环 | 需展示并确认暴露范围 | 其他设备可能访问被转发的内部服务 |
| Remote 非回环 | 需确认请求范围 | 服务端 GatewayPorts/策略影响实际绑定；客户端请求值不是实际暴露审计结果 |
| 外部导入暴露配置 | 只导入定义 | 不能继承导入文件内的“已同意”标记 |

授权与 TunnelId、相关端点和配置 revision 绑定；改变监听范围使旧授权失效。可以设计为本地配置中的显式政策字段，但外部导入必须重新确认，不能信任文件自带 consent。不要为了方便自行改系统防火墙、要求管理员权限或杀掉占端口进程。

端口错误分别呈现 AddrInUse、PermissionDenied/系统错误码和无效地址。可提供只读进程定位；无法识别所属进程时明确“未知”，不猜测是用户自己的进程。

## 3. 主机密钥

保持 Strict 为默认。Unknown 可以经明确确认进入信任；Changed 必须单独阻断，不因 Retry、网络恢复或一次普通确认静默替换已知密钥。现有严格路径与大小限制要保留。[S12](13_SOURCE_INDEX.md#s12)

审批 envelope 至少包含 tunnel/host ID、显示的 hostname:port、hop index/total、algorithm、fingerprint、generation、attempt、prompt ID、过期时间。HostKeyAlias 支持后，要同时显示“连接地址”和“校验身份”，不能让别名遮住实际目标。

提示默认焦点为取消/返回。必须支持复制完整 SHA256 指纹，显示信任保存的位置和作用域。不要只用红色表示风险，也不要把“信任并保存”设为回车默认选项。

临时信任的推荐契约为 **本次 Tunnel Run 期间**，可跨其自动重连，明确在 Stop、关键主机配置变更或进程退出时失效。键值包含目标身份、端口、算法、完整指纹及 run ID；known_hosts 的 changed/revoked 优先于临时允许。不要扩大为“同 IP 所有主机”或所有端口。若实现保持一次握手语义，文案必须直说，不能称会话信任。

多文件 known_hosts 重查需完整完成再给出允许；不能在第一份文件匹配时提前退出、跳过后续冲突。保存操作有锁、上限、原子/可恢复策略。应用维护的文件可以写，用户全局文件默认不擅自覆盖。证书、撤销项、hash host、IPv6 与自定义端口应有明确的支持矩阵；不支持的安全语义必须 fail closed 或将相关导入项标为待处理，不能悄悄降级。

## 4. 凭据与 Keyboard Interactive

Password、PrivateKey、Agent 已有实现，不改成读取明文配置。不为每次连接把所有主机凭据读入长期全局缓存；按实际 chain 需要加载并尽快释放。输入值尽量少复制，使用受控敏感容器；**不能承诺 GPUI InputState、russh 和系统 API 内的全部副本均已零化**。[S10](13_SOURCE_INDEX.md#s10)、[S16](13_SOURCE_INDEX.md#s16)、[S18](13_SOURCE_INDEX.md#s18)

私钥读取继续使用限制长度的 reader，不根据 metadata 大小就无界读取。解码/解密可能是 CPU 密集工作，需移出 async 热路径并限制并发；文件读取、解析、解密都纳入阶段/尝试预算。导入器读取 Include 也应检查普通文件、总字节数、文件数和递归范围，避免无限流或非预期网络路径。

Keyboard Interactive 的最小完整交付为：支持多轮 server challenge；正确处理每个 prompt 的 echo；显示经过长度/控制字符处理的服务端说明；按 tunnel/hop/generation 路由；取消、Stop、窗口关闭与总交互预算均生效；响应不落配置、不落日志、不跨代缓存。建议上限作为首轮预算：单轮 8 个 prompt、总 8 轮、展示文本 4 KiB、单响应 16 KiB，超限返回明确不支持/中止而非静默截断认证内容。

密码与口令编辑提供 Keep/Replace/Remove，不把空字符串当“清除”。凭据轮换与备份恢复按 05 的引用集合处理，不能保存后立即删除所有旧引用。

## 5. 超时不是一个字段

| 阶段 | 预算原则 | 取消要求 |
|---|---|---|
| keyring/密钥准备 | 有界工作和总尝试预算，解码并发受限 | 已启动 blocking job 如实追踪；不谎称 abort 已停止 |
| DNS/TCP/banner/KEX | 使用配置网络预算 | Stop 可中止网络等待 |
| 等待主机密钥确认 | 仅真正出现 prompt 时进入独立人机预算 | 过期/Stop 关闭 prompt，旧按钮无效 |
| Password/Key/Agent 认证 | 有界认证预算 | 不吞掉取消并误归类为认证失败重试 |
| Keyboard Interactive | 每轮与总轮数、总交互预算 | 完整接线取消与代号 |
| channel-open | 从 admission 起的整体 deadline | 包括队列等待，不只网络 await |
| health probe | 每跳与整轮预算均明确 | 不阻塞 accept，过时代号结果被拒绝 |
| graceful stop | 共享 StopContext 剩余时间 | 不能分层累加完整时限 |

自动 keepalive、用于展示 RTT 的 probe、SSH transport inactivity 和业务连接 idle timeout 各有含义。原 v1 的 inactivity 设置需要补齐，但长期空闲隧道默认不能仅因无业务流量被关闭。启用探测对 inactivity 计时的影响必须写入设置帮助，并测试真实行为。

## 6. channel-open 并发与健康

Local worker 不应在收到一个 open 请求后阻塞主事件循环等待完成。保持窄所有权，使用 bounded in-flight futures（如受限 FuturesUnordered 或等价任务集）；借用 session 的生命周期结束前先取消并排空这些 futures，再释放 chain。不要为了跨任务而给整个连接树加一把粗锁。

建议初始值为单隧道并发 open=4、全局 pending open=32、单隧道 active relay=64。它们是待测试调参的预算，不是已测最佳值。请求应携带 accepted_at、deadline、attempt、reply sender；过期后释放 permit 并关闭对应连接。

健康探测与 admission 分离，同一个 session 不积压无限 ping。逐跳结果不是独立网络段 RTT：后续 SSH 连接通过前面的链建立，不能把各跳 RTT 简单相加当总 RTT。界面显示“到该跳的 session RTT”。

固定 russh 0.63.3 的 send_ping 忽略回执通道错误，见 [TW-022](02_DEFECT_REGISTER.md#tw-022)。必须加断线回归并确定修正策略；不能只因 API 名叫 send_ping 就断言它在所有失败路径都给出可靠成功证据。[E10](13_SOURCE_INDEX.md#e10)

## 7. Local / Dynamic 数据面

保留当前 half-close 方向性：收到 EOF 后仅 shutdown 对端写半部，继续接收另一方向，直到正常结束、错误或取消。保留 16 KiB/方向作为已知基线，缓冲大小调整必须用小包和大流量测试，而不是凭感觉减小。

SOCKS 只实现 NO AUTH + CONNECT + IPv4/IPv6/DOMAIN，明确拒绝 BIND/UDP ASSOCIATE。Domain 传给 SSH 远端，不做本地预解析。只在 SSH channel 成功建立后发送 SOCKS success；端口 0、空域名、异常版本、RSV、过长/畸形输入都走有界错误路径。[E07](13_SOURCE_INDEX.md#e07)、[S25](13_SOURCE_INDEX.md#s25)、[S07](13_SOURCE_INDEX.md#s07)

错误码按实际信息映射。普通 queue/open 超时不能不加区分地称 IP TTL expired；未知错误用一般失败，明确知道拒绝/不可达时才用对应码。成功回复中的 BND 地址若无法取得实际值，要明确兼容策略并做客户端互通测试，不能伪造目标地址就是绑定地址。

重连不能恢复已经断开的 TCP 流，恢复的是新建转发能力。所有 UI/README 必须说清，避免承诺数据库连接等业务流无缝续传。

## 8. Remote 数据面

成功注册后保存真实分配端口，即使进入 Degraded 也不丢失这个信息。连接是否健康与 registration 元数据分开，attempt 变化后旧 registration 不再有效。

forwarded-tcpip 必须与当前授权 registration 对应。IP 地址比较先规范化；wildcard、具体地址、IPv6 和 server 返回地址形式做互通测试。不能为修复兼容性将校验降成“端口一样就接收任意 forwarded channel”。客户端请求非回环地址也不代表已核验服务器最终监听范围。[S12](13_SOURCE_INDEX.md#s12)、[E08](13_SOURCE_INDEX.md#e08)

控制顺序为停止接受新转发、尽力取消注册、取消本地 relays、关闭 transport。Remote target connect 失败只关闭该条 channel 并记录原因，不应反复重启健康的 SSH session。服务端拒绝绑定固定端口、端口 0 分配、权限限制、取消请求无回应分别测试。

## 9. Socket 选项与依赖补丁

本项目用手动 TcpStream + connect_stream；要在尚能访问真实 socket 时验证 TCP_NODELAY。多跳 ChannelStream 没有相同 OS socket 选项，不能伪造一个统一 setter。设置/读取选项需有测试，不凭 client config 中的 nodelay 字段认定已生效。[S10](13_SOURCE_INDEX.md#s10)、[E10](13_SOURCE_INDEX.md#e10)

依赖变更必须最小化。若需要修补 russh 的 ping 错误传播，提交独立补丁、来源版本、回归、上游 issue/PR 线索及撤销条件。不能顺便重写 SSH 协议、放宽算法/信任策略或更新全部依赖。现有 GPUI vendor 修复同样保持可追溯，不与业务补丁混在一个大提交。

## 10. 诊断与安全输出

错误结构至少有错误码、阶段、host/tunnel ID、hop、attempt、generation、可重试性、用户可执行动作。日志只输出安全字段，不输出密码、passphrase、交互响应、私钥 PEM、完整带凭据命令或 SSH packet payload。

错误文案要能区分“本地端口被占用”“第 2 跳主机密钥发生变化”“SSH 链已连通但目标服务拒绝连接”“等待用户批准”“系统凭据不可用”。任何新诊断文本都经长度/控制字符处理，且用户能在本地复制或导出，无远程上传。
