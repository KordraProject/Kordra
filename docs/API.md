# API v1 (pre-alpha)

Default bind: `http://127.0.0.1:8079`. Optional bearer token via `KORDRA_API_TOKEN`. Never expose plain HTTP bearer auth to the public internet.

| Method | Path | Notes |
|---|---|---|
| GET | /api/v1/health | Health |
| GET | /api/v1/downloads | Up to 500 most recent tasks |
| GET | /api/v1/downloads/{id} | One task |
| POST | /api/v1/downloads/{id}/pause | Pause queued tasks or active librqbit torrents |
| GET | /api/v1/downloads/{id}/torrent | Live details: name, infohash, trackers, files' byte progress, upload and status |
| POST | /api/v1/downloads/{id}/resume | Resume paused task (requeue or unpause torrent) |
| POST | /api/v1/downloads/{id}/retry | Retry failed/interrupted task; returns 409 for ineligible states |
| POST | /api/v1/downloads/{id}/cancel | Cancel queued, paused, active yt-dlp or active librqbit; preserve files |
| POST | /api/v1/downloads/{id}/stop-seeding | Forget completed torrent from session; keep downloaded files and completed history |
| GET | /api/v1/engines/capabilities | Discover implemented engines and capabilities |
| POST | /api/v1/downloads | `{"source":"https://...","engine":"yt-dlp"}` or `{"source":"magnet:?xt=...","engine":"librqbit"}`; returns 501 for unimplemented engines |
| GET | /api/v1/stats | Persistent totals |
| GET | /api/v1/stats/daily | Recent daily transfer bytes |
| GET | /api/v1/stats/engines | Grouped task stats by engine |
| GET | /api/v1/engines/yt-dlp | Active, fallback, pinned and installed versions |
| GET | /api/v1/runtimes/deno | Deno runtime state |
| POST | /api/v1/runtimes/deno/update | Download latest stable Deno, with optional `{"activate":true}` |
| POST | /api/v1/runtimes/deno/install/{version} | Install a specific stable release tag |
| POST | /api/v1/runtimes/deno/activate/{version} | Activate already installed version |
| POST | /api/v1/runtimes/deno/rollback | Restore fallback |
| POST | /api/v1/runtimes/deno/policy | `{"pinned":null,"auto_update":true}` |
| POST | /api/v1/engines/yt-dlp/update | `{"channel":"stable","activate":true}` |
| POST | /api/v1/engines/yt-dlp/install/{version} | `{"channel":"stable","activate":false}` |
| POST | /api/v1/engines/yt-dlp/activate/{version} | Manually switch installed version |
| POST | /api/v1/engines/yt-dlp/rollback | Activate prior version |
| POST | /api/v1/engines/yt-dlp/policy | `{"channel":"stable","pinned":null,"auto_update":true}` |
| GET | /metrics | Prometheus scrape; separate metrics token available |

Agent uses a separate protocol in its own repo; pairing and remote control are not shipped yet.

This is not the final API. WebSocket streaming, task cancellation/retry, identity, roles, OpenAPI and formal capability negotiation are upcoming.
