# Managed external tools

Initial implementation: `yt-dlp` standalone. Kordra stores its active version, fallback, pinned version, channel and installed versions under the application data directory.

- Install a particular release tag from a specified channel.
- Automatically check a channel for updates (stable/nightly/master).
- Verify release binary SHA-256 against upstream `SHA2-256SUMS`.
- Validate executable with `--version` before activating.
- Keep the previous active release as a fallback.
- Pin installed versions; switch and roll back manually.

The first download automatically installs yt-dlp if no active binary exists. Later milestones generalize this to Deno, FFmpeg, aria2c, gallery-dl and rclone, including their version-specific checksums.

**Limitations:** the current downloader supports Windows x64 and Linux x64/arm64 yt-dlp standalone assets. Windows arm64 support requires a verified upstream asset strategy. A simple SHA-256 verification is not a supply-chain signature: signed-manifest verification and hardened atomic Windows activation remain planned.

## Deno runtime

Kordra now manages Deno as a separately versioned runtime (from `denoland/deno`). For a YouTube or youtu.be URL, the yt-dlp worker installs Deno on demand if absent and passes its managed path via `--js-runtimes`. Other hosts do not trigger Deno installation.

Stable release archives are verified with their matching `.zip.sha256sum` asset, checked for executable size, extracted and health-checked via `deno --version`. Active, previous fallback and pinned versions are stored in `runtimes/deno` under the application data directory.

API endpoints and Web UI support version installation, manual activation, pinning and rollback. Active installations periodically check for updates once a day (if enabled and not pinned). Versioned paths permit concurrent downloads to continue using a previous runtime.

These are SHA-256 checks, not cryptographic publisher signatures. Full artifact attestations and Windows atomic activation are future milestones.
