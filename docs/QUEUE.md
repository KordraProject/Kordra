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

## Recovery and retries

Daemon startup marks previously running jobs `interrupted` (not silently retried). The user can explicitly restart them, or retry failed jobs, using `POST /api/v1/downloads/{id}/retry`, `kordra-cli retry <id>`, or the unified queue's Retry button. Only one concurrent retry is accepted for the same task. The persistent `attempts` counter increases only when a job actually transitions from queued to running, not merely when a retry is requested. Download paths and task ID stay unchanged.

The first implementation relies on native yt-dlp partial-file resume and librqbit's existing on-disk data during torrent recheck. Cross-restart torrent metadata and seeding restoration are **not implemented**; additional persistence/recheck work remains required. Retrying can fail for sources no longer available.

## Cancellation and startup backlog

`POST /api/v1/downloads/{id}/cancel` and `kordra-cli cancel <id>`:

- `queued` or `paused` without a live torrent handle: atomically transition to `cancelled`. Its worker cannot start later.
- Active `yt-dlp`: transition `running → cancelling`, signal the managed worker, kill/reap the direct child process and finalize `cancelled`. Competing completion/error transitions are resolved so tasks cannot remain in `cancelling`.
- Active/paused live `librqbit`: forget from the session through `Session::delete(..., false)`, preserving files; initial magnet resolution may still block cancellation until a managed handle is available.
- Completed/seeding torrents can stop seeding without changing their completed status via the separate stop-seeding endpoint.
- Successful cancellation is not recorded as a failed download. `cancelled` is terminal; re-adding the source creates a new task.

Startup scans **all** queued tasks in creation order instead of silently ignoring any beyond the 500 most recently shown in the Web UI. The initial scheduler still spawns a waiting future per queued task; bounded-memory, priority-aware scheduling remains planned. Cancelling a yt-dlp subprocess currently stops only the direct child, not its complete process tree. Full Windows Job Object and Unix process-group management is still required.

## Durable BitTorrent preferences

SQLite stores file selection and whether a completed torrent should keep seeding. On startup, Kordra repairs jobs caught in `cancelling` to `interrupted`, allowing an explicit retry instead of leaving them permanently stuck. In hot and cold/persistent modes, completed torrents with seeding enabled are reattached to librqbit using existing files. Cold/temporary mode forgets completed torrents from the embedded session without deleting files, rather than merely removing Kordra's in-memory handle.
