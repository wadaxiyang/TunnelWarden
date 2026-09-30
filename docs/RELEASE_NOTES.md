TunnelWarden 0.2.0 adds connection diagnostics and an on-demand interface.

- **Test connection** on saved SSH hosts: verifies real SSH authentication,
  jump routes and host fingerprints, without starting a tunnel. Supports cancel.
- **Encrypted private keys**: passphrase prompt, wrong-passphrase retry and
  cancellation. Decryption uses pinned Oryxis `oryxis-vault::import_key`.
  Passphrases are never saved; unlocked keys stay in memory until app exit.
- **Export error logs**: native Save As produces JSON with errors, recent
  activity, runtime state, version and upstream commit. Oryxis redaction masks
  known secret shapes. Review server addresses/error details before sharing.
- **Remote listener guidance**: server policy requirements, original refusal
  errors and an explicit distinction between established forwarding and
  unverified external/public reachability.
- **UI on demand**: closing destroys the WebView while Rust tunnels and tray
  continue. Tray/second launch recreates it. Login startup creates no WebView.
  Unsaved edits receive a close confirmation.

Install the Windows x64 `x64-setup.exe` (per-user NSIS) or `.msi` package.
Installers acquire Microsoft WebView2 if needed. Packages are unsigned.
SHA256SUMS.txt contains the artifact hashes. Existing v0.1 configuration is
compatible; this update does not migrate or reset saved profiles.

Oryxis remains pinned to 16ad2af83b424780ff60b6420a6c20a3b30c7143. SSH engine
behavior is unchanged; standalone runtime and diagnostic masking retain source
provenance. AGPL-3.0-or-later; source and license notices accompany this release.

Known limits: Windows x64 only. Encrypted files require manual unlock after a
full process restart, or an external agent for unattended authentication.
Unknown/changed host keys require manual verification. No terminal, SFTP,
built-in agent, certificate or interactive OTP UI. Remote/public access still
depends on server policy and network configuration. Reports cover the current
process session (up to 1,000 entries); earlier rotating files are separate.
The app must remain running in the tray to maintain tunnels.
