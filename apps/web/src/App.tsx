import { useCallback, useEffect, useMemo, useState } from "react";
import { api, post, prettyBytes } from "./api";
import type { Channel, DailyTransfer, Download, EngineState, Stats } from "./types";

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
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [source, setSource] = useState("");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [lastUpdated, setLastUpdated] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.setProperty("--accent", accent);
    localStorage.setItem(themeKey, theme);
    localStorage.setItem(accentKey, accent);
  }, [theme, accent]);
  const refresh = useCallback(async () => {
    const [downloadList, summary, state, days] = await Promise.all([
      api<Download[]>("/downloads", token), api<Stats>("/stats", token),
      api<EngineState>("/engines/yt-dlp", token),
      api<DailyTransfer[]>("/stats/daily", token)
    ]);
    setTasks(downloadList);
    setStats(summary);
    setDaily(days);
    setEngine(state);
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
      await post("/downloads", token, { source: source.trim(), engine: "yt-dlp" });
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
              const maximum = Math.max(1, ...recent.map(item => item.downloaded_bytes));
              return recent.length ? recent.map(item => <div className="history-col" key={item.day}
                title={`${item.day}: ${prettyBytes(item.downloaded_bytes)}`}>
                <div className="history-bar" style={{height: `${Math.max(4, 100 * item.downloaded_bytes / maximum)}%`}} />
                <span>{item.day.slice(5)}</span>
              </div>) : <p>No traffic recorded yet.</p>;
            })()}
          </div>
        </section>
        <section className="panel">
          <form onSubmit={addTask} className="add-form">
            <div className="form-copy"><h2>New download</h2><p>Paste a media URL. yt-dlp is currently the only executable engine.</p></div>
            <div className="input-row">
              <input aria-label="Media URL" type="url" required placeholder="https://..." value={source}
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
            <th>Source</th><th>Engine</th><th>State</th><th>Received</th><th>Created</th>
          </tr></thead><tbody>
            {visible.map(task => <tr key={task.id}>
              <td><span className="source-cell" title={task.source}>{task.source}</span>{task.error && <small className="error-text">{task.error}</small>}</td>
              <td>{task.engine}</td><td><span className={`status ${task.status}`}>{task.status}</span></td>
              <td>{prettyBytes(task.downloaded_bytes)}</td><td>{new Date(task.created_at).toLocaleString()}</td>
            </tr>)}
            {!visible.length && <tr><td colSpan={5} className="empty">No matching downloads yet.</td></tr>}
          </tbody></table></div>
        </section>
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
