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

Startup uses a fixed-size Tokio dispatcher that **atomically claims** queued tasks from SQLite in FIFO order. It does not allocate one task/future per backlog entry, so thousands of queued downloads consume bounded scheduler memory. The UI still shows the 500 newest tasks, but `GET /api/v1/queue` shows actual global counts. Concurrency is configurable through `KORDRA_CONCURRENCY` (1–32; default 2). Priority scheduling, rate limits and live concurrency reconfiguration remain planned. Cancelling a yt-dlp subprocess currently stops only the direct child, not its complete process tree. Full Windows Job Object and Unix process-group management is still required.

## Durable BitTorrent preferences

SQLite stores file selection and whether a completed torrent should keep seeding. On startup, Kordra repairs jobs caught in `cancelling` to `interrupted`, allowing an explicit retry instead of leaving them permanently stuck. In hot and cold/persistent modes, completed torrents with seeding enabled are reattached to librqbit using existing files. Cold/temporary mode forgets completed torrents from the embedded session without deleting files, rather than merely removing Kordra's in-memory handle.

## Dispatcher, cancellation and wakeups

The durable SQLite table is the **single source of truth** for pending downloads. At startup Kordra creates at most `KORDRA_CONCURRENCY` queue dispatchers, instead of a task for every queued item. Each dispatcher obtains a semaphore slot and performs one atomic `UPDATE … RETURNING` claim of the highest-priority queued row (FIFO among tasks with equal priority). When the queue is empty it waits on `tokio::sync::Notify`; on create, resume or retry the API only sends a wake-up. This ensures no work is scheduled twice even when multiple dispatchers race.

The yt-dlp registration and cancellation paths now take the same token-map mutex while inspecting or changing the SQLite state. A cancellation immediately after a dispatcher claims a task is persisted as `cancelling` and detected before the subprocess starts. Attempt counters increase only on an actual claim. `GET /api/v1/queue` and `kordra-cli queue` report queued, running, paused, maximum concurrent and available slots.

## Task priorities

A queued or paused task has a durable SQLite priority level: -2 Very low, -1 Low, 0 Normal (default), 1 High, 2 Very high. Higher priorities are claimed first; equal priorities remain FIFO by creation time and task ID. Change priority using the one queue's dropdown, `PUT /api/v1/downloads/{id}/priority` with `{"priority":1}`, or `kordra-cli set-priority <id> 1`. Changing priority does not preempt a running download or evict one already occupying a slot. The queued priority level persists across daemon restarts. The queue has an index on status/priority/creation time to keep claims responsive even with large SQLite backlogs.
