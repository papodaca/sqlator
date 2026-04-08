<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import ColorPicker from "./ColorPicker.svelte";
  import SshProfileSelector from "./SshProfileSelector.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import type { ConnectionColorId, ConnectionInfo, ParsedConnectionUrl } from "$lib/types";

  let {
    editing = null,
    onclose,
  }: {
    editing?: ConnectionInfo | null;
    onclose: () => void;
  } = $props();

  type DbType = "postgres" | "mysql" | "mariadb" | "sqlite" | "mssql" | "oracle" | "clickhouse";
  type Tab = "url" | "manual";

  // ── Form-level state ─────────────────────────────────────────────────────────
  let name = $state("");
  let colorId = $state<ConnectionColorId>("blue");
  let sshProfileId = $state<string | null>(null);
  let groupId = $state<string | null>(null);
  let activeTab = $state<Tab>("url");
  let saving = $state(false);
  let saveError = $state("");
  let testStatus = $state<"idle" | "testing" | "success" | "error">("idle");
  let testMessage = $state("");

  // ── URL tab state ─────────────────────────────────────────────────────────────
  let url = $state("");

  // ── Manual tab state ──────────────────────────────────────────────────────────
  let dbType = $state<DbType>("postgres");
  let host = $state("");
  let port = $state(5432);
  let database = $state("");
  let username = $state("");
  let password = $state("");

  // Track which editing ID is currently loaded
  let lastEditingId: string | undefined = undefined;

  $effect(() => {
    const currentId = editing?.id;
    if (currentId !== lastEditingId) {
      name = editing?.name ?? "";
      colorId = (editing?.color_id as ConnectionColorId) ?? "blue";
      sshProfileId = editing?.ssh_profile_id ?? null;
      groupId = editing?.group_id ?? null;
      url = "";
      testStatus = "idle";
      testMessage = "";
      saveError = "";
      lastEditingId = currentId;

      // Pre-populate manual fields from ConnectionInfo (no password — must re-enter)
      if (editing) {
        dbType = (editing.db_type as DbType) ?? "postgres";
        host = editing.host ?? "";
        port = editing.port ?? defaultPortFor(dbType);
        database = editing.database ?? "";
        username = editing.username ?? "";
        password = "";
      } else {
        dbType = "postgres";
        host = "";
        port = 5432;
        database = "";
        username = "";
        password = "";
      }
    }
  });

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function defaultPortFor(dt: DbType): number {
    if (dt === "postgres") return 5432;
    if (dt === "mysql" || dt === "mariadb") return 3306;
    if (dt === "mssql") return 1433;
    if (dt === "oracle") return 1521;
    if (dt === "clickhouse") return 8123;
    return 0;
  }

  /** Build a postgres/mysql/sqlite URL from the manual fields */
  function buildUrlFromFields(): string {
    if (!host && dbType !== "sqlite") return "";
    const scheme = dbType;
    const userPart = username
      ? password
        ? `${encodeURIComponent(username)}:${encodeURIComponent(password)}@`
        : `${encodeURIComponent(username)}@`
      : "";
    const portPart = port && port !== defaultPortFor(dbType) ? `:${port}` : `:${port}`;
    const hostPart = dbType === "sqlite" ? "" : `${host}${portPart}`;
    const dbPart = database ? `/${database}` : "";
    return `${scheme}://${userPart}${hostPart}${dbPart}`;
  }

  /** Parse a connection URL into manual fields. Returns false on failure. */
  function applyUrlToFields(rawUrl: string): boolean {
    try {
      const u = new URL(rawUrl);
      const scheme = u.protocol.replace(":", "");
      const dt: DbType =
        scheme === "postgresql" || scheme === "postgres"
          ? "postgres"
          : scheme === "mysql"
            ? "mysql"
            : scheme === "mariadb"
              ? "mariadb"
              : scheme === "sqlite"
                ? "sqlite"
                : scheme === "mssql" || scheme === "sqlserver" || scheme === "tds"
                  ? "mssql"
                  : scheme === "oracle"
                  ? "oracle"
                  : scheme === "clickhouse"
                  ? "clickhouse"
                  : "postgres";

      dbType = dt;
      host = u.hostname;
      port = u.port ? parseInt(u.port) : defaultPortFor(dt);
      database = u.pathname.replace(/^\//, "");
      username = decodeURIComponent(u.username);
      password = decodeURIComponent(u.password);
      return true;
    } catch {
      return false;
    }
  }

  // ── Tab switching ─────────────────────────────────────────────────────────────

  function switchTab(tab: Tab) {
    if (tab === activeTab) return;
    if (tab === "manual" && url.trim()) {
      applyUrlToFields(url.trim());
    }
    activeTab = tab;
  }

  // ── URL tab handlers ──────────────────────────────────────────────────────────

  function onUrlBlur() {
    if (url.trim()) applyUrlToFields(url.trim());
  }

  function onUrlPaste(e: ClipboardEvent) {
    const pasted = e.clipboardData?.getData("text") ?? "";
    if (pasted.includes("://")) {
      // Let the paste land in the input, then sync after the next tick
      setTimeout(() => {
        const trimmed = url.trim();
        if (trimmed) applyUrlToFields(trimmed);
      }, 0);
    }
  }

  // ── Manual tab handlers ───────────────────────────────────────────────────────

  function onFieldChange() {
    url = buildUrlFromFields();
  }

  function onDbTypeChange() {
    port = defaultPortFor(dbType);
    onFieldChange();
  }

  // ── Effective URL (what gets submitted) ───────────────────────────────────────

  const effectiveUrl = $derived(
    activeTab === "url" ? url.trim() : buildUrlFromFields(),
  );

  const canSave = $derived(name.trim() !== "" && effectiveUrl !== "");
  const canTest = $derived(effectiveUrl !== "" && testStatus !== "testing");

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function handleTest() {
    if (!effectiveUrl) return;
    testStatus = "testing";
    testMessage = "";
    try {
      if (sshProfileId) {
        testMessage = await invoke<string>("test_connection_with_ssh", {
          url: effectiveUrl,
          sshProfileId,
        });
      } else {
        testMessage = await connections.test(effectiveUrl);
      }
      testStatus = "success";
    } catch (e) {
      testMessage = String(e);
      testStatus = "error";
    }
  }

  async function handleSave() {
    if (!canSave) return;
    saving = true;
    saveError = "";
    try {
      const config = {
        name: name.trim(),
        color_id: colorId,
        url: effectiveUrl,
        ssh_profile_id: sshProfileId ?? null,
        group_id: groupId ?? null,
      };
      if (editing) {
        await connections.update(editing.id, config);
      } else {
        await connections.save(config);
      }
      onclose();
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onclose}>
  <div class="form-dialog" onclick={(e: MouseEvent) => e.stopPropagation()}>
    <h2 class="form-title">{editing ? "Edit" : "New"} Connection</h2>

    <!-- Name -->
    <label class="field">
      <span class="label">Name</span>
      <input
        type="text"
        bind:value={name}
        placeholder="My Database"
        class="input"
        autofocus
      />
    </label>

    <!-- Tabs -->
    <div class="tabs">
      <button
        class="tab-btn"
        class:active={activeTab === "url"}
        type="button"
        onclick={() => switchTab("url")}
      >
        Quick (URL)
      </button>
      <button
        class="tab-btn"
        class:active={activeTab === "manual"}
        type="button"
        onclick={() => switchTab("manual")}
      >
        Advanced (Fields)
      </button>
    </div>

    <!-- URL tab -->
    {#if activeTab === "url"}
      <label class="field">
        <span class="label">Connection URL</span>
        <input
          type="text"
          bind:value={url}
          onblur={onUrlBlur}
          onpaste={onUrlPaste}
          placeholder="postgres://user:pass@host:5432/dbname"
          class="input font-mono text-sm"
        />
      </label>
      {#if editing}
        <p class="hint">Re-enter the full URL to test or change the connection. Previous URL: <code class="masked">{editing.masked_url}</code></p>
      {/if}

    <!-- Manual tab -->
    {:else}
      <div class="manual-grid">
        <label class="field">
          <span class="label">Database type</span>
          <select
            bind:value={dbType}
            onchange={onDbTypeChange}
            class="input select"
          >
            <option value="postgres">PostgreSQL</option>
            <option value="mysql">MySQL</option>
            <option value="mariadb">MariaDB</option>
            <option value="sqlite">SQLite</option>
            <option value="mssql">MS SQL Server</option>
            <option value="oracle">Oracle (experimental)</option>
            <option value="clickhouse">ClickHouse</option>
          </select>
        </label>

        {#if dbType !== "sqlite"}
          <label class="field">
            <span class="label">Host</span>
            <input
              type="text"
              bind:value={host}
              oninput={onFieldChange}
              placeholder="localhost"
              class="input"
            />
          </label>

          <label class="field field-port">
            <span class="label">Port</span>
            <input
              type="number"
              bind:value={port}
              oninput={onFieldChange}
              min="1"
              max="65535"
              class="input"
            />
          </label>
        {/if}

        <label class="field field-wide">
          <span class="label">{dbType === "sqlite" ? "File path" : "Database"}</span>
          <input
            type="text"
            bind:value={database}
            oninput={onFieldChange}
            placeholder={dbType === "sqlite" ? "/path/to/db.sqlite" : "myapp"}
            class="input"
          />
        </label>

        {#if dbType !== "sqlite"}
          <label class="field">
            <span class="label">Username</span>
            <input
              type="text"
              bind:value={username}
              oninput={onFieldChange}
              placeholder="admin"
              class="input"
            />
          </label>

          <label class="field">
            <span class="label">Password</span>
            <input
              type="password"
              bind:value={password}
              oninput={onFieldChange}
              placeholder={editing ? "(unchanged)" : ""}
              class="input"
            />
          </label>
        {/if}
      </div>
    {/if}

    <!-- Color -->
    <div class="field">
      <span class="label">Color</span>
      <ColorPicker bind:value={colorId} />
    </div>

    <!-- Group -->
    {#if groups.list.length > 0}
      <label class="field">
        <span class="label">Group</span>
        <select class="input select" bind:value={groupId}>
          <option value={null}>— No group —</option>
          {#each groups.list as g (g.id)}
            <option value={g.id}>{g.name}</option>
          {/each}
        </select>
      </label>
    {/if}

    <!-- SSH profile -->
    <div class="field">
      <SshProfileSelector bind:value={sshProfileId} />
    </div>

    <!-- Test result -->
    {#if testStatus !== "idle"}
      <div
        class="test-result"
        class:success={testStatus === "success"}
        class:error={testStatus === "error"}
      >
        {#if testStatus === "testing"}
          {sshProfileId ? "Establishing SSH tunnel…" : "Testing connection…"}
        {:else}
          {testMessage}
        {/if}
      </div>
    {/if}

    {#if saveError}
      <div class="test-result error">{saveError}</div>
    {/if}

    <!-- Actions -->
    <div class="actions">
      <button class="btn btn-secondary" type="button" onclick={onclose}>
        Cancel
      </button>
      <button
        class="btn btn-secondary"
        type="button"
        onclick={handleTest}
        disabled={!canTest}
      >
        Test Connection
      </button>
      <button
        class="btn btn-primary"
        type="button"
        onclick={handleSave}
        disabled={!canSave || saving}
      >
        {saving ? "Saving…" : "Save"}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .form-dialog {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 24px;
    width: 520px;
    max-width: 90vw;
    max-height: 90vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .form-title {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text);
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 0;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    overflow: hidden;
  }

  .tab-btn {
    flex: 1;
    padding: 7px 12px;
    font-size: 13px;
    font-weight: 500;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: background-color 0.15s, color 0.15s;
  }

  .tab-btn:not(:last-child) {
    border-right: 1px solid var(--color-border);
  }

  .tab-btn:hover:not(.active) {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .tab-btn.active {
    background: var(--color-accent);
    color: white;
  }

  /* Fields */
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-muted);
  }

  .input {
    padding: 8px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    color: var(--color-text);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
    width: 100%;
    box-sizing: border-box;
  }

  .input:focus {
    border-color: var(--color-accent);
  }

  .select {
    appearance: none;
    cursor: pointer;
  }

  /* Manual grid */
  .manual-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .field-wide {
    grid-column: 1 / -1;
  }

  /* Hint text */
  .hint {
    font-size: 12px;
    color: var(--color-text-muted);
    margin: 0;
  }

  .masked {
    font-family: monospace;
    font-size: 11px;
    color: var(--color-text-muted);
  }

  /* Test result */
  .test-result {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }

  .test-result.success {
    background: oklch(0.95 0.05 145);
    color: oklch(0.35 0.15 145);
  }

  :global(.dark) .test-result.success {
    background: oklch(0.25 0.05 145);
    color: oklch(0.75 0.12 145);
  }

  .test-result.error {
    background: oklch(0.95 0.05 25);
    color: oklch(0.4 0.15 25);
  }

  :global(.dark) .test-result.error {
    background: oklch(0.25 0.05 25);
    color: oklch(0.75 0.12 25);
  }

  /* Actions */
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
  }

  .btn {
    padding: 8px 16px;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: background-color 0.15s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: var(--color-accent);
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-secondary {
    background: var(--color-surface-2);
    color: var(--color-text);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--color-border);
  }

  .font-mono {
    font-family: monospace;
  }

  .text-sm {
    font-size: 13px;
  }
</style>
