# Architecture (pre-alpha)

One queue and one server-side task database shared by all interfaces:

```
   React Web UI (initial)    CLI (initial)
            |                |
          REST /api/v1  +  /metrics
                     |
               Kordra Core
          SQLite / worker / stats
                     |
             Engine adapters
       yt-dlp standalone (initial)
       librqbit/libtorrent/aria2/rclone (planned)
```

The Web UI is built using React, TypeScript and Vite and served by the headless daemon as static assets, with no separate web service. When built in Docker, the Web UI is on by default and can be disabled via `KORDRA_WEB_ENABLED=false`. Running without Web assets leaves a functioning headless API.

Planned: Tauri 2 desktop, Telegram bot and Mini App, WebSocket API, extra engines, plugin sandbox, automatic HTTPS and agent-controlled remote devices. Kordra-Agent lives in a separate repository and initially binds to loopback only.

Engine activation: global hot/cold policy plus temporary/persistent cold retention, with a real live BitTorrent session and managed CLI tool readiness. See [lifecycle](ENGINE_LIFECYCLE.md).\n\nAll CI, package builds and tests run **exclusively** on GitHub-hosted GitHub Actions. Personal machines and servers are not development, build or testing hosts.
