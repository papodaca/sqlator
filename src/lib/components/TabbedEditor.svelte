<script lang="ts">
  import { api } from "$lib/api";
  import { tabs } from "$lib/stores/tabs.svelte";
  import { connections } from "$lib/stores/connections.svelte";
  import { serverMode } from "$lib/stores/server-mode.svelte";
  import ConnectionTabBar from "./ConnectionTabBar.svelte";
  import QueryTabBar from "./QueryTabBar.svelte";
  import SqlEditor from "./SqlEditor.svelte";
  import ResultPane from "./ResultPane.svelte";
  import EditorToolbar from "./EditorToolbar.svelte";
  import EnhancedGrid from "./EnhancedGrid.svelte";
  import type { TableBrowseState } from "$lib/types";

  // Expose a way for the sidebar to open the connection form
  let { onopenconnectionform }: { onopenconnectionform: () => void } = $props();

  async function handleSelectConnection(connectionId: string) {
    tabs.setActiveConnection(connectionId);
  }

  async function handleCloseConnection(connectionId: string) {
    tabs.closeConnection(connectionId);
    await api.invoke("disconnect_database", { id: connectionId }).catch(() => {});
  }

  function handleNewConnectionTab() {
    onopenconnectionform();
  }

  function handleNewQueryTab() {
    const id = tabs.activeConnectionId;
    if (id) tabs.createQueryTab(id);
  }

  function handleSelectQueryTab(queryTabId: string) {
    const id = tabs.activeConnectionId;
    if (id) tabs.setActiveQueryTab(id, queryTabId);
  }

  function handleCloseQueryTab(queryTabId: string) {
    const id = tabs.activeConnectionId;
    if (id) tabs.closeQueryTab(id, queryTabId);
  }

  const activeConnectionTab = $derived(tabs.activeConnectionTab);
  const activeQueryTab = $derived(tabs.activeQueryTab);
</script>

<div class="tabbed-editor">
  {#if tabs.connectionTabs.length > 0}
    {#if !serverMode.isSingleDb}
      <ConnectionTabBar
        connectionTabs={tabs.connectionTabs}
        activeConnectionId={tabs.activeConnectionId}
        onselect={handleSelectConnection}
        onclose={handleCloseConnection}
        onnew={handleNewConnectionTab}
      />
    {/if}

    {#if activeConnectionTab}
      <QueryTabBar
        connectionId={activeConnectionTab.connectionId}
        queryTabs={activeConnectionTab.queryTabs}
        activeId={activeConnectionTab.activeQueryTabId}
        onselect={handleSelectQueryTab}
        onclose={handleCloseQueryTab}
        onnew={handleNewQueryTab}
      />

      <div class="editor-area" role="tabpanel" id="query-panel-{activeQueryTab?.id}">
        {#if activeConnectionTab.status === "error"}
          <div class="connection-error">
            <span class="error-msg">{activeConnectionTab.error ?? "Connection failed"}</span>
            <button
              class="retry-btn"
              onclick={async () => {
                const id = activeConnectionTab.connectionId;
                tabs.setConnectionStatus(id, "connecting");
                try {
                  await connections.connectRaw(id);
                  tabs.setConnectionStatus(id, "connected");
                } catch (e) {
                  tabs.setConnectionStatus(id, "error", String(e));
                }
              }}
            >Retry</button>
          </div>
        {:else if activeConnectionTab.status === "connecting"}
          <div class="connection-loading">
            <div class="spinner"></div>
            <span>Connecting…</span>
          </div>
        {:else if activeConnectionTab.status === "connected" && activeQueryTab}
          {#if activeQueryTab.tableBrowse}
            <!-- Table browse mode: EnhancedGrid with server-side sort/filter -->
            <div class="table-browse-header">
              <span class="table-browse-title">
                📋 {activeQueryTab.label}
              </span>
            </div>
            <EnhancedGrid
              browseState={activeQueryTab.tableBrowse}
              onStateChange={(patch) => {
                tabs.updateTableBrowseState(
                  activeConnectionTab.connectionId,
                  activeQueryTab.id,
                  patch
                );
              }}
            />
          {:else}
            <!-- SQL editor mode -->
            <EditorToolbar
              connectionTab={activeConnectionTab}
              queryTab={activeQueryTab}
            />
            <SqlEditor
              connectionId={activeConnectionTab.connectionId}
              queryTabId={activeQueryTab.id}
              sql={activeQueryTab.sql}
              dbType={connections.list.find((c) => c.id === activeConnectionTab.connectionId)?.db_type ?? "postgres"}
            />
            <ResultPane
              result={activeQueryTab.result}
              isExecuting={activeQueryTab.isExecuting}
              connectionId={activeConnectionTab.connectionId}
              queryTabId={activeQueryTab.id}
              dbType={connections.list.find((c) => c.id === activeConnectionTab.connectionId)?.db_type ?? "postgres"}
              onReExecute={() => {
                const dbT = connections.list.find((c) => c.id === activeConnectionTab.connectionId)?.db_type ?? "postgres";
                tabs.executeQuery(activeConnectionTab.connectionId, activeQueryTab.id, activeQueryTab.sql, dbT);
              }}
            />
          {/if}
        {:else}
          <div class="empty-state">
            <p>Select a connection to start querying</p>
          </div>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="welcome">
      <p>Open a connection from the sidebar to start</p>
    </div>
  {/if}
</div>

<style>
  .tabbed-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .editor-area {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .table-browse-header {
    display: flex;
    align-items: center;
    padding: 6px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
    flex-shrink: 0;
  }

  .table-browse-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text);
  }

  .welcome,
  .empty-state,
  .connection-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: 12px;
    color: var(--color-text-muted);
    font-size: 14px;
  }

  .connection-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: 12px;
  }

  .error-msg {
    font-size: 13px;
    color: var(--color-error);
    max-width: 400px;
    text-align: center;
  }

  .retry-btn {
    font-size: 12px;
    padding: 5px 14px;
    border: 1px solid var(--color-error);
    border-radius: 5px;
    background: transparent;
    color: var(--color-error);
    cursor: pointer;
  }

  .retry-btn:hover {
    background: var(--color-error);
    color: white;
  }

  .spinner {
    width: 22px;
    height: 22px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
