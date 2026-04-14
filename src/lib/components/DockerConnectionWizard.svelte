<script lang="ts">
  import { api } from "$lib/api";
  import { connections } from "$lib/stores/connections.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { sshProfiles } from "$lib/stores/ssh-profiles.svelte";
  import { onMount } from "svelte";
  import ColorPicker from "./ColorPicker.svelte";
  import SshProfileSelector from "./SshProfileSelector.svelte";
  import type { ConnectionColorId, ContainerInfo, ContainerSummary } from "$lib/types";

  let {
    onclose,
  }: {
    onclose: () => void;
  } = $props();

  type Step = "ssh" | "container" | "credentials" | "test";
  type DbType = "postgres" | "mysql" | "mariadb" | "mssql" | "oracle" | "clickhouse";

  // Wizard state
  let step = $state<Step>("ssh");
  let sshProfileId = $state<string | null>(null);

  // null sshProfileId = local Docker; non-null = remote via SSH
  const isLocal = $derived(sshProfileId === null);
  let containerName = $state("");
  let containerInfo = $state<ContainerInfo | null>(null);
  let discovering = $state(false);
  let discoverError = $state("");

  // Running containers list (fetched when entering step 2)
  let runningContainers = $state<ContainerSummary[]>([]);
  let containersLoading = $state(false);

  // Only containers whose image suggests a known DB type
  const dbContainers = $derived(
    runningContainers.filter((c) => c.database_type_hint !== null),
  );

  // Credentials
  let name = $state("");
  let colorId = $state<ConnectionColorId>("blue");
  let groupId = $state<string | null>(null);
  let dbType = $state<DbType>("postgres");
  let containerPort = $state(5432);
  let database = $state("");
  let username = $state("");
  let password = $state("");
  // SSL: "prefer" = driver default, "disable" = no TLS, "require" = force TLS
  let sslMode = $state<"prefer" | "disable" | "require">("prefer");

  // Test / save
  let testStatus = $state<"idle" | "testing" | "success" | "error">("idle");
  let testMessage = $state("");
  let saving = $state(false);
  let saveError = $state("");

  onMount(async () => {
    if (sshProfiles.list.length === 0 && !sshProfiles.loading) {
      await sshProfiles.load();
    }
  });

  function defaultPortFor(dt: DbType): number {
    if (dt === "postgres") return 5432;
    if (dt === "mysql" || dt === "mariadb") return 3306;
    if (dt === "mssql") return 1433;
    if (dt === "oracle") return 1521;
    if (dt === "clickhouse") return 8123;
    return 5432;
  }

  function defaultSslModeFor(dt: DbType): "prefer" | "disable" | "require" {
    // MySQL 5.x / MariaDB use old TLS ciphers incompatible with rustls.
    // Connections are already secured by the SSH tunnel, so disable is safe.
    if (dt === "mysql" || dt === "mariadb") return "disable";
    return "prefer";
  }

  function onDbTypeChange() {
    containerPort = defaultPortFor(dbType);
    sslMode = defaultSslModeFor(dbType);
  }

  async function loadRunningContainers() {
    containersLoading = true;
    runningContainers = [];
    try {
      if (isLocal) {
        runningContainers = await api.invoke<ContainerSummary[]>("list_local_containers");
      } else {
        runningContainers = await api.invoke<ContainerSummary[]>("list_running_containers", {
          sshProfileId,
        });
      }
    } catch {
      // Silently ignore — manual entry still works
    } finally {
      containersLoading = false;
    }
  }

  function goToContainerStep() {
    step = "container";
    loadRunningContainers();
  }

  function selectContainer(c: ContainerSummary) {
    containerName = c.name;
    containerInfo = null;
    discoverError = "";
    handleDiscover();
  }

  async function handleDiscover() {
    if (!containerName.trim()) return;
    discovering = true;
    discoverError = "";
    containerInfo = null;
    try {
      if (isLocal) {
        containerInfo = await api.invoke<ContainerInfo>("discover_local_container", {
          containerName: containerName.trim(),
        });
      } else {
        containerInfo = await api.invoke<ContainerInfo>("discover_container", {
          sshProfileId,
          containerName: containerName.trim(),
        });
      }
      if (containerInfo.status !== "running") {
        discoverError = containerInfo.status === "stopped"
          ? `Container '${containerName}' is stopped. Start it and try again.`
          : `Container '${containerName}' not found. Check the name and try again.`;
        containerInfo = null;
        return;
      }
      // Auto-detect db type from hint
      if (containerInfo.database_type_hint) {
        const hint = containerInfo.database_type_hint;
        if (hint === "postgres") { dbType = "postgres"; containerPort = 5432; }
        else if (hint === "mysql") { dbType = "mysql"; containerPort = 3306; }
        else if (hint === "mariadb") { dbType = "mariadb"; containerPort = 3306; }
        else if (hint === "mssql") { dbType = "mssql"; containerPort = 1433; }
        else if (hint === "oracle") { dbType = "oracle"; containerPort = 1521; }
        else if (hint === "clickhouse") { dbType = "clickhouse"; containerPort = 8123; }
        sslMode = defaultSslModeFor(dbType);
      }
      // Default connection name
      if (!name) name = containerName.trim();
    } catch (e) {
      discoverError = String(e);
    } finally {
      discovering = false;
    }
  }

  function buildSslParam(dt: DbType, mode: "prefer" | "disable" | "require"): string {
    if (mode === "prefer") return "";
    if (dt === "postgres") {
      return mode === "disable" ? "?sslmode=disable" : "?sslmode=require";
    }
    if (dt === "mysql" || dt === "mariadb") {
      return mode === "disable" ? "?ssl-mode=disabled" : "?ssl-mode=required";
    }
    return "";
  }

  function buildUrl(): string {
    const scheme = dbType;
    const userPart = username
      ? password
        ? `${encodeURIComponent(username)}:${encodeURIComponent(password)}@`
        : `${encodeURIComponent(username)}@`
      : "";
    const dbPart = database ? `/${database}` : "";
    const sslParam = buildSslParam(dbType, sslMode);
    // Use a placeholder host — on actual connect, the backend re-discovers
    // the container IP and rewrites it before opening the pool.
    return `${scheme}://${userPart}localhost:${containerPort}${dbPart}${sslParam}`;
  }

  const canDiscover = $derived(containerName.trim().length > 0);
  const canProceedToTest = $derived(name.trim() !== "" && username.trim() !== "");

  async function handleTest() {
    if (!containerInfo) return;
    testStatus = "testing";
    testMessage = "";
    try {
      if (isLocal) {
        testMessage = await api.invoke<string>("test_local_docker_connection", {
          containerName: containerName.trim(),
          containerPort,
          url: buildUrl(),
        });
      } else {
        testMessage = await api.invoke<string>("test_docker_connection", {
          sshProfileId,
          containerName: containerName.trim(),
          containerPort,
          url: buildUrl(),
        });
      }
      testStatus = "success";
    } catch (e) {
      testMessage = String(e);
      testStatus = "error";
    }
  }

  async function handleSave() {
    if (!canProceedToTest || !containerInfo) return;
    saving = true;
    saveError = "";
    try {
      await connections.save({
        name: name.trim(),
        color_id: colorId,
        url: buildUrl(),
        ssh_profile_id: isLocal ? null : sshProfileId,
        group_id: groupId ?? null,
        connection_type: isLocal ? "local_docker_container" : "docker_container",
        container_name: containerName.trim(),
        container_port: containerPort,
      });
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

  function handleContainerNameKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && canDiscover && !discovering) handleDiscover();
  }

  // ── Error classification ──────────────────────────────────────────────────

  type DockerErrorKind =
    | "permission_denied"
    | "daemon_unreachable"
    | "network_isolated"
    | "not_found"
    | "stopped"
    | "timeout"
    | "invalid_name"
    | "ssh"
    | "other";

  interface ClassifiedError {
    kind: DockerErrorKind;
    /** Short hint shown below the error message; empty means no hint. */
    hint: string;
    /** True for errors that may resolve on retry without user action. */
    transient: boolean;
  }

  function classifyDockerError(msg: string): ClassifiedError {
    const m = msg.toLowerCase();
    if (m.includes("permission denied") || m.includes("docker access") || m.includes("got permission denied")) {
      return { kind: "permission_denied", transient: false,
        hint: "Add the SSH user to the 'docker' group on the server:\nsudo usermod -aG docker <username>  && newgrp docker" };
    }
    if (m.includes("docker daemon") || m.includes("cannot connect to the docker") || m.includes("daemon not responding")) {
      return { kind: "daemon_unreachable", transient: true,
        hint: "Check that Docker is running on the server:\nsudo systemctl status docker" };
    }
    if (m.includes("isolated network") || m.includes("docker network")) {
      return { kind: "network_isolated", transient: false,
        hint: "The container has no routable IP. Connect it to a bridge network:\ndocker network connect bridge <container>" };
    }
    if (m.includes("not running") || m.includes("is stopped")) {
      return { kind: "stopped", transient: false,
        hint: "Start the container, then retry discovery:\ndocker start <container>" };
    }
    if (m.includes("not found") || m.includes("no such object") || m.includes("check the name")) {
      return { kind: "not_found", transient: false,
        hint: "List all containers to verify the name:\ndocker ps -a --format '{{.Names}}'" };
    }
    if (m.includes("timed out") || m.includes("timeout")) {
      return { kind: "timeout", transient: true,
        hint: "The SSH command timed out. Check server connectivity and try again." };
    }
    if (m.includes("injection") || m.includes("invalid container name")) {
      return { kind: "invalid_name", transient: false,
        hint: "Container names may only contain letters, numbers, hyphens, underscores, and dots." };
    }
    if (m.includes("ssh") || m.includes("auth") || m.includes("handshake") || m.includes("connection refused")) {
      return { kind: "ssh", transient: true,
        hint: "Check the SSH profile settings and ensure the server is reachable." };
    }
    return { kind: "other", transient: false, hint: "" };
  }

  function classifyTestError(msg: string): ClassifiedError {
    const m = msg.toLowerCase();
    // TLS/SSL errors — "HandshakeFailure" is a TLS alert from the database,
    // not an SSH problem. Check before the generic SSH pattern.
    if (m.includes("handshakefailure") || m.includes("fatal alert") || m.includes("tls") || m.includes("ssl")) {
      return { kind: "other", transient: false,
        hint: "TLS/SSL negotiation with the database failed. If the database doesn't require TLS, try appending ?sslmode=disable to the connection URL." };
    }
    // SSH / tunnel setup errors
    if (m.includes("ssh") || m.includes("auth failed") || m.includes("authentication rejected") || m.includes("connection refused to ssh")) {
      return { kind: "ssh", transient: true,
        hint: "SSH tunnel failed to establish. Verify the SSH profile credentials." };
    }
    if (m.includes("connection refused") || m.includes("no route to host") || m.includes("timed out") || m.includes("timeout")) {
      return { kind: "timeout", transient: true,
        hint: "Could not reach the database port inside the container. Confirm the port and that the database is accepting connections." };
    }
    if (m.includes("password") || m.includes("authentication") || m.includes("login failed") || m.includes("access denied")) {
      return { kind: "permission_denied", transient: false,
        hint: "Check the database username and password." };
    }
    return { kind: "other", transient: false, hint: "" };
  }

  const discoverErrorInfo = $derived(discoverError ? classifyDockerError(discoverError) : null);
  // Always produce a classification when there is an error — hint may be empty.
  const testErrorInfo = $derived(testStatus === "error" ? classifyTestError(testMessage) : null);
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onclose}>
  <div class="dialog" onclick={(e: MouseEvent) => e.stopPropagation()}>
    <div class="dialog-header">
      <h2 class="dialog-title">New Docker Container Connection</h2>
      <div class="steps">
        <span class="step" class:active={step === "ssh"} class:done={step !== "ssh"}>1. SSH</span>
        <span class="step-sep">›</span>
        <span class="step" class:active={step === "container"} class:done={step === "credentials" || step === "test"}>2. Container</span>
        <span class="step-sep">›</span>
        <span class="step" class:active={step === "credentials"} class:done={step === "test"}>3. Credentials</span>
        <span class="step-sep">›</span>
        <span class="step" class:active={step === "test"}>4. Test</span>
      </div>
    </div>

    <!-- Step 1: SSH Profile -->
    {#if step === "ssh"}
      <div class="step-body">
        <p class="step-desc">Select an SSH profile to connect to a remote server, or leave "None" to use the local Docker socket.</p>
        <div class="field">
          <SshProfileSelector bind:value={sshProfileId} />
        </div>
        <div class="step-actions">
          <button class="btn btn-secondary" onclick={onclose}>Cancel</button>
          <button class="btn btn-primary" onclick={goToContainerStep}>Next</button>
        </div>
      </div>

    <!-- Step 2: Container -->
    {:else if step === "container"}
      <div class="step-body">
        <p class="step-desc">Select a detected database container or enter a name manually.</p>

        {#if containersLoading}
          <p class="containers-loading">Scanning for running containers…</p>
        {:else if dbContainers.length > 0}
          <div class="field">
            <span class="label">Detected database containers</span>
            <div class="container-list">
              {#each dbContainers as c (c.name)}
                <button
                  type="button"
                  class="container-card"
                  class:selected={containerName === c.name}
                  onclick={() => selectContainer(c)}
                >
                  <span class="container-card-name">{c.name}</span>
                  <span class="container-card-meta">
                    {#if c.database_type_hint}
                      <span class="container-card-hint">{c.database_type_hint}</span>
                    {/if}
                    <span class="container-card-image" title={c.image}>{c.image}</span>
                  </span>
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <label class="field">
          <span class="label">{dbContainers.length > 0 ? "Or enter name manually" : "Container name"}</span>
          <div class="input-row">
            <input
              type="text"
              bind:value={containerName}
              onkeydown={handleContainerNameKeydown}
              placeholder="my-database-container"
              class="input"
              autofocus
            />
            <button
              class="btn btn-primary"
              onclick={handleDiscover}
              disabled={!canDiscover || discovering}
            >
              {discovering ? "Discovering…" : "Discover"}
            </button>
          </div>
        </label>

        {#if discoverError && discoverErrorInfo}
          <div class="error-box">
            <span class="error-msg">{discoverError}</span>
            {#if discoverErrorInfo.hint}
              <pre class="error-hint">{discoverErrorInfo.hint}</pre>
            {/if}
            {#if discoverErrorInfo.transient}
              <span class="error-retry-note">This may be a temporary issue — click Discover to try again.</span>
            {/if}
          </div>
        {/if}

        {#if containerInfo}
          <div class="info-box success-box">
            <span class="info-label">Container found</span>
            <div class="info-row">
              <span class="info-key">IP address</span>
              <code class="info-val">{containerInfo.ip_address}</code>
            </div>
            {#if containerInfo.database_type_hint}
              <div class="info-row">
                <span class="info-key">Detected type</span>
                <span class="info-val">{containerInfo.database_type_hint}</span>
              </div>
            {/if}
          </div>
        {/if}

        <div class="step-actions">
          <button class="btn btn-secondary" onclick={() => (step = "ssh")}>Back</button>
          <button
            class="btn btn-primary"
            onclick={() => (step = "credentials")}
            disabled={!containerInfo}
          >
            Next
          </button>
        </div>
      </div>

    <!-- Step 3: Credentials -->
    {:else if step === "credentials"}
      <div class="step-body">
        <p class="step-desc">Configure the database connection details inside the container.</p>

        <label class="field">
          <span class="label">Connection name</span>
          <input type="text" bind:value={name} placeholder="My Container DB" class="input" autofocus />
        </label>

        <div class="field-row field-row-3">
          <label class="field">
            <span class="label">Database type</span>
            <select bind:value={dbType} onchange={onDbTypeChange} class="input select">
              <option value="postgres">PostgreSQL</option>
              <option value="mysql">MySQL</option>
              <option value="mariadb">MariaDB</option>
              <option value="mssql">MS SQL Server</option>
              <option value="oracle">Oracle</option>
              <option value="clickhouse">ClickHouse</option>
            </select>
          </label>

          <label class="field">
            <span class="label">Container port</span>
            <input
              type="number"
              bind:value={containerPort}
              min="1"
              max="65535"
              class="input"
            />
          </label>

          <label class="field">
            <span class="label">SSL</span>
            <select bind:value={sslMode} class="input select">
              <option value="prefer">Prefer (default)</option>
              <option value="disable">Disable</option>
              <option value="require">Require</option>
            </select>
          </label>
        </div>

        <label class="field">
          <span class="label">Database name</span>
          <input type="text" bind:value={database} placeholder="myapp" class="input" />
        </label>

        <div class="field-row">
          <label class="field">
            <span class="label">Username</span>
            <input type="text" bind:value={username} placeholder="admin" class="input" />
          </label>

          <label class="field">
            <span class="label">Password</span>
            <input type="password" bind:value={password} class="input" />
          </label>
        </div>

        <div class="field">
          <span class="label">Color</span>
          <ColorPicker bind:value={colorId} />
        </div>

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

        <div class="step-actions">
          <button class="btn btn-secondary" onclick={() => (step = "container")}>Back</button>
          <button
            class="btn btn-secondary"
            onclick={() => { step = "test"; handleTest(); }}
            disabled={!canProceedToTest}
          >
            Test Connection
          </button>
          <button
            class="btn btn-primary"
            onclick={handleSave}
            disabled={!canProceedToTest || saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>

    <!-- Step 4: Test -->
    {:else if step === "test"}
      <div class="step-body">
        <p class="step-desc">Testing the connection through the SSH tunnel to the container.</p>

        {#if testStatus === "testing"}
          <div class="info-box">Establishing SSH tunnel and testing connection…</div>
        {:else if testStatus === "success"}
          <div class="info-box success-box">{testMessage}</div>
        {:else if testStatus === "error"}
          <div class="error-box">
            <span class="error-msg">{testMessage || "Test failed with no error message."}</span>
            {#if testErrorInfo?.hint}
              <span class="error-hint-inline">{testErrorInfo.hint}</span>
            {/if}
          </div>
        {/if}

        {#if saveError}
          <div class="error-box"><span class="error-msg">{saveError}</span></div>
        {/if}

        <div class="step-actions">
          <button class="btn btn-secondary" onclick={() => (step = "credentials")}>Back</button>
          <button class="btn btn-secondary" onclick={handleTest} disabled={testStatus === "testing"}>
            {testStatus === "error" ? "Retry Test" : "Run Test"}
          </button>
          <button
            class="btn btn-primary"
            onclick={handleSave}
            disabled={!canProceedToTest || saving || testStatus === "testing"}
          >
            {saving ? "Saving…" : "Save Connection"}
          </button>
        </div>
      </div>
    {/if}
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

  .dialog {
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
    gap: 0;
  }

  .dialog-header {
    margin-bottom: 20px;
  }

  .dialog-title {
    margin: 0 0 12px 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text);
  }

  .steps {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }

  .step {
    color: var(--color-text-muted);
    font-weight: 500;
  }

  .step.active {
    color: var(--color-accent);
    font-weight: 600;
  }

  .step.done {
    color: oklch(0.5 0.12 145);
  }

  :global(.dark) .step.done {
    color: oklch(0.65 0.12 145);
  }

  .step-sep {
    color: var(--color-border);
  }

  .step-body {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .step-desc {
    margin: 0;
    font-size: 13px;
    color: var(--color-text-muted);
  }


  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .field-row-3 {
    grid-template-columns: 2fr 1fr 1fr;
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

  .input-row {
    display: flex;
    gap: 8px;
  }

  .input-row .input {
    flex: 1;
  }

  .info-box {
    padding: 10px 14px;
    border-radius: 8px;
    font-size: 13px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
  }

  .success-box {
    background: oklch(0.95 0.04 145);
    border-color: oklch(0.8 0.08 145);
    color: oklch(0.35 0.15 145);
  }

  :global(.dark) .success-box {
    background: oklch(0.22 0.04 145);
    border-color: oklch(0.35 0.08 145);
    color: oklch(0.75 0.12 145);
  }

  .error-box {
    padding: 10px 14px;
    border-radius: 8px;
    font-size: 13px;
    background: oklch(0.95 0.04 25);
    border: 1px solid oklch(0.8 0.08 25);
    color: oklch(0.4 0.15 25);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  :global(.dark) .error-box {
    background: oklch(0.22 0.04 25);
    border-color: oklch(0.35 0.08 25);
    color: oklch(0.75 0.12 25);
  }

  .error-msg {
    font-weight: 500;
  }

  .error-hint {
    margin: 0;
    font-family: monospace;
    font-size: 11px;
    white-space: pre-wrap;
    opacity: 0.85;
    background: rgba(0, 0, 0, 0.06);
    border-radius: 4px;
    padding: 4px 8px;
  }

  :global(.dark) .error-hint {
    background: rgba(255, 255, 255, 0.06);
  }

  .error-hint-inline {
    font-size: 12px;
    opacity: 0.85;
  }

  .error-retry-note {
    font-size: 12px;
    font-style: italic;
    opacity: 0.75;
  }

  .info-label {
    font-weight: 600;
    display: block;
    margin-bottom: 6px;
  }

  .info-row {
    display: flex;
    gap: 8px;
    font-size: 12px;
    margin-top: 4px;
  }

  .info-key {
    color: inherit;
    opacity: 0.7;
    min-width: 90px;
  }

  .info-val {
    font-family: monospace;
  }

  .step-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--color-border);
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
    border: 1px solid var(--color-border);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--color-border);
  }

  /* Container list */
  .containers-loading {
    font-size: 13px;
    color: var(--color-text-muted);
    margin: 0;
  }

  .container-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .container-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-bg);
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    color: var(--color-text);
    transition: border-color 0.15s, background-color 0.15s;
  }

  .container-card:hover {
    border-color: var(--color-accent);
    background: var(--color-surface-2);
  }

  .container-card.selected {
    border-color: var(--color-accent);
    background: color-mix(in oklch, var(--color-accent) 10%, var(--color-bg));
  }

  .container-card-name {
    font-weight: 500;
    flex-shrink: 0;
  }

  .container-card-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .container-card-hint {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .container-card-image {
    font-size: 11px;
    color: var(--color-text-muted);
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
