# 08 — Acceptance Matrix

A first release is not complete until the relevant rows pass on the target OS.

| Area | Case | Expected |
| --- | --- | --- |
| Auth | password | connects using Oryxis path |
| Auth | private key | connects using Oryxis path |
| Auth | ssh-agent | connects using Oryxis path |
| Host | non-22 SSH port | connects to configured SSH port |
| Jump | one hop | tunnel routes through jump host |
| Jump | multi-hop supported by Oryxis | tunnel reaches target |
| `-L` | basic local forward | local listener reaches remote target |
| `-R` | basic remote forward | remote listener reaches local target |
| `-R` | stop/delete | server-side bind is explicitly released |
| `-D` | SOCKS CONNECT | SOCKS listener routes request through SSH |
| Shared conn | two rules, same SSH host | one shared PTY-less SSH connection semantics retained |
| Lifecycle | Stop while connecting | late successful connection does not resurrect rule |
| Lifecycle | Stop while running | listener/channels stop and status becomes stopped |
| Lifecycle | delete live rule | rule cancels before deletion completes |
| Failure | local port occupied | actionable error, no false running state |
| Failure | auth rejected | actionable error |
| Failure | host key failure | secure failure path, no silent bypass |
| Recovery | running connection drops | eligible rule becomes retrying |
| Recovery | retry backoff | Oryxis 15/30/60/120s-ceiling semantics preserved unless intentionally documented otherwise |
| Recovery | manual Stop during retry | retry is removed and rule stays stopped |
| Auto-start | app launch | enabled rules start through normal runtime path |
| Persistence | app restart | saved config returns; transient runtime state does not |
| UI | Remote editor | remote listen port and local target port both editable and unambiguous |
| UI | SSH host | SSH server port independently editable |
| IPC | start action | UI waits for backend runtime result, no optimistic false-ready state |

## v0.2 acceptance extension

The explicitly requested v0.2 scope retains every v0.1 acceptance item and adds:

| Area | Required result |
| --- | --- |
| SSH test | Real authentication/jump/fingerprint check; cancel; no listener created; active tunnels unaffected |
| Encrypted key | Incorrect/correct passphrase and cancel; no persisted passphrase/plain key; fresh unattended session fails clearly |
| Error export | Native Save As/cancel; parseable JSON; errors plus bounded recent context; no credential/key payload |
| Remote policy | Original refusal error retained; external bind guidance; no public-access claim from Running |
| Disposable UI | Zero WebView2 processes after close; live tunnel transfers; reopen reflects runtime; dirty draft confirmation |
| Headless startup | Login/background launch creates runtime/tray without WebView; installed package checked on CI |
