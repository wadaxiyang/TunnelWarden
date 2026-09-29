# 01 — Upstream and License

## Fixed upstream baseline

TunnelWarden v0.1 development is based on:

- Oryxis: `https://github.com/wilsonglasser/oryxis`
- Commit: `16ad2af83b424780ff60b6420a6c20a3b30c7143`
- License: GNU AGPL-3.0-or-later

The upstream revision is pinned so Astra works against a stable source tree. Do not silently follow Oryxis `main` during implementation.

## Dependency policy

`oryxis-core` and `oryxis-ssh` are consumed directly from the pinned Git revision in the workspace `Cargo.toml`. They are the authoritative SSH domain/engine implementation.

If TunnelWarden eventually needs to patch either crate, first vendor the exact pinned source, keep upstream attribution, document every modified file, and preserve a clean diff against the pin.

## Copied `oryxis-app` code

Standalone forwarding runtime behavior is application-layer code in Oryxis. TunnelWarden intentionally ports the required subset into `tunnelwarden-core` rather than depending on Oryxis' iced application crate.

Every copied/modified source unit must retain the relevant upstream notice and be listed in `docs/02_COPY_MAP.md`.

## License files

- `LICENSE`: AGPL license text
- `NOTICE.ORYXIS`: upstream Oryxis notice
- `THIRD_PARTY_NOTICES.md`: project attribution map

This engineering document is not legal advice; when distributing binaries or offering a network-accessible modified covered work, verify current compliance requirements before release.
