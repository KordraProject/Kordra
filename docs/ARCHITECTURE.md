# Architecture (pre-alpha)

Single universal download queue shared across Desktop, Web, Telegram, CLI and headless interfaces.

```
Kordra desktop/web/CLI/Telegram (planned, CLI implemented)
          | versioned REST / WebSocket (WebSocket planned)
          v
   API + auth boundary
          |
   SQLite + statistics + scheduler
          |
     engine adapters
          |
 librqbit (default, planned), libtorrent helper (planned),
 yt-dlp standalone (first implemented), aria2, gallery-dl, rclone (planned)
```

Kordra-Agent is an independently released repository and is designed to manage local Kordra headless instances, not duplicate download engine code. Remote pairing is not yet implemented; agent binds loopback only.

## Rules
- Core has no frontend dependency; the desktop shell will use Tauri 2.
- Engine capabilities are explicit: no phantom controls.
- External tool versions can be pinned and rolled back; tasks resolve executable path when starting.
- All downloads use managed paths and subprocesses, never shell interpolation.
- Public HTTPS/OIDC/multi-user/agent pairing require additional security work; current daemon is local-first.
- CI and release builds run solely on GitHub-hosted Actions runners; absolutely no personal VPS/staging/server.
- Plugin permissions are a contract; manifest declarations do not yet enforce sandbox security.
