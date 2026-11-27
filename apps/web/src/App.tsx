import { useCallback, useEffect, useMemo, useState } from "react";
import { api, post, prettyBytes } from "./api";
import type { Channel, DailyTransfer, Download, EngineState, Stats, TorrentDetails, DenoState, EngineLifecycleStatus, StartupMode, ColdRetention, QueueStatus, TransferSettings } from "./types";

const emptyStats: Stats = {
  total_downloaded_bytes: 0, total_uploaded_bytes: 0,
  total_tasks: 0, completed_tasks: 0, failed_tasks: 0, active_tasks: 0
};
const themeKey = "kordra-theme";
const accentKey = "kordra-accent";
type Filter = "all" | "active" | "queued" | "paused" | "completed" | "failed" | "cancelled";
type Page = "downloads" | "activity" | "engines" | "settings";
type Theme = "dark" | "light";

export default function App() {
  const [theme, setTheme] = useState<Theme>(() => localStorage.getItem(themeKey) === "light" ? "light" : "dark");
  const [accent, setAccent] = useState(() => localStorage.getItem(accentKey) ?? "#5285de");
  const [token, setToken] = useState(() => sessionStorage.getItem("kordra-api-token") ?? "");
  const [tokenDraft, setTokenDraft] = useState(token);
  const [tasks, setTasks] = useState<Download[]>([]);
  const [stats, setStats] = useState<Stats>(emptyStats);
  const [queue, setQueue] = useState<QueueStatus | null>(null);
  const [transferSettings, setTransferSettings] = useState<TransferSettings | null>(null);
  const [transferDraft, setTransferDraft] = useState<TransferSettings | null>(null);
  const [daily, setDaily] = useState<DailyTransfer[]>([]);
  const [engine, setEngine] = useState<EngineState | null>(null);
  const [deno, setDeno] = useState<DenoState | null>(null);
  const [lifecycle, setLifecycle] = useState<EngineLifecycleStatus | null>(null);
  const [denoVersion, setDenoVersion] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [source, setSource] = useState("");
  const [sourceEngine, setSourceEngine] = useState("yt-dlp");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [lastUpdated, setLastUpdated] = useState("");
  const [page, setPage] = useState<Page>("downloads");
  const [addOpen, setAddOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [engineFilter, setEngineFilter] = useState("all");
  const [sort, setSort] = useState<"newest" | "oldest" | "priority">("newest");
  const [selectedTorrent, setSelectedTorrent] = useState<{ taskId: string; details: TorrentDetails } | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.setProperty("--accent", accent);
    localStorage.setItem(themeKey, theme);
    localStorage.setItem(accentKey, accent);
  }, [theme, accent]);
  const refresh = useCallback(async () => {
    const [downloadList, summary, state, days, denoState, lifecycleStatus, queueStatus, transferPolicy] = await Promise.all([
      api<Download[]>("/downloads", token), api<Stats>("/stats", token),
      api<EngineState>("/engines/yt-dlp", token),
      api<DailyTransfer[]>("/stats/daily", token),
      api<DenoState>("/runtimes/deno", token),
      api<EngineLifecycleStatus>("/engines/lifecycle", token),
      api<QueueStatus>("/queue", token),
      api<TransferSettings>("/queue/settings", token)
    ]);
    setTasks(downloadList);
    setStats(summary);
    setDaily(days);
    setEngine(state);
    setDeno(denoState);
    setLifecycle(lifecycleStatus);
    setQueue(queueStatus);
    setTransferSettings(transferPolicy);
    setTransferDraft(previous => previous ?? transferPolicy);
    setLastUpdated(new Date().toLocaleTimeString());
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
      (filter === "active" && ["running", "cancelling"].includes(task.status)) ||
      (filter === "queued" && task.status === "queued") || (filter === "paused" && task.status === "paused") ||
      (filter === "completed" && task.status === "completed") ||
      (filter === "failed" && ["failed", "interrupted"].includes(task.status)) ||
      (filter === "cancelled" && task.status === "cancelled");
    return statusMatch && (engineFilter === "all" || task.engine === engineFilter) && `${task.source} ${task.engine} ${task.id}`.toLowerCase().includes(search.toLowerCase());
  }).sort((a,b) => sort === "priority" ? b.priority-a.priority || a.created_at.localeCompare(b.created_at) : sort === "oldest" ? a.created_at.localeCompare(b.created_at) : b.created_at.localeCompare(a.created_at)), [tasks, filter, search, engineFilter, sort]);

  async function operation(action: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await action(); await refresh(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  }
  async function addTask(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!source.trim()) return;
    await operation(async () => {
      await post("/downloads", token, { source: source.trim(), engine: sourceEngine });
      setSource(""); setAddOpen(false); setPage("downloads"); setFilter("all"); setEngineFilter("all"); setSearch("");
    });
  }
  function saveToken(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    sessionStorage.setItem("kordra-api-token", tokenDraft);
    setToken(tokenDraft);
    setError("");
  }

  const selected = tasks.find(t => t.id === selectedId);
  const nav = (next: Page) => { setPage(next); setSelectedId(null); setSelectedTorrent(null); };
  const selectTask = (task: Download) => {
    setSelectedId(task.id); setSelectedTorrent(null);
    if (task.engine === "librqbit" && ["running","paused","completed"].includes(task.status)) {
      void api<TorrentDetails>(`/downloads/${task.id}/torrent`, token).then(details => setSelectedTorrent({taskId:task.id,details})).catch(() => {});
    }
  };
  const activeTorrent = selectedTorrent?.taskId === selectedId;
  const count = (f: Filter) => f === "all" ? stats.total_tasks : f === "active" ? queue?.running ?? 0 : f === "queued" ? queue?.queued ?? 0 : f === "paused" ? queue?.paused ?? 0 : tasks.filter(t => f === "failed" ? ["failed","interrupted"].includes(t.status) : t.status === f).length;
  const action = (task: Download, name: string) => void operation(() => post(`/downloads/${task.id}/${name}`, token));
  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setAddOpen(false); setSelectedId(null); setSelectedTorrent(null); }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") { event.preventDefault(); setAddOpen(true); }
    };
    window.addEventListener("keydown",handle);
    return () => window.removeEventListener("keydown",handle);
  }, []);
  return <div className="workspace">
    <aside className="navigation">
      <div className="wordmark"><span className="mark"><Icon name="download"/></span><span>Kordra<small>Download manager</small></span></div>
      <button className="new-task" onClick={() => setAddOpen(true)}><Icon name="plus"/> New download <kbd>Ctrl N</kbd></button>
      <div className="nav-label">Library</div>
      <nav aria-label="Downloads">
        {([["all","All downloads","list"],["active","Downloading","download"],["queued","Waiting","clock"],["paused","Paused","pause"],["completed","Completed","check"],["failed","Failed","alert"],["cancelled","Cancelled","x"]] as const).map(([id,label,icon]) =>
          <button key={id} className={"nav-button "+(page==="downloads"&&filter===id?"chosen":"")} onClick={() => { nav("downloads"); setFilter(id); }}><Icon name={icon}/><span>{label}</span><small>{count(id)}</small></button>
        )}
      </nav>
      <div className="nav-label">Workspace</div>
      <nav aria-label="Workspace">
        {([["activity","Statistics","chart"],["engines","Engines","layers"],["settings","Settings","settings"]] as const).map(([id,label,icon]) =>
          <button key={id} className={"nav-button "+(page===id?"chosen":"")} onClick={() => nav(id)}><Icon name={icon}/><span>{label}</span></button>
        )}
      </nav>
      <div className="nav-bottom"><button className="nav-button" onClick={() => setTheme(t => t==="dark"?"light":"dark")}><Icon name={theme==="dark"?"sun":"moon"}/><span>{theme==="dark"?"Light appearance":"Dark appearance"}</span></button><span className="build-label">KORDRA / PRE-ALPHA</span></div>
    </aside>
    <main className="main-work">
      <header className="workspace-header"><div><span className="crumb">Kordra / {page==="downloads"?"Library":"Workspace"}</span><h1>{page==="downloads"?({all:"All downloads",active:"Downloading",queued:"Waiting",paused:"Paused",completed:"Completed",failed:"Failed",cancelled:"Cancelled"}[filter]):({activity:"Statistics",engines:"Engines",settings:"Settings"}[page])}</h1></div>
      <div className="header-controls"><button className="icon-button" title="Refresh" aria-label="Refresh" onClick={() => void operation(refresh)} disabled={busy}><Icon name="refresh"/></button><button className="button primary" onClick={() => setAddOpen(true)}><Icon name="plus"/> Add download</button></div></header>
      {error && <div role="alert" className="warning"><Icon name="alert"/><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError("")}><Icon name="x"/></button></div>}
      {page==="downloads" ? <>
        <div className="queue-toolbar"><div className="search-box"><Icon name="search"/><input aria-label="Search downloads" placeholder="Find a download" value={search} onChange={e=>setSearch(e.target.value)}/>{search&&<button className="icon-button" aria-label="Clear search" onClick={()=>setSearch("")}><Icon name="x"/></button>}</div>
          <select aria-label="Filter by engine" value={engineFilter} onChange={e=>setEngineFilter(e.target.value)}><option value="all">All engines</option><option value="librqbit">BitTorrent</option><option value="yt-dlp">yt-dlp</option><option value="aria2">aria2</option><option value="gallery-dl">gallery-dl</option></select>
          <select aria-label="Sort downloads" value={sort} onChange={e=>setSort(e.target.value as typeof sort)}><option value="newest">Newest first</option><option value="oldest">Oldest first</option><option value="priority">Priority</option></select>
          <span className="results-count">{visible.length} shown</span>
        </div>
        <div className="queue-space">
          <div className="table-wrap"><table className="download-table"><thead><tr><th>Name</th><th>Status</th><th>Progress</th><th>Received</th><th>Uploaded</th><th>Priority</th><th aria-label="Actions"/></tr></thead><tbody>
          {visible.map(task=>{ const percent=task.status==="completed"?100:task.total_bytes>0?Math.min(100,100*task.downloaded_bytes/task.total_bytes):null;
            return <tr key={task.id} className={selectedId===task.id?"selected-row":""} onClick={()=>selectTask(task)}>
              <td><button className="task-name" onClick={e=>{e.stopPropagation();selectTask(task);}}><span className="file-symbol"><Icon name={task.engine==="librqbit"?"layers":task.engine==="yt-dlp"?"film":"file"}/></span><span className="file-text"><strong title={task.source}>{taskName(task.source)}</strong><small>{task.engine==="librqbit"?"BitTorrent":task.engine} · {hostName(task.source)}</small></span></button></td>
              <td><span className={"task-state "+task.status}><i/>{statusName(task.status)}</span></td>
              <td><div className="progress-cell"><span>{percent===null?"—":percent.toFixed(percent===100?0:1)+"%"}</span><div className="progress-track"><div className={task.status==="completed"?"complete":""} style={{width:(percent??0)+"%"}}/></div></div></td>
              <td className="numeric">{prettyBytes(task.downloaded_bytes)}<small>{task.total_bytes>0?"of "+prettyBytes(task.total_bytes):""}</small></td>
              <td className="numeric">{prettyBytes(task.uploaded_bytes)}</td>
              <td onClick={e=>e.stopPropagation()}><select className="queue-priority" aria-label={`Priority of ${task.id}`} value={task.priority??0} disabled={busy||!["queued","paused"].includes(task.status)} onChange={e=>void operation(()=>api<Download>(`/downloads/${task.id}/priority`,token,{method:"PUT",body:JSON.stringify({priority:Number(e.target.value)})}))}><option value={2}>Very high</option><option value={1}>High</option><option value={0}>Normal</option><option value={-1}>Low</option><option value={-2}>Very low</option></select></td>
              <td><div className="row-actions" onClick={e=>e.stopPropagation()}>
              {(task.status==="queued"||(task.status==="running"&&task.engine==="librqbit"))&&<button className="icon-button" aria-label="Pause" title="Pause" disabled={busy} onClick={()=>action(task,"pause")}><Icon name="pause"/></button>}
              {task.status==="paused"&&<button className="icon-button" aria-label="Resume" title="Resume" disabled={busy} onClick={()=>action(task,"resume")}><Icon name="play"/></button>}
              {["failed","interrupted"].includes(task.status)&&<button className="icon-button" title="Retry" aria-label="Retry" disabled={busy} onClick={()=>action(task,"retry")}><Icon name="refresh"/></button>}
              {["queued","paused","running"].includes(task.status)&&<button className="icon-button stop-button" title="Cancel (keep files)" aria-label="Cancel download" disabled={busy} onClick={()=>{if(window.confirm("Cancel this download? Existing files will be kept."))action(task,"cancel");}}><Icon name="x"/></button>}
              <button className="icon-button" title="Details" aria-label="Details" onClick={()=>selectTask(task)}><Icon name="chevron"/></button></div></td>
            </tr>})}
          </tbody></table></div>
          {!visible.length&&<div className="empty-state"><div className="empty-symbol"><Icon name="download"/></div><h2>{tasks.length?"No matching downloads":"Ready for your first download"}</h2><p>{tasks.length?"Change the filters or search for another file.":"Add a link or magnet to start a transfer."}</p><button className="button secondary" onClick={()=>tasks.length?(setSearch(""),setFilter("all"),setEngineFilter("all")):setAddOpen(true)}>{tasks.length?"Reset filters":"Add download"}</button></div>}
        </div>
      </> : <div className="page-body">
      {page==="activity" && <><section className="summary-line"><Stat label="Total received" value={prettyBytes(stats.total_downloaded_bytes)}/><Stat label="Total uploaded" value={prettyBytes(stats.total_uploaded_bytes)}/><Stat label="Completed" value={String(stats.completed_tasks)}/><Stat label="Failed attempts" value={String(stats.failed_tasks)}/></section>        <section className="panel">
          <div className="panel-head"><div><h2>Transfer history</h2><p>Daily downloaded bytes, persisted by the backend.</p></div></div>
          <div className="history">
            {(() => {
              const recent = [...daily].reverse().slice(-14);
              const maximum = Math.max(1, ...recent.flatMap(item => [item.downloaded_bytes, item.uploaded_bytes]));
              return recent.length ? recent.map(item => <div className="history-col" key={item.day}
                title={`${item.day}: Downloaded ${prettyBytes(item.downloaded_bytes)} · Uploaded ${prettyBytes(item.uploaded_bytes)}`}>
                <div className="history-bars">
                  <div className="history-bar downloaded" style={{height: `${100 * item.downloaded_bytes / maximum}%`}} />
                  <div className="history-bar uploaded" style={{height: `${100 * item.uploaded_bytes / maximum}%`}} />
                </div>
                <span>{item.day.slice(5)}</span>
              </div>) : <p>No traffic recorded yet.</p>;
            })()}
          </div>
          <p className="history-legend"><span className="legend-down" /> Downloaded <span className="legend-up" /> Uploaded</p>
        </section>
</>}
      {page==="engines" && <>        <section className="panel">
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
</>}
      {page==="settings" && <>        <section className="panel">
          <div className="panel-head"><div><h2>Concurrency &amp; speed limits</h2>
            <p>Live controls for active torrent traffic and future yt-dlp tasks.</p></div>
          </div>
          <div className="version-grid">
            <label>Parallel downloads (1–32)
              <input type="number" min={1} max={32} step={1}
                disabled={busy || !transferDraft} value={transferDraft?.concurrency ?? 2}
                onChange={event => setTransferDraft(current => current ? {
                  ...current, concurrency: Number(event.target.value)
                } : null)} />
            </label>
            <label>yt-dlp download (KiB/s, 0 = unlimited)
              <input type="number" min={0} max={976562} step={1}
                disabled={busy || !transferDraft}
                value={Math.round((transferDraft?.yt_dlp_download_bps ?? 0) / 1024)}
                onChange={event => setTransferDraft(current => current ? {
                  ...current, yt_dlp_download_bps: Number(event.target.value) > 0
                    ? Number(event.target.value) * 1024 : null
                } : null)} />
            </label>
            <label>BitTorrent download (KiB/s, 0 = unlimited)
              <input type="number" min={0} max={4194303} step={1}
                disabled={busy || !transferDraft}
                value={Math.round((transferDraft?.torrent_download_bps ?? 0) / 1024)}
                onChange={event => setTransferDraft(current => current ? {
                  ...current, torrent_download_bps: Number(event.target.value) > 0
                    ? Number(event.target.value) * 1024 : null
                } : null)} />
            </label>
            <label>BitTorrent upload (KiB/s, 0 = unlimited)
              <input type="number" min={0} max={4194303} step={1}
                disabled={busy || !transferDraft}
                value={Math.round((transferDraft?.torrent_upload_bps ?? 0) / 1024)}
                onChange={event => setTransferDraft(current => current ? {
                  ...current, torrent_upload_bps: Number(event.target.value) > 0
                    ? Number(event.target.value) * 1024 : null
                } : null)} />
            </label>
            <div className="version-actions">
              <button className="button primary" disabled={busy || !transferDraft || !Number.isInteger(transferDraft.concurrency)}
                onClick={() => void operation(async () => {
                  const saved = await api<TransferSettings>("/queue/settings", token, {
                    method: "PUT",
                    headers: {"Content-Type": "application/json"},
                    body: JSON.stringify(transferDraft)
                  });
                  setTransferSettings(saved);
                  setTransferDraft(saved);
                })}>Apply live limits</button>
              <button className="button secondary" disabled={busy || !transferSettings}
                onClick={() => setTransferDraft(transferSettings)}>Discard edits</button>
            </div>
          </div>
          <p className="version-note">
            Changes persist after restart. Lowering parallelism does not interrupt running tasks.
            Torrent limits apply immediately to the shared session (minimum 64 KiB/s).
            yt-dlp limits apply to newly started jobs (minimum 1 KiB/s), not existing subprocesses.
          </p>
        </section>
        <section className="panel">
          <div className="panel-head"><h2>Appearance & connection</h2>
            </div>
          <div className="settings-fields">
            <label>Accent color<input type="color" value={accent} onChange={event => setAccent(event.target.value)} /></label>
            <form onSubmit={saveToken}><label>API access token (current tab session only)
              <input type="password" autoComplete="off" value={tokenDraft}
                onChange={event => setTokenDraft(event.target.value)} placeholder="Optional for local daemon" />
            </label><button className="button secondary" type="submit">Apply token</button></form>
          </div>
        </section>
</>}
      </div>}
      <footer className="statusbar"><span><i className={"connection-dot "+(!lastUpdated?"offline":"")}/>{lastUpdated?"Last sync "+lastUpdated:"Connecting"}</span><span>{queue?.running??0} running / {queue?.max_concurrent??2} slots</span><span>{queue?.queued??0} waiting</span><div className="status-spacer"/><span><Icon name="download"/> {prettyBytes(stats.total_downloaded_bytes)}</span><span><Icon name="upload"/> {prettyBytes(stats.total_uploaded_bytes)}</span></footer>
    </main>
    {page==="downloads"&&selected&&<aside className="inspector" aria-label="Download details"><div className="inspector-top"><span>Download details</span><button className="icon-button" aria-label="Close details" onClick={()=>{setSelectedId(null);setSelectedTorrent(null);}}><Icon name="x"/></button></div><div className="inspector-body">
      <div className="inspector-file"><span className="file-symbol"><Icon name="file"/></span><h2>{activeTorrent?selectedTorrent!.details.name||taskName(selected.source):taskName(selected.source)}</h2><span className={"task-state "+selected.status}><i/>{statusName(selected.status)}</span></div>
      <dl><dt>Engine</dt><dd>{selected.engine}</dd><dt>Received</dt><dd>{prettyBytes(selected.downloaded_bytes)}</dd><dt>Uploaded</dt><dd>{prettyBytes(selected.uploaded_bytes)}</dd><dt>Attempts</dt><dd>{selected.attempts}</dd><dt>Added</dt><dd>{new Date(selected.created_at).toLocaleString()}</dd><dt>Source</dt><dd className="break">{selected.source}</dd></dl>
      {selected.error&&<p role="alert" className="error-text">{selected.error}</p>}
      {selectedTorrent && activeTorrent && <section className="torrent-details">
          <div className="panel-head">
            <div><h2>{selectedTorrent.details.name || "Torrent details"}</h2>
              <p>Info hash: {selectedTorrent.details.info_hash}</p></div>
            <button className="button secondary" onClick={() => { setSelectedTorrent(null); setSelectedId(null); }}>Close</button>
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
          <h3>Files ({selectedTorrent.details.files.length})</h3>
          <div className="torrent-files">
            {selectedTorrent.details.files.map(file => <label className="torrent-file" key={file.index}>
              <input type="checkbox" checked={file.included}
                disabled={busy || selectedTorrent.details.stats.finished}
                onChange={event => void operation(async () => {
                  const selected = selectedTorrent.details.files.filter(item => item.included).map(item => item.index);
                  const next = event.target.checked ? [...selected, file.index] : selected.filter(idx => idx !== file.index);
                  const details = await post<TorrentDetails>(`/downloads/${selectedTorrent.taskId}/torrent/files`, token, {only_files: next});
                  setSelectedTorrent({ taskId: selectedTorrent.taskId, details });
                })} />
              <span className="torrent-file-name" title={file.name}>{file.name}</span>
              <span className="torrent-file-size">{prettyBytes(file.downloaded_bytes)} / {prettyBytes(file.length)}</span>
            </label>)}
          </div>
          <p>File selection updates the active torrent without deleting existing data. At least one file must remain selected.</p>
          {selectedTorrent.details.stats.finished &&
            <button className="button secondary" disabled={busy}
              onClick={() => void operation(async () => {
                await post(`/downloads/${selectedTorrent.taskId}/stop-seeding`, token);
                setSelectedTorrent(null);
              })}>Stop seeding (keep files)</button>}
        </section>}
      {selected.engine==="librqbit"&&!activeTorrent&&<button className="button secondary" onClick={()=>void operation(async()=>{const details=await api<TorrentDetails>(`/downloads/${selected.id}/torrent`,token);setSelectedTorrent({taskId:selected.id,details});})}>Refresh torrent details</button>}
    </div></aside>}
    {addOpen&&<div className="modal-shade" onMouseDown={e=>{if(e.target===e.currentTarget&&!busy)setAddOpen(false);}}><section className="add-dialog" role="dialog" aria-modal="true" aria-labelledby="add-title"><div className="dialog-head"><h2 id="add-title">New download</h2><button className="icon-button" aria-label="Close dialog" onClick={()=>setAddOpen(false)}><Icon name="x"/></button></div><form onSubmit={addTask}><label>Source URL or magnet<input autoFocus required value={source} placeholder="Paste a link…" onChange={e=>{setSource(e.target.value);if(e.target.value.trim().startsWith("magnet:"))setSourceEngine("librqbit");}}/></label><label>Download engine<select value={sourceEngine} onChange={e=>setSourceEngine(e.target.value)}><option value="yt-dlp">yt-dlp — Media</option><option value="librqbit">librqbit — BitTorrent</option><option value="aria2">aria2 — Direct files</option><option value="gallery-dl">gallery-dl — Galleries</option></select></label><div className="dialog-actions"><button type="button" className="button secondary" onClick={()=>setAddOpen(false)}>Cancel</button><button type="submit" className="button primary" disabled={busy||!source.trim()}>Add download</button></div></form></section></div>}
  </div>;
}
function Stat({label,value}:{label:string;value:string}){return <div className="stat"><span>{label}</span><strong>{value}</strong></div>;}
function taskName(source:string){try{const u=new URL(source);if(u.protocol==="magnet:")return u.searchParams.get("dn")||"Magnet link";return decodeURIComponent(u.pathname.split("/").filter(Boolean).pop()||u.hostname);}catch{return source;}}
function hostName(source:string){try{return new URL(source).hostname||"Magnet";}catch{return "Source";}}
function statusName(value:string){return ({running:"Downloading",queued:"Waiting",paused:"Paused",completed:"Completed",failed:"Failed",interrupted:"Interrupted",cancelled:"Cancelled",cancelling:"Cancelling"} as Record<string,string>)[value]??value;}
const icons:Record<string,string>={download:"M12 3v12m-5-5 5 5 5-5M4 16v4h16v-4",upload:"M12 16V4m-5 5 5-5 5 5M4 16v4h16v-4",plus:"M12 5v14M5 12h14",list:"M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01",clock:"M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0",pause:"M9 5v14M15 5v14",play:"m8 5 12 7-12 7z",check:"m5 12 4 4L19 6",alert:"M12 8v5m0 3h.01M10 3 2 18q-1 2 2 2h16q3 0 2-2L14 3q-2-3-4 0",x:"m6 6 12 12M18 6 6 18",chart:"M4 4v16h16M8 15v-3m4 3V7m4 8v-5",layers:"m12 3 9 5-9 5-9-5zm-9 9 9 5 9-5M3 16l9 5 9-5",settings:"M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M12 2v3m0 14v3M2 12h3m14 0h3M5 5l2 2m10 10 2 2M19 5l-2 2M7 17l-2 2",sun:"M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M12 2v3m0 14v3M2 12h3m14 0h3M5 5l2 2m10 10 2 2M19 5l-2 2M7 17l-2 2",moon:"M20 15a9 9 0 0 1-11-11 9 9 0 1 0 11 11",refresh:"M20 7v5h-5M4 17v-5h5M5 9a8 8 0 0 1 13-4l2 3M19 15a8 8 0 0 1-13 4l-2-3",search:"M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0",file:"M5 3h9l5 5v13H5zM14 3v6h5",film:"M3 5h18v14H3zM7 5v14m10-14v14M3 12h4m10 0h4",chevron:"m9 6 6 6-6 6"};
function Icon({name}:{name:string}){return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={icons[name]||icons.file}/></svg>;}
