# Development roadmap

- [x] Create organization and two repositories.
- [x] Rust workspace, task DB, persistent aggregate metrics, API, CLI and daemon source.
- [x] yt-dlp external version manager with active/fallback/pin and checksum verification.
- [x] First yt-dlp worker (limited progress and process control).
- [x] Plugin SDK manifest (no plugin executor yet).
- [ ] Green GitHub-hosted CI across Linux and Windows; enforce formatting/Clippy.
- [ ] Generalize engine manager for Deno, FFmpeg, aria2, gallery-dl, rclone and version selection.
- [ ] Implement librqbit BitTorrent default; libtorrent in isolated helper.
- [ ] Retry/resume/pause/cancel, per-engine capabilities, queue scheduling, file operations, task events.
- [ ] Versioned API with WebSocket, scoped tokens, user accounts and OIDC.
- [ ] Web UI with one universal downloads view, statistics, themes and draggable panels.
- [ ] Tauri 2 desktop, Windows .exe/.msi/portable and Linux .deb/.rpm/.AppImage.
- [ ] Telegram bot and Mini App; configurable Docker components (web on by default).
- [ ] TLS ACME auto-renew / manual cert / disabled behind reverse proxy.
- [ ] Fully permissioned sandboxed plugin execution (engines, widgets, automations).
- [ ] Kordra-Agent pairing, authenticated transport, per-node views and health metrics.
- [ ] GitHub-hosted builds for x64/arm64 headless and Docker images to GHCR.
- [ ] Native installers, signatures, supply-chain verification, updater rollback, backups.
