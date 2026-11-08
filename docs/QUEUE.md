# Download task lifecycle (pre-alpha)

The single Kordra queue includes yt-dlp and librqbit. A task starts as `queued`, becomes `running` when a worker acquires the shared concurrency semaphore, then `completed` or `failed`.

`POST /api/v1/downloads/{id}/pause`:
- Queued task: atomically move to `paused` without running.
- Active librqbit task: suspend the managed torrent in its in-process session.
- Active yt-dlp: currently rejected. OS-process suspend/resume is not reliably portable; future support will gracefully terminate and restart via yt-dlp's resume facilities.

`POST /api/v1/downloads/{id}/resume`:
- A paused torrent with an active handle is unpaused in place.
- A paused queued task, or one whose process was restarted, is requeued.

**Known limitations:** pause keeps the semaphore permit allocated for an active torrent; fully featured scheduler/fairness, cancellation, seeding across daemon restart and process-tree control are next. Completed torrents may continue seeding in the current process. This is not yet a production torrent manager.
