# 07 — UI Product Specification

## Design goal

A compact Windows-friendly utility: restrained, readable, consistent, and much simpler than a full SSH client. No dashboard theatrics. The interface should optimize “configure correctly, see whether it is running, stop it immediately”.

## Navigation

```text
Tunnels
SSH Hosts
Logs
Settings
About
```

## Tunnel list

Each row/card shows:

- name
- mode badge: `L`, `R`, or `D`
- human-readable route
- runtime status
- Start/Stop primary action
- Edit secondary action

### Route rendering

**Local**
```text
127.0.0.1:15432  →  server.example:22  →  db.internal:5432
Local listener       SSH host              Remote target
```

**Remote**
```text
0.0.0.0:8443  ←  server.example:22  ←  127.0.0.1:3000
Remote listener    SSH host              Local target
```

**Dynamic**
```text
SOCKS5 127.0.0.1:1080  →  server.example:22
```

## Tunnel editor

Use a segmented mode selector: `Local (-L)` / `Remote (-R)` / `Dynamic (-D)`.

### Common fields

- Name
- SSH host profile
- Auto-start
- Auto-reconnect, if retained from the selected Oryxis behavior

### Local `-L`

- Local listen address
- Local listen port
- Remote target host
- Remote target port

### Remote `-R`

- Remote listen address
- Remote listen port
- Local target host
- Local target port

### Dynamic `-D`

- Local listen address
- Local SOCKS port

Do not show irrelevant fields disabled; omit them entirely for the selected mode.

## SSH host editor

- Name
- Hostname/IP
- **SSH port**
- Username
- Authentication method
- Key/agent/password-related fields as supported by the Oryxis adapter
- Jump route selector

The SSH server port must never be conflated with tunnel listener/target ports.

## Visual rules

- 8px base spacing grid
- 8–12px radius, not excessive nested cards
- one primary action per surface
- no more than two hierarchy levels of surfaces
- labels always visible; placeholders do not replace labels
- error text next to the relevant form section
- dark/light/system theme may be added without changing information architecture

## v0.2 authorized additions

Saved hosts include a cancellable connection test. Private-key unlock asks for
an ephemeral passphrase and explains process-memory caching. Logs export a
current-session error report through native Save As. Remote configuration
explains server forwarding/bind policies and external reachability remains
explicitly unverified. Closing the UI releases its WebView; an unsaved editor,
settings draft or pending question requests confirmation. Backend runtime and
tray survive, and a recreated UI always reloads the authoritative state.
