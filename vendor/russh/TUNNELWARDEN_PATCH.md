This directory contains russh 0.63.3, licensed under Apache-2.0.
Source: https://github.com/warp-tech/russh

TunnelWarden carries one local fix in `src/client/mod.rs`: `Handle::send_ping`
returns an error when its reply channel closes before a server response. The
upstream 0.63.3 implementation ignored that channel error and returned success.
The adjacent `dropped_ping_reply_is_not_success` test exercises this case.

Keep the Cargo patch and this change under review when updating russh.
