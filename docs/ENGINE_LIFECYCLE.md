# Engine activation lifecycle (pre-alpha)

Kordra now has **two startup modes** and two cold retention policies, configured by `PUT /api/v1/engines/lifecycle` and persisted in `<data>/config/engine-lifecycle.json`.

```json
{"startup":"cold","cold_retention":"persistent"}
```

Defaults: cold/persistent (maintains pre-existing torrent seeding behavior).

- **hot:** initialize installed engines on daemon startup; start the embedded librqbit network session. Installed yt-dlp/Deno assets are validated, but missing third-party binaries are **not downloaded**. Hot overrides temporary cold retention while in effect.
- **cold / temporary:** create a BitTorrent session when needed; stop it after the last active torrent finishes, so seeding stops too. yt-dlp subprocesses always exit when their commands finish, regardless of the retention policy.
- **cold / persistent:** on first use initialize engine; preserve the embedded BitTorrent session and allow completed torrents to seed until Kordra exits. CLI tools remain ready via their installed executable path rather than a useless idle process.

The explicit prepare endpoint `POST /api/v1/engines/{id}/prepare` accepts `librqbit`, `yt-dlp`, or `deno`. Preparing an absent CLI tool installs it on demand, but **hot startup never downloads missing tools**. Runtime state available via `GET /api/v1/engines/lifecycle`.

Changing policy never kills an actively downloading task. Existing seeding torrents may remain until daemon restart if switching from persistent to temporary. More precise per-engine activation and cancellation will follow once the full scheduler is ready.

Neither yt-dlp nor Deno has an idle daemon process in this implementation: they are CLI executables/runtimes. A future optional persistent worker-host protocol may support plugins that need a long-running subprocess.
