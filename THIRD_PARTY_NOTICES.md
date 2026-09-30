# Third-Party Notices

## Oryxis

TunnelWarden intentionally reuses and adapts code from **Oryxis**.

- Upstream repository: `https://github.com/wilsonglasser/oryxis`
- Pinned source revision: `16ad2af83b424780ff60b6420a6c20a3b30c7143`
- Upstream copyright: Copyright (C) 2026 Wilson Glasser and Oryxis contributors
- License: **GNU Affero General Public License v3.0 or later**
- Original notice: `NOTICE.ORYXIS`
- License text: `LICENSE`

TunnelWarden source copied or adapted from Oryxis must retain attribution and modification notices required by the license. This file is a project engineering notice, not legal advice.

### v0.2.1 SSH instrumentation

`vendor/oryxis-ssh` is the complete Oryxis SSH crate at the pinned SHA above,
modified 2026-09-30 only for passive per-forward payload counters and manifest
adaptation. See its PROVENANCE.md, LICENSE and the checked source diff in
`upstream/oryxis-ssh-traffic.patch`. GNU AGPL-3.0-or-later.
