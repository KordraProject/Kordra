# Kordra

Extensible universal download manager. **Early pre-alpha**.

Kordra is a Rust-first, self-hostable download-management platform with one universal queue and a versioned API. The **Kordra-Agent** project lives in [its own repository](https://github.com/KordraProject/Kordra-Agent).

## Currently implemented
- Rust workspace, SQLite task storage, persistent transfer counters.
- API and CLI, headless daemon.
- yt-dlp standalone version manager (checksum checks, channels, pinned/active/fallback versions).
- Initial yt-dlp task worker (other engines are planned, not implemented).
- Prometheus-compatible `/metrics` and API statistics.
- Plugin SDK **contract only**, not a third-party plugin loader.
- GitHub-hosted CI; no personal server is used for development or builds.

## Security
By default the API listens only on `127.0.0.1:8079`. Do not publish it to the internet: TLS, user accounts and remote pairing are not yet implemented. An API token is mandatory for non-loopback binding, but **plain HTTP with a bearer token is not sufficient for public deployment**.

## Development
All builds/tests/releases run on GitHub-hosted GitHub Actions runners. No personal server, VPS, remote desktop or self-hosted runner is used for this project.

See [architecture](docs/ARCHITECTURE.md), [roadmap](docs/ROADMAP.md), [API](docs/API.md) and [engine versions](docs/ENGINE_VERSIONS.md).
