<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    describeError,
    type InventoryItem,
    type BackendStatus,
    type ResidueGraph,
    type Artifact,
    type RemovalPlan,
    type RemovalJob,
    type HistoryItem,
  } from "$lib/api";

  // ---------------- theme ----------------
  let theme = $state<"light" | "dark">("light");
  function initTheme() {
    const saved = localStorage.getItem("ar-theme");
    theme =
      saved === "light" || saved === "dark"
        ? saved
        : matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light";
  }
  function toggleTheme() {
    theme = theme === "dark" ? "light" : "dark";
    localStorage.setItem("ar-theme", theme);
  }
  $effect(() => {
    document.documentElement.dataset.theme = theme;
  });

  // ---------------- inventory ----------------
  let items = $state<InventoryItem[]>([]);
  let backends = $state<BackendStatus[]>([]);
  let dbWritable = $state(false);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let query = $state("");

  // ---------------- scan / plan / job ----------------
  let selected = $state<InventoryItem | null>(null);
  let graph = $state<ResidueGraph | null>(null);
  let scanning = $state(false);
  let scanError = $state<string | null>(null);
  let plan = $state<RemovalPlan | null>(null);
  let acceptedRisk = $state<Set<string>>(new Set());
  let planError = $state<string | null>(null);
  let working = $state(false);
  let job = $state<RemovalJob | null>(null);
  let jobError = $state<string | null>(null);

  // ---------------- modals ----------------
  let aboutOpen = $state(false);
  let historyOpen = $state(false);
  let history = $state<HistoryItem[]>([]);
  let undoMsg = $state<string | null>(null);

  const verdictClass: Record<string, string> = {
    safe: "ok",
    risky: "warn",
    blocked: "lock",
    "manual-review": "lock",
  };
  const stepClass: Record<string, string> = {
    succeeded: "ok",
    failed: "lock",
    running: "warn",
    pending: "",
    skipped: "",
  };

  let filtered = $derived(
    query.trim() === ""
      ? items
      : items.filter((i) => i.name.toLowerCase().includes(query.trim().toLowerCase()))
  );
  let grouped = $derived(groupArtifacts(graph?.artifacts ?? []));

  let stats = $derived({
    total: items.length,
    removable: items.filter((i) => !i.isProtected).length,
    protected: items.filter((i) => i.isProtected).length,
  });
  const STEPS = ["Scan", "Plan", "Approve", "Remove"];
  let stepIndex = $derived(
    !graph ? 0 : !plan ? 1 : plan.status !== "approved" ? 2 : 3
  );

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      aboutOpen = false;
      historyOpen = false;
    }
  }

  async function load() {
    loading = true;
    error = null;
    try {
      const [health, inv] = await Promise.all([api.health(), api.listInventory()]);
      backends = health.backends;
      dbWritable = health.dbWritable;
      items = inv.items;
    } catch (e) {
      error = describeError(e);
    } finally {
      loading = false;
    }
  }

  async function scan(item: InventoryItem) {
    if (item.isProtected) return;
    selected = item;
    graph = null;
    scanError = null;
    plan = null;
    planError = null;
    job = null;
    jobError = null;
    scanning = true;
    try {
      graph = await api.createScan(item.canonicalAppId);
    } catch (e) {
      scanError = describeError(e);
    } finally {
      scanning = false;
    }
  }

  async function composePlan() {
    if (!selected || !graph) return;
    plan = null;
    planError = null;
    job = null;
    jobError = null;
    working = true;
    try {
      plan = await api.createPlan(selected.canonicalAppId, graph.scanVersion);
      acceptedRisk = new Set();
    } catch (e) {
      planError = describeError(e);
    } finally {
      working = false;
    }
  }

  function toggleRisk(id: string) {
    const next = new Set(acceptedRisk);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    acceptedRisk = next;
  }

  async function doApprove() {
    if (!plan) return;
    planError = null;
    working = true;
    try {
      plan = await api.approvePlan(plan.planId, [...acceptedRisk]);
    } catch (e) {
      planError = describeError(e);
    } finally {
      working = false;
    }
  }

  async function createJob() {
    if (!plan) return;
    job = null;
    jobError = null;
    working = true;
    try {
      job = await api.createJob(plan.planId);
    } catch (e) {
      jobError = describeError(e);
    } finally {
      working = false;
    }
  }

  async function beginRemoval() {
    if (!job) return;
    jobError = null;
    working = true;
    try {
      job = await api.beginJob(job.jobId);
    } catch (e) {
      jobError = describeError(e);
    } finally {
      working = false;
    }
  }

  async function openHistory() {
    historyOpen = true;
    undoMsg = null;
    try {
      history = (await api.getHistory()).items;
    } catch (e) {
      undoMsg = describeError(e);
    }
  }

  async function doUndo(auditRecordId: string) {
    undoMsg = null;
    try {
      const r = await api.undoRemoval(auditRecordId);
      undoMsg = `Restored ${r.restoredFiles} file(s); reinstalled ${r.reinstalledPackages.length}${
        r.deferredPackages.length ? `; deferred ${r.deferredPackages.length}` : ""
      }.`;
      history = (await api.getHistory()).items;
    } catch (e) {
      undoMsg = describeError(e);
    }
  }

  function groupArtifacts(arts: Artifact[]): Array<[string, Artifact[]]> {
    const map = new Map<string, Artifact[]>();
    for (const a of arts) {
      const list = map.get(a.category) ?? [];
      list.push(a);
      map.set(a.category, list);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }

  onMount(() => {
    initTheme();
    load();
  });
</script>

<svelte:window onkeydown={onKey} />

<main>
  <header class="appbar">
    <div class="brand">
      <img class="logo" src="/logo.png" alt="App Remover" />
      <div>
        <h1>App Remover</h1>
        <p class="sub">Safely remove applications and their residue</p>
      </div>
    </div>
    <div class="actions">
      <button class="icon" onclick={toggleTheme} title="Toggle light / dark" aria-label="Toggle theme">
        {theme === "dark" ? "☀️" : "🌙"}
      </button>
      <button onclick={() => (aboutOpen = true)}>About</button>
      <button onclick={openHistory}>History</button>
      <button class="primary" onclick={load} disabled={loading}>
        {loading ? "Loading…" : "Refresh"}
      </button>
    </div>
  </header>

  <div class="meta">
    <span class="dot" class:ok={dbWritable} class:bad={!dbWritable}></span>
    <span class="muted">db {dbWritable ? "writable" : "read-only"}</span>
    <span class="sep"></span>
    <span class="muted">{stats.total} apps</span>
    <span class="ok">{stats.removable} removable</span>
    <span class="warn">{stats.protected} protected</span>
    <span class="sep"></span>
    <span class="muted">{backends.filter((b) => b.present).length}/{backends.length} sources</span>
  </div>

  {#if error}
    <p class="error box">{error}</p>
  {/if}

  <div class="grid">
    <!-- INVENTORY -->
    <section class="panel">
      <div class="panel-head">
        <h2>Installed apps</h2>
        <span class="count">{filtered.length} / {items.length}</span>
      </div>
      <div class="toolbar">
        <input type="search" placeholder="Filter packages…" bind:value={query} aria-label="Filter packages" />
      </div>
      {#if loading && items.length === 0}
        <div class="state"><div class="spinner"></div><p>Loading installed applications…</p></div>
      {:else if filtered.length === 0}
        <div class="state"><p class="big">🔍</p><p>No packages match “{query}”.</p></div>
      {:else}
        <div class="table-wrap">
          <table>
            <thead>
              <tr><th>Application</th><th>Status</th></tr>
            </thead>
            <tbody>
              {#each filtered as item (item.canonicalAppId)}
                <tr
                  class:selected={selected?.canonicalAppId === item.canonicalAppId}
                  class:disabled={item.isProtected}
                  onclick={() => scan(item)}
                >
                  <td>
                    <div class="name">{item.name}</div>
                    <div class="submeta">
                      <span>{item.version ?? "—"}</span>
                      {#if item.installMethods.length}<span class="dim">·</span><span class="cap">{item.installMethods.join(", ")}</span>{/if}
                    </div>
                  </td>
                  <td>
                    {#if item.isProtected}
                      <span class="badge lock">🔒 protected</span>
                    {:else}
                      <span class="badge ok">removable</span>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <!-- DETAIL -->
    <section class="panel detail">
      {#if !selected}
        <div class="state"><p class="big">👈</p><p>Select a package to scan its residue and plan a removal.</p></div>
      {:else if scanning}
        <div class="state"><div class="spinner"></div><p>Scanning <strong>{selected.name}</strong>…</p></div>
      {:else if scanError}
        <p class="error box">{scanError}</p>
      {:else if graph}
        <div class="detail-head">
          <h2>{selected.name}</h2>
          <span class="tag">scan v{graph.scanVersion}</span>
        </div>
        <ol class="stepper" aria-label="Removal workflow">
          {#each STEPS as s, i}
            <li class:active={i === stepIndex} class:done={i < stepIndex}>
              <span class="stepnum">{i < stepIndex ? "✓" : i + 1}</span>{s}
            </li>
          {/each}
        </ol>
        <div class="counts">
          <span class="pill ok">● exclusive {graph.counts.exclusive}</span>
          <span class="pill warn">● shared {graph.counts.shared}</span>
          <span class="pill sys">● system {graph.counts.system}</span>
          <span class="pill">{graph.artifacts.length} artifacts</span>
        </div>

        {#if graph.runtimeWarnings.length > 0}
          <ul class="warnings">
            {#each graph.runtimeWarnings as w}<li>⚠ {w.detail}</li>{/each}
          </ul>
        {/if}

        <div class="groups">
          {#each grouped as [category, arts]}
            <details>
              <summary>{category} <span class="muted">({arts.length})</span></summary>
              <ul class="artifacts">
                {#each arts.slice(0, 100) as a}
                  <li>
                    <span class="mono target" title={a.target}>{a.target}</span>
                    <span class="badge mini">{a.scope ?? "system"}</span>
                  </li>
                {/each}
                {#if arts.length > 100}<li class="muted">… {arts.length - 100} more</li>{/if}
              </ul>
            </details>
          {/each}
        </div>

        <div class="plan">
          {#if !plan}
            <button class="primary" onclick={composePlan} disabled={working}>
              {working ? "Composing…" : "Compose removal plan"}
            </button>
          {:else}
            <h3>Removal plan <span class="badge status-{plan.status}">{plan.status}</span></h3>
            {#if plan.blockedReasons.length > 0}
              <ul class="warnings">
                {#each plan.blockedReasons as r}<li>⛔ {r}</li>{/each}
              </ul>
            {/if}
            <ul class="ops">
              {#each plan.operations as op (op.operationId)}
                <li>
                  <span class="badge {verdictClass[op.verdict] ?? ''}">{op.verdict}</span>
                  <span class="mono op-target" title={op.targetRef}>{op.action} → {op.targetRef}</span>
                  {#if op.verdict === "risky" && plan.status === "draft"}
                    <label class="accept">
                      <input type="checkbox" checked={acceptedRisk.has(op.operationId)} onchange={() => toggleRisk(op.operationId)} /> accept
                    </label>
                  {/if}
                  {#if op.impact.length}<span class="muted small">impact {op.impact.length}</span>{/if}
                </li>
              {/each}
            </ul>
            {#if plan.status === "draft"}
              <button class="primary" onclick={doApprove} disabled={working}>
                {working ? "Approving…" : "Approve plan"}
              </button>
            {:else}
              <p class="muted">✓ approved — ready for execution</p>
              <div class="job">
                {#if !job}
                  <button class="primary" onclick={createJob} disabled={working}>
                    Create job (capture snapshot)
                  </button>
                {:else}
                  <div class="jobrow">
                    <span class="badge status-{job.status}">{job.status}</span>
                    {#if job.snapshotId}<span class="muted small">snapshot ✓</span>{/if}
                  </div>
                  {#if job.steps.length}
                    <ul class="ops">
                      {#each job.steps as s (s.stepId)}
                        <li>
                          <span class="badge {stepClass[s.status] ?? ''}">{s.status}</span>
                          <span class="mono small">{s.operationId}</span>
                          {#if s.error}<span class="error small">{s.error.message}</span>{/if}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                  {#if job.status === "created"}
                    <button class="primary danger" onclick={beginRemoval} disabled={working}>
                      {working ? "Removing…" : "▶ Begin removal (prompts for root)"}
                    </button>
                  {:else if job.status === "completed"}
                    <p class="ok">✓ removed</p>
                  {:else if job.status === "rolled-back"}
                    <p class="warn">↩ rolled back — package restored</p>
                  {:else if job.status === "failed"}
                    <p class="error">✗ failed: {job.failure?.message ?? "see steps"}</p>
                  {/if}
                {/if}
                {#if jobError}<p class="error small">{jobError}</p>{/if}
              </div>
            {/if}
          {/if}
          {#if planError}<p class="error">{planError}</p>{/if}
        </div>
      {/if}
    </section>
  </div>
</main>

<!-- ABOUT -->
{#if aboutOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="overlay" onclick={() => (aboutOpen = false)} role="presentation">
    <div class="modal" tabindex="-1" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-label="About App Remover">
      <div class="modal-head">
        <h2>About App Remover</h2>
        <button class="icon" onclick={() => (aboutOpen = false)} aria-label="Close">✕</button>
      </div>
      <img class="hero" src="/logo-full.png" alt="App Remover 3D logo" />
      <p><strong>App Remover</strong> <span class="muted">v0.1.0</span> — safely remove applications and their residue across package managers.</p>
      <p class="muted">Plan, snapshot, remove (via polkit), and undo — with an append-only audit trail.</p>
      <h3>Supported sources</h3>
      <div class="chips">
        {#each backends as b}<span class="chip" class:on={b.present}>{b.backend}</span>{/each}
      </div>
      <h3>Safety</h3>
      <ul class="tight">
        <li>Protected system packages can't be removed.</li>
        <li>Every removal is snapshotted first; failures auto-roll back.</li>
        <li>Removals need your password (polkit) for system packages.</li>
      </ul>
      <p class="muted small">User manual: <code>/usr/lib/App Remover/USER_MANUAL.md</code></p>
    </div>
  </div>
{/if}

<!-- HISTORY -->
{#if historyOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="overlay" onclick={() => (historyOpen = false)} role="presentation">
    <div class="modal wide" tabindex="-1" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-label="History">
      <div class="modal-head">
        <h2>History</h2>
        <button class="icon" onclick={() => (historyOpen = false)} aria-label="Close">✕</button>
      </div>
      {#if undoMsg}<p class="muted small">{undoMsg}</p>{/if}
      {#if history.length === 0}
        <p class="muted">No removals yet.</p>
      {:else}
        <div class="table-wrap">
          <table>
            <thead><tr><th>App</th><th>When</th><th>Outcome</th><th></th></tr></thead>
            <tbody>
              {#each history as h (h.auditRecordId)}
                <tr>
                  <td class="name">{h.appName ?? "—"}</td>
                  <td class="mono small">{new Date(h.createdAt).toLocaleString()}</td>
                  <td><span class="badge status-{h.outcome}">{h.outcome}</span></td>
                  <td>
                    {#if h.undoable}
                      <button class="small-btn" onclick={() => doUndo(h.auditRecordId)}>Undo</button>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  :root {
    --bg: #f5f7fb;
    --surface: #ffffff;
    --surface-2: #f1f4fa;
    --fg: #14161c;
    --muted: #5b6472;
    --border: #e7eaf1;
    --accent: #3b6fe0;
    --ok: #198a4a;
    --warn: #b07300;
    --danger: #d23f3f;
    --row-hover: rgba(20, 30, 60, 0.045);
    --overlay: rgba(18, 24, 38, 0.5);
    --shadow: 0 1px 2px rgba(16, 24, 40, 0.05), 0 6px 18px rgba(16, 24, 40, 0.06);
  }
  :root[data-theme="dark"] {
    --bg: #15171c;
    --surface: #1c1f26;
    --surface-2: #232730;
    --fg: #e8eaee;
    --muted: #9aa3b2;
    --border: #2c313c;
    --accent: #5b8def;
    --ok: #3fb950;
    --warn: #d4b73a;
    --danger: #e06b5a;
    --row-hover: rgba(255, 255, 255, 0.06);
    --overlay: rgba(0, 0, 0, 0.6);
    --shadow: 0 1px 2px rgba(0, 0, 0, 0.4), 0 10px 30px rgba(0, 0, 0, 0.45);
  }

  :global(html) { color-scheme: light dark; }
  :global(body) { margin: 0; }

  main {
    font-family: Inter, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
    color: var(--fg);
    background: var(--bg);
    max-width: 1240px;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }

  /* appbar */
  .appbar { display: flex; align-items: center; justify-content: space-between; gap: 1rem; flex-wrap: wrap; }
  .brand { display: flex; align-items: center; gap: 0.7rem; }
  .logo { height: 40px; width: auto; display: block; }
  h1 { margin: 0; font-size: 1.35rem; letter-spacing: -0.01em; }
  .sub { margin: 0; color: var(--muted); font-size: 0.8rem; }
  .actions { display: flex; gap: 0.4rem; align-items: center; }

  .dot { width: 0.55rem; height: 0.55rem; border-radius: 50%; display: inline-block; }
  .dot.ok { background: var(--ok); }
  .dot.bad { background: var(--danger); }
  .sep { width: 1px; height: 1rem; background: var(--border); margin: 0 0.25rem; }
  .chip { padding: 0.1rem 0.5rem; border-radius: 999px; border: 1px solid var(--border); opacity: 0.45; font-size: 0.7rem; }
  .chip.on { opacity: 1; border-color: color-mix(in srgb, var(--ok) 50%, transparent); }

  /* layout */
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; align-items: start; }
  @media (max-width: 880px) { .grid { grid-template-columns: 1fr; } }
  .panel { background: var(--surface); border: 1px solid var(--border); border-radius: 14px; padding: 0.85rem 1rem; min-height: 360px; box-shadow: var(--shadow); }
  .panel-head { display: flex; align-items: baseline; justify-content: space-between; margin-bottom: 0.6rem; }
  .panel-head h2 { margin: 0; font-size: 0.95rem; }
  .count { font-size: 0.75rem; color: var(--muted); }
  .toolbar { margin-bottom: 0.5rem; }

  /* inputs */
  input[type="search"] {
    width: 100%; box-sizing: border-box; padding: 0.5rem 0.75rem; border-radius: 9px;
    border: 1px solid var(--border); background: var(--surface-2); color: var(--fg);
  }
  input[type="search"]:focus { outline: 2px solid var(--accent); outline-offset: -1px; border-color: var(--accent); }

  /* tables */
  .table-wrap { max-height: 62vh; overflow: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  th, td { text-align: left; padding: 0.42rem 0.5rem; border-bottom: 1px solid var(--border); }
  th { font-size: 0.68rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); position: sticky; top: 0; background: var(--surface); }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: var(--row-hover); }
  tr.selected { background: color-mix(in srgb, var(--accent) 16%, transparent); }
  tr.disabled { cursor: not-allowed; opacity: 0.65; }
  .name { font-weight: 600; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.78rem; color: var(--muted); }

  /* badges / pills */
  .badge { padding: 0.08rem 0.45rem; border-radius: 999px; font-size: 0.68rem; white-space: nowrap; }
  .badge.ok { background: color-mix(in srgb, var(--ok) 22%, transparent); color: var(--ok); }
  .badge.warn { background: color-mix(in srgb, var(--warn) 24%, transparent); color: var(--warn); }
  .badge.lock { background: var(--surface-2); color: var(--muted); }
  .badge.mini { background: var(--surface-2); font-size: 0.62rem; color: var(--muted); }
  .counts { display: flex; flex-wrap: wrap; gap: 0.4rem; margin: 0.2rem 0 0.7rem; }
  .pill { padding: 0.12rem 0.55rem; border-radius: 999px; font-size: 0.72rem; border: 1px solid var(--border); color: var(--muted); }
  .pill.ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 45%, transparent); }
  .pill.warn { color: var(--warn); border-color: color-mix(in srgb, var(--warn) 45%, transparent); }
  .pill.sys { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 45%, transparent); }
  .detail-head { display: flex; align-items: baseline; gap: 0.5rem; }
  .detail-head h2 { margin: 0 0 0.4rem; font-size: 1.05rem; }
  .tag { font-size: 0.7rem; color: var(--muted); border: 1px solid var(--border); padding: 0.05rem 0.4rem; border-radius: 6px; }

  /* warnings / groups */
  .warnings { margin: 0 0 0.7rem; padding: 0.45rem 0.8rem; border-radius: 9px; background: color-mix(in srgb, var(--warn) 14%, transparent); border: 1px solid color-mix(in srgb, var(--warn) 35%, transparent); font-size: 0.8rem; list-style: none; }
  .groups details { border-top: 1px solid var(--border); padding: 0.3rem 0; }
  .groups summary { cursor: pointer; font-size: 0.8rem; text-transform: capitalize; }
  .artifacts { list-style: none; padding: 0.25rem 0 0; margin: 0; max-height: 190px; overflow: auto; }
  .artifacts li { display: flex; align-items: center; gap: 0.5rem; padding: 0.1rem 0; }
  .target { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* buttons */
  button {
    padding: 0.4rem 0.85rem; border-radius: 9px; border: 1px solid var(--border);
    background: var(--surface-2); color: var(--fg); cursor: pointer; font: inherit; font-size: 0.82rem;
  }
  button:hover:not(:disabled) { border-color: var(--accent); }
  button:disabled { opacity: 0.5; cursor: default; }
  button.icon { padding: 0.4rem 0.6rem; }
  .primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  .primary:hover:not(:disabled) { filter: brightness(1.08); border-color: var(--accent); }
  .danger { background: var(--danger); border-color: var(--danger); color: #fff; }
  .small-btn { padding: 0.2rem 0.6rem; font-size: 0.74rem; border-radius: 7px; }

  /* plan / job */
  .plan { margin-top: 0.8rem; border-top: 1px solid var(--border); padding-top: 0.7rem; }
  h3 { margin: 0 0 0.4rem; font-size: 0.92rem; }
  .ops { list-style: none; padding: 0; margin: 0 0 0.5rem; max-height: 220px; overflow: auto; }
  .ops li { display: flex; align-items: center; gap: 0.4rem; padding: 0.2rem 0; font-size: 0.8rem; }
  .op-target { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .accept { font-size: 0.72rem; color: var(--muted); display: inline-flex; align-items: center; gap: 0.2rem; }
  .job { margin-top: 0.5rem; }
  .jobrow { display: flex; align-items: center; gap: 0.5rem; margin: 0.3rem 0; }
  .status-approved, .status-completed { background: color-mix(in srgb, var(--ok) 22%, transparent); color: var(--ok); }
  .status-draft, .status-created { background: var(--surface-2); color: var(--muted); }
  .status-rolled-back { background: color-mix(in srgb, var(--warn) 24%, transparent); color: var(--warn); }
  .status-failed { background: color-mix(in srgb, var(--danger) 22%, transparent); color: var(--danger); }
  .status-running { background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent); }

  /* states */
  .state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 0.4rem; color: var(--muted); text-align: center; padding: 2.5rem 1rem; }
  .state .big { font-size: 1.8rem; margin: 0; }
  .spinner { width: 1.5rem; height: 1.5rem; border: 3px solid var(--border); border-top-color: var(--accent); border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .muted { color: var(--muted); }
  .small { font-size: 0.72rem; }
  .ok { color: var(--ok); }
  .warn { color: var(--warn); }
  .error { color: var(--danger); }
  .box { padding: 0.6rem 0.8rem; border-radius: 9px; background: color-mix(in srgb, var(--danger) 12%, transparent); border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent); }
  code { font-family: ui-monospace, monospace; font-size: 0.8rem; background: var(--surface-2); padding: 0.1rem 0.35rem; border-radius: 5px; }

  /* modals */
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: flex; align-items: center; justify-content: center; z-index: 50; padding: 1rem; }
  .modal { background: var(--surface); border: 1px solid var(--border); border-radius: 14px; padding: 1.1rem 1.2rem; max-width: 460px; width: 100%; box-shadow: 0 20px 50px rgba(0,0,0,0.35); }
  .modal.wide { max-width: 640px; }
  .modal-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 0.7rem; }
  .modal h2 { margin: 0; font-size: 1.05rem; }
  .modal h3 { margin: 0.9rem 0 0.35rem; font-size: 0.82rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); }
  .chips { display: flex; flex-wrap: wrap; gap: 0.3rem; }
  ul.tight { margin: 0.3rem 0; padding-left: 1.1rem; font-size: 0.82rem; color: var(--muted); }
  ul.tight li { margin: 0.15rem 0; }

  /* workflow stepper */
  .stepper { display: flex; list-style: none; padding: 0; margin: 0 0 0.8rem; gap: 0.3rem; }
  .stepper li { display: flex; align-items: center; gap: 0.3rem; flex: 1; font-size: 0.72rem; color: var(--muted); padding: 0.35rem 0.45rem; border-radius: 8px; background: var(--surface-2); border: 1px solid var(--border); white-space: nowrap; }
  .stepper li.active { color: var(--accent); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); font-weight: 600; }
  .stepper li.done { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 40%, transparent); }
  .stepnum { width: 1.15rem; height: 1.15rem; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; font-size: 0.65rem; background: var(--border); color: var(--fg); flex-shrink: 0; }
  .stepper li.active .stepnum { background: var(--accent); color: #fff; }
  .stepper li.done .stepnum { background: var(--ok); color: #fff; }

  /* about hero */
  .hero { width: 100%; max-height: 160px; object-fit: contain; margin: 0 0 0.6rem; }

  /* slim meta line + table subtitle (decluttered header) */
  .meta { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; margin: 0.65rem 0 1rem; font-size: 0.8rem; }
  .meta .muted { color: var(--muted); }
  .meta .ok { color: var(--ok); }
  .meta .warn { color: var(--warn); }
  .meta .sep { width: 1px; height: 0.95rem; background: var(--border); }
  .submeta { display: flex; gap: 0.35rem; font-size: 0.72rem; color: var(--muted); margin-top: 0.1rem; }
  .submeta .cap { text-transform: capitalize; }
  .dim { opacity: 0.5; }
</style>
