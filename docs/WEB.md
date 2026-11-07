# Web UI (pre-alpha)

Web files: `apps/web`. React 19 + TypeScript + Vite. Built by GitHub Actions and into the Docker image.

To serve: set `KORDRA_WEB_ENABLED=true` and `KORDRA_WEB_DIR` to the built `dist` directory; the daemon serves static assets on the same listener as the API. Docker defaults to Web UI when available.

Current features: one unified downloads table, task filters/search, persistent totals, create a yt-dlp task, channel/versions/pinning/rollback, basic appearance controls and mobile responsive layout.

The API token, if set, is stored **in browser sessionStorage** for the active tab rather than localStorage; this is only a pre-alpha local client and is not a replacement for secure authentication. Exposing HTTP on an untrusted network will leak bearer tokens. Use localhost or private HTTPS reverse proxy until native TLS and user accounts are implemented.
