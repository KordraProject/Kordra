import { useCallback, useEffect, useMemo, useState } from "react";
import { api, post, prettyBytes } from "./api";
import type { Channel, DailyTransfer, Download, EngineState, Stats, TorrentDetails, DenoState, EngineLifecycleStatus, StartupMode, ColdRetention } from "./types";

const emptyStats: Stats = {
  total_downloaded_bytes: 0, total_uploaded_bytes: 0,
  total_tasks: 0, completed_tasks: 0, failed_tasks: 0, active_tasks: 0
};
const themeKey = "kordra-theme";
const accentKey = "kordra-accent";
type Filter = "all" | "active" | "completed" | "failed";
type Theme = "dark" | "light";

export default function App() {
  const [theme, setTheme] = useState<Theme>(() => localStorage.getItem(themeKey) === "light" ? "light" : "dark");
  const [accent, setAccent] = useState(() => localStorage.getItem(accentKey) ?? "#698cff");
  const [token, setToken] = useState(() => sessionStorage.getItem("kordra-api-token") ?? "");
  const [tokenDraft, setTokenDraft] = useState(token);
  const [tasks, setTasks] = useState<Download[]>([]);
  const [stats, setStats] = useState<Stats>(emptyStats);
  const [daily, setDaily] = useState<DailyTransfer[]>([]);
  const [engine, setEngine] = useState<EngineState | null>(null);
  const [deno, setDeno] = useState<DenoState | null>(null);
  const [lifecycle, setLifecycle] = useState<EngineLifecycleStatus | null>(null);
  const [denoVersion, setDenoVersion] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [source, setSource] = useState("");
  const [sourceEngine, setSourceEngine] = useState<"yt-dlp" | "librqbit">("yt-dlp");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [lastUpdated, setLastUpdated] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [selectedTorrent, setSelectedTorrent] = useState<{ taskId: string; details: TorrentDetails } | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.setProperty("--accent", accent);
    localStorage.setItem(themeKey, theme);
    localStorage.setItem(accentKey, accent);
  }, [theme, accent]);
  const refresh = useCallback(async () => {
    const [downloadList, summary, state, days, denoState, lifecycleStatus] = await Promise.all([
      api<Download[]>("/downloads", token), api<Stats>("/stats", token),
      api<EngineState>("/engines/yt-dlp", token),
      api<DailyTransfer[]>("/stats/daily", token),
      api<DenoState>("/runtimes/deno", token),
      api<EngineLifecycleStatus>("/engines/lifecycle", token)
    ]);
    setTasks(downloadList);
    setStats(summary);
    setDaily(days);
    setEngine(state);
    setDeno(denoState);
    setLifecycle(lifecycleStatus);
    setLastUpdated(new Date().toLocaleTimeString());
    setError("");
  }, [token]);
  useEffect(() => {
    let active = true;
    async function poll() {
      try { await refresh(); } catch (reason) {
        if (active) setError(reason instanceof Error ? reason.message : String(reason));
      }
    }
    void poll();
    const interval = window.setInterval(() => { void poll(); }, 3500);
    return () => { active = false; window.clearInterval(interval); };
  }, [refresh]);
  const visible = useMemo(() => tasks.filter(task => {
    const statusMatch = filter === "all" ||
      (filter === "active" && ["running", "queued"].includes(task.status)) ||
      (filter === "completed" && task.status === "completed") ||
      (filter === "failed" && ["failed", "interrupted"].includes(task.status));
    return statusMatch && `${task.source} ${task.engine} ${task.id}`.toLowerCase().includes(search.toLowerCase());
  }), [tasks, filter, search]);

  async function operation(action: () => Promise<unknown>) {
    setBusy(true);
    try { await action(); await refresh(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  }
  async function addTask(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!source.trim()) return;
    await operation(async () => {
      await post("/downloads", token, { source: source.trim(), engine: sourceEngine });
      setSource("");
    });
  }
  function saveToken(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    sessionStorage.setItem("kordra-api-token", tokenDraft);
    setToken(tokenDraft);
    setError("");
  }
  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-icon">K</span><span>Kordra<small>pre-alpha</small></span></div>
        <div className="sidebar-label">Workspace</div>
        <div className="nav-active">↓ All downloads</div>
        <div className="nav-item" onClick={() => setSettingsOpen(true)} role="button" tabIndex={0}
          onKeyDown={event => { if (event.key === "Enter") setSettingsOpen(true); }}>⚙ Preferences</div>
        <div className="sidebar-bottom">One queue · All engines<br/>Local-first, open-source</div>
      </aside>
      <main className="content">
        <header className="topbar">
          <div><h1>All downloads</h1><p>Manage every transfer in a single workspace.</p></div>
          <div className="top-actions">
            <button className="button secondary" onClick={() => setSettingsOpen(v => !v)}>Settings</button>
            <button className="button secondary" onClick={() => setTheme(t => t === "dark" ? "light" : "dark")}>
              {theme === "dark" ? "Light mode" : "Dark mode"}
            </button>
          </div>
        </header>
        {error && <div className="warning" role="alert">{error}</div>}
        <section className="stats-grid" aria-label="Transfer statistics">
          <Stat label="Downloaded" value={prettyBytes(stats.total_downloaded_bytes)} />
          <Stat label="Uploaded" value={prettyBytes(stats.total_uploaded_bytes)} />
          <Stat label="Active tasks" value={stats.active_tasks.toLocaleString()} />
          <Stat label="Completed" value={stats.completed_tasks.toLocaleString()} />
        </section>
        <section className="panel">
          <div className="panel-head"><div><h2>Transfer history</h2><p>Daily downloaded bytes, persisted by the backend.</p></div></div>
          <div className="history">
            {(() => {
              const recent = [...daily].reverse().slice(-14);
              const maximum = Math.max(1, ...recent.flatMap(item => [item.downloaded_bytes, item.uploaded_bytes]));
              return recent.length ? recent.map(item => <div className="history-col" key={item.day}
                title={`${item.day}: Downloaded ${prettyBytes(item.downloaded_bytes)} · Uploaded ${prettyBytes(item.uploaded_bytes)}`}>
                <div className="history-bars">
                  <div className="history-bar downloaded" style={{height: `${Math.max(4, 100 * item.downloaded_bytes / maximum)}%`}} />
                  <div className="history-bar uploaded" style={{height: `${Math.max(4, 100 * item.uploaded_bytes / maximum)}%`}} />
                </div>
                <span>{item.day.slice(5)}</span>
              </div>) : <p>No traffic recorded yet.</p>;
            })()}
          </div>
          <p className="history-legend"><span className="legend-down" /> Downloaded <span className="legend-up" /> Uploaded</p>
        </section>
        <section className="panel">
          <form onSubmit={addTask} className="add-form">
            <div className="form-copy"><h2>New download</h2><p>One queue for media and BitTorrent. Select the appropriate engine.</p></div>
            <div className="input-row">
              <select aria-label="Download engine" value={sourceEngine} onChange={event => setSourceEngine(event.target.value as "yt-dlp" | "librqbit")}>
                <option value="yt-dlp">yt-dlp</option>
                <option value="librqbit">BitTorrent · librqbit</option>
              </select>
              <input aria-label="Download URL or magnet link" type="text" required placeholder={sourceEngine === "librqbit" ? "magnet:?xt=... or https://example.org/file.torrent" : "https://..."} value={source}
                onChange={event => setSource(event.target.value)} />
              <button className="button primary" type="submit" disabled={busy}>Add download</button>
            </div>
          </form>
        </section>
        <section className="panel">
          <div className="panel-head"><div><h2>Download queue</h2><p>{tasks.length} total tasks · Updated {lastUpdated || "—"}</p></div>
            <button className="button secondary" onClick={() => void operation(refresh)} disabled={busy}>Refresh</button></div>
          <div className="filters">
            {(["all", "active", "completed", "failed"] as Filter[]).map(item =>
              <button key={item} className={filter === item ? "filter active" : "filter"}
                onClick={() => setFilter(item)}>{item}</button>)}
            <input className="search" aria-label="Search downloads" placeholder="Search tasks..." value={search}
              onChange={event => setSearch(event.target.value)} />
          </div>
          <div className="table-wrap"><table><thead><tr>
            <th>Source</th><th>Engine</th><th>State</th><th>Progress</th><th>Received / Upload</th><th>Controls</th>
          </tr></thead><tbody>
            {visible.map(task => <tr key={task.id}>
              <td><span className="source-cell" title={task.source}>{task.source}</span>{task.error && <small className="error-text">{task.error}</small>}</td>
              <td>{task.engine}</td><td><span className={`status ${task.status}`}>{task.status}</span></td>
              <td>{task.total_bytes > 0 ? <div className="task-progress">
                <span>{(Math.min(100, 100 * task.downloaded_bytes / task.total_bytes)).toFixed(1)}%</span>
                <div className="task-bar"><div style={{width: `${Math.min(100, 100 * task.downloaded_bytes / task.total_bytes)}%`}} /></div>
              </div> : "—"}</td>
              <td>{prettyBytes(task.downloaded_bytes)} / {prettyBytes(task.uploaded_bytes)}</td>
              <td>{(task.status === "queued" || (task.status === "running" && task.engine === "librqbit")) &&
                <button className="button secondary" disabled={busy}
                  onClick={() => void operation(() => post(`/downloads/${task.id}/pause`, token))}>Pause</button>}
                {task.status === "paused" &&
                <button className="button secondary" disabled={busy}
                  onClick={() => void operation(() => post(`/downloads/${task.id}/resume`, token))}>Resume</button>}
                {task.engine === "librqbit" && task.status !== "queued" &&
                  <button className="button secondary" disabled={busy}
                    onClick={() => void operation(async () => {
                      const details = await api<TorrentDetails>(`/downloads/${task.id}/torrent`, token);
                      setSelectedTorrent({ taskId: task.id, details });
                    })}>Details</button>}
              </td>
            </tr>)}
            {!visible.length && <tr><td colSpan={6} className="empty">No matching downloads yet.</td></tr>}
          </tbody></table></div>
        </section>
        {selectedTorrent && <section className="panel torrent-details">
          <div className="panel-head">
            <div><h2>{selectedTorrent.details.name || "Torrent details"}</h2>
              <p>Info hash: {selectedTorrent.details.info_hash}</p></div>
            <button className="button secondary" onClick={() => setSelectedTorrent(null)}>Close</button>
          </div>
          <div className="torrent-metadata">
            <div><span>Received</span><strong>{prettyBytes(selectedTorrent.details.stats.progress_bytes)}</strong></div>
            <div><span>Uploaded</span><strong>{prettyBytes(selectedTorrent.details.stats.uploaded_bytes)}</strong></div>
            <div><span>Total size</span><strong>{prettyBytes(selectedTorrent.details.stats.total_bytes)}</strong></div>
            <div><span>Status</span><strong>{selectedTorrent.details.paused ? "Paused" : selectedTorrent.details.stats.finished ? "Seeding" : "Downloading"}</strong></div>
          </div>
          <h3>Trackers ({selectedTorrent.details.trackers.length})</h3>
          <div className="torrent-trackers">
            {selectedTorrent.details.trackers.length ? selectedTorrent.details.trackers.map(s => <div key={s}>{s}</div>) : <p>No trackers; torrent may use DHT.</p>}
          </div>
          <p>Files reported: {selectedTorrent.details.stats.file_progress.length}. Detailed names and file selection will be added in the next BitTorrent milestone.</p>
        </section>}
        <section className="panel">
          <div className="panel-head"><div><h2>yt-dlp versions</h2><p>Managed executable, fallback and update policy.</p></div>
            <span className="chip">{engine?.active ?? "Not installed"}</span></div>
          <div className="version-grid">
            <label>Update channel
              <select value={engine?.channel ?? "stable"} disabled={!engine || busy}
                onChange={event => void operation(() => post("/engines/yt-dlp/policy", token, {
                  channel: event.target.value as Channel, pinned: engine?.pinned ?? null,
                  auto_update: engine?.auto_update ?? true
                }))}>
                <option value="stable">Stable</option><option value="nightly">Nightly</option><option value="master">Master</option>
              </select>
            </label>
            <label>Version policy
              <select disabled={!engine || busy} value={engine?.pinned ?? "auto"}
                onChange={event => void operation(() => post("/engines/yt-dlp/policy", token, {
                  channel: engine?.channel ?? "stable",
                  pinned: event.target.value === "auto" ? null : event.target.value,
                  auto_update: event.target.value === "auto"
                }))}>
                <option value="auto">Automatic</option>
                {engine?.installed.map(v => <option key={v} value={v}>Pin {v}</option>)}
              </select>
            </label>
            <label>Install a release tag
              <input placeholder="e.g. 2026.01.01" value={version} onChange={event => setVersion(event.target.value)} />
            </label>
            <div className="version-actions">
              <button className="button secondary" disabled={busy || !version}
                onClick={() => void operation(() => post(`/engines/yt-dlp/install/${encodeURIComponent(version)}`, token,
                  { channel: engine?.channel ?? "stable", activate: false }))}>Install version</button>
              <button className="button secondary" disabled={busy || !engine?.fallback}
                onClick={() => void operation(() => post("/engines/yt-dlp/rollback", token))}>Rollback</button>
              <button className="button primary" disabled={busy}
                onClick={() => void operation(() => post("/engines/yt-dlp/update", token,
                  { channel: engine?.channel ?? "stable", activate: true }))}>Check updates</button>
            </div>
          </div>
          <div className="version-note">Fallback: {engine?.fallback ?? "—"} · Installed: {engine?.installed.join(", ") || "none"}</div>
        </section>
        <section className="panel">
          <div className="panel-head"><div><h2>Deno runtime versions</h2>
            <p>Automatically installed for YouTube downloads, with pinning and fallback.</p></div>
            <span className="chip">{deno?.active ?? "Not installed"}</span></div>
          <div className="version-grid">
            <label>Version policy
              <select disabled={!deno || busy} value={deno?.pinned ?? "auto"}
                onChange={event => void operation(() => post("/runtimes/deno/policy", token, {
                  pinned: event.target.value === "auto" ? null : event.target.value,
                  auto_update: event.target.value === "auto"
                }))}>
                <option value="auto">Automatic</option>
                {deno?.installed.map(v => <option key={v} value={v}>Pin {v}</option>)}
              </select>
            </label>
            <label>Install release tag
              <input placeholder="e.g. v2.9.8" value={denoVersion}
                onChange={event => setDenoVersion(event.target.value)} />
            </label>
            <div className="version-actions">
              <button className="button secondary" disabled={busy || !denoVersion}
                onClick={() => void operation(() => post(`/runtimes/deno/install/${encodeURIComponent(denoVersion)}`, token, {activate:false}))}>Install version</button>
              <button className="button secondary" disabled={busy || !deno?.fallback}
                onClick={() => void operation(() => post("/runtimes/deno/rollback", token))}>Rollback</button>
              <button className="button primary" disabled={busy}
                onClick={() => void operation(() => post("/runtimes/deno/update", token, {activate:true}))}>Check updates</button>
            </div>
          </div>
          <div className="version-note">Fallback: {deno?.fallback ?? "—"} · Installed: {deno?.installed.join(", ") || "none"}</div>
        </section>
        <section className="panel">
          <div className="panel-head"><div><h2>Engine runtime</h2>
            <p>Choose startup behavior and how long activated engines remain available.</p></div>
          </div>
          <div className="version-grid">
            <label>Startup mode
              <select disabled={!lifecycle || busy} value={lifecycle?.policy.startup ?? "cold"}
                onChange={event => void operation(async () => {
                  await api<EngineLifecycleStatus>("/engines/lifecycle", token,
                    { method: "PUT", body: JSON.stringify({
                      startup: event.target.value as StartupMode,
                      cold_retention: lifecycle?.policy.cold_retention ?? "persistent"
                    }), headers: {"Content-Type":"application/json"} });
                })}>
                <option value="hot">Hot · initialize installed engines at startup</option>
                <option value="cold">Cold · initialize when first used</option>
              </select>
            </label>
            <label>Cold engine retention
              <select disabled={!lifecycle || busy || lifecycle.policy.startup === "hot"}
                value={lifecycle?.policy.cold_retention ?? "persistent"}
                onChange={event => void operation(async () => {
                  await api<EngineLifecycleStatus>("/engines/lifecycle", token,
                    { method: "PUT", body: JSON.stringify({
                      startup: lifecycle?.policy.startup ?? "cold",
                      cold_retention: event.target.value as ColdRetention
                    }), headers: {"Content-Type":"application/json"} });
                })}>
                <option value="temporary">Temporary · release after last task</option>
                <option value="persistent">Persistent · keep activated engine ready</option>
              </select>
            </label>
          </div>
          <div className="version-note">
            Temporary torrent sessions stop seeding when the last active torrent completes.
            yt-dlp is a short-lived CLI process, so persistent mode prepares its adapter without keeping an idle yt-dlp process.
          </div>
          <div className="engine-runtime-list">
            {lifecycle?.engines.map(engine => <div key={engine.id} className="engine-runtime-item">
              <div><strong>{engine.id}</strong><span>{engine.kind.replaceAll("_", " ")} · {engine.ready ? "Ready" : "Not initialized"}</span></div>
              <button className="button secondary" disabled={busy || engine.ready}
                onClick={() => void operation(() => post(`/engines/${engine.id}/prepare`, token))}>Prepare</button>
            </div>)}
          </div>
        </section>
        {settingsOpen && <section className="panel settings-panel">
          <div className="panel-head"><h2>Appearance & connection</h2>
            <button className="button secondary" onClick={() => setSettingsOpen(false)}>Close</button></div>
          <div className="settings-fields">
            <label>Accent color<input type="color" value={accent} onChange={event => setAccent(event.target.value)} /></label>
            <form onSubmit={saveToken}><label>API access token (current tab session only)
              <input type="password" autoComplete="off" value={tokenDraft}
                onChange={event => setTokenDraft(event.target.value)} placeholder="Optional for local daemon" />
            </label><button className="button secondary" type="submit">Apply token</button></form>
          </div>
        </section>}
        <footer>Pre-alpha · More engines, installer packages and advanced layout editing are planned.</footer>
      </main>
    </div>
  );
}
function Stat({ label, value }: { label: string; value: string }) {
  return <div className="stat"><span>{label}</span><strong>{value}</strong></div>;
}
