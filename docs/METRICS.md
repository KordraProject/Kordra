# Statistics and Prometheus metrics

`GET /api/v1/stats` gives persistent aggregates stored in SQLite. `GET /metrics` exports text in Prometheus exposition format.

Current metrics:
- `kordra_download_bytes_total` (counter)
- `kordra_upload_bytes_total` (counter, currently zero until an upload-capable engine exists)
- `kordra_tasks_total` (gauge)
- `kordra_active_tasks` (gauge)
- `kordra_completed_tasks_total` (counter)
- `kordra_failed_tasks_total` (counter)

Scraping a protected metrics endpoint requires `Authorization: Bearer <KORDRA_METRICS_TOKEN>` (or the API token if no separate metrics token is set). Recommended to keep endpoint private and use a reverse proxy for TLS.

Counters use monotonic observations per task to avoid counting identical progress events more than once. They do not count transfer bytes that the downloader never reports, and do not yet provide daily/time-series or per-engine breakdowns.
