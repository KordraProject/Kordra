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
