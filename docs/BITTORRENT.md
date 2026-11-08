# BitTorrent engine — librqbit (pre-alpha)

The default torrent engine is embedded **librqbit 9.1.0**, rather than a downloaded external process. Kordra starts its DHT/session lazily on the first torrent request. Requests can specify `engine: "librqbit"` with a magnet link or an HTTP(S) link to a .torrent file.

During an active transfer Kordra polls completed bytes, uploaded bytes and total bytes from librqbit and records persistent counters and size, including seeding after completion. A second torrent with the same infohash is rejected instead of creating two Kordra queue records attached to the same internal session.

Current limitations:
- libtorrent-rasterbar is **not** yet integrated; issue #2 tracks it.
- Session persistence and automatic torrent recovery after daemon restart are not implemented.
- Pause/resume of active torrents is available via the shared task API; file selection, tracker management and .torrent file upload are not yet available.
- The main system's concurrency semaphore limits active download workers, not ongoing seeding.
- The code has compile/unit CI only, not full live integration tests against test torrents.
- HTTP(S) .torrent URLs are allowed; local filesystem torrent filenames are not accepted through the remote API.
