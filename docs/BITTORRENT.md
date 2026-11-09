# BitTorrent engine — librqbit (pre-alpha)

The default torrent engine is embedded **librqbit 9.1.0**, rather than a downloaded external process. Kordra starts its DHT/session lazily on the first torrent request. Requests can specify `engine: "librqbit"` with a magnet link or an HTTP(S) link to a .torrent file.

During an active transfer Kordra polls completed bytes, uploaded bytes and total bytes from librqbit and records persistent counters and size, including seeding after completion. A second torrent with the same infohash is rejected instead of creating two Kordra queue records attached to the same internal session.

Current limitations:
- libtorrent-rasterbar is **not** yet integrated; issue #2 tracks it.
- Kordra stores the last file selection and seeding preference in SQLite and can reattach completed torrents after restart in hot/cold-persistent mode. Metadata resolution and recheck may fail if sources disappear; see recovery notes below.
- Pause/resume and per-file selection are available through the shared task API; tracker management and .torrent-file upload are not yet available.
- The main system's concurrency semaphore limits active download workers, not ongoing seeding.
- The code has compile/unit CI only, not full live integration tests against test torrents.
- HTTP(S) .torrent URLs are allowed; local filesystem torrent filenames are not accepted through the remote API.

## Cancel and stop seeding

The universal queue's Cancel action now supports live torrents. Kordra asks librqbit to forget the torrent with `Session::delete(..., false)`, preserving all on-disk files. The task becomes `cancelled`, not `failed`. During initial magnet metadata resolution, cancellation may be unavailable until the managed handle is created.

A completed torrent can stop uploading with `POST /api/v1/downloads/{id}/stop-seeding`. This preserves the completed history and local files. In cold/temporary mode seeding is already stopped automatically when the final torrent completes.

## Select files to download

`GET /api/v1/downloads/{id}/torrent` returns `files` as an array containing stable metadata indices, names, lengths, inclusion flags, and per-file downloaded bytes. The unified Web UI displays checkboxes and per-file progress.

`POST /api/v1/downloads/{id}/torrent/files` with `{"only_files":[0,2,3]}` changes the selection of an active or paused torrent via `librqbit::Session::update_only_files`. Indices must be within metadata bounds; empty and oversized requests are rejected. Selection doesn't delete any already-downloaded files; completed task selection changes are not supported in this pre-alpha API.

## Recovery and persistence (pre-alpha)

- Torrent file selection is persisted per task in `torrent_preferences` and reapplied on a subsequent retry, daemon restart or seeding reattach. The database uses an additive migration and existing tasks default to selecting all files.
- `seed_enabled` defaults to true for legacy completed torrents. Calling `stop-seeding` persists `false` **before** stopping the live torrent, so a daemon crash cannot unexpectedly resume that torrent's uploading.
- On startup in **hot** or **cold/persistent** mode, Kordra enumerates all completed torrents allowed to seed and reattaches them asynchronously. The reattach uses `AddTorrentOptions::overwrite=true` as required by librqbit to verify/reuse existing files. Interrupted and failed torrents are **not** silently resumed: they still require user-initiated Retry. Paused torrents retain their status and can be resumed explicitly.
- Restored torrents add their fresh in-process uploaded byte count on top of the pre-restart total, preventing the cumulative upload counter from resetting. If a previously completed torrent has missing data, librqbit may re-download it during recheck.
- In **cold/temporary** mode, completed torrents are forgotten with `Session::delete(..., false)` and the session is shut down when no other torrents remain; files stay on disk.
- A failed reattach is logged without changing a completed task to failed, and will be retried on another daemon start. The current implementation does not persist raw .torrent metadata, and magnet/HTTP sources may become unavailable.

This is an early recovery mechanism. Full crash-safe session metadata, fastresume snapshots, rate controls and regression tests against a local synthetic torrent are still required before production use.
