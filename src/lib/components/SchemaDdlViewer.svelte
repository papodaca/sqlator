<script lang="ts">
  import { onMount } from "svelte";
  import { EditorView } from "@codemirror/view";
  import { EditorState } from "@codemirror/state";
  import { sql } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { api } from "$lib/api";
  import type { SchemaDdlState } from "$lib/types";

  let {
    ddlState,
    onStateChange,
  }: {
    ddlState: SchemaDdlState;
    onStateChange: (patch: Partial<SchemaDdlState>) => void;
  } = $props();

  let editorContainer: HTMLElement | undefined = $state();
  let view: EditorView | undefined;
  let copyFeedback = $state(false);

  async function fetchDdl() {
    onStateChange({ isLoading: true, error: null });
    try {
      const ddl = await api.invoke<string>("get_ddl", {
        connectionId: ddlState.connectionId,
        tableName: ddlState.tableName,
        schema: ddlState.schema,
      });
      onStateChange({ ddl, isLoading: false });
    } catch (e) {
      onStateChange({ error: String(e), isLoading: false });
    }
  }

  function renderEditor(ddl: string) {
    if (!editorContainer) return;
    if (view) view.destroy();

    view = new EditorView({
      state: EditorState.create({
        doc: ddl,
        extensions: [
          sql(),
          oneDark,
          EditorView.editable.of(false),
          EditorView.lineWrapping,
        ],
      }),
      parent: editorContainer,
    });
  }

  $effect(() => {
    if (ddlState.ddl) {
      renderEditor(ddlState.ddl);
    }
    return () => { view?.destroy(); };
  });

  async function handleCopy() {
    if (!ddlState.ddl) return;
    await navigator.clipboard.writeText(ddlState.ddl);
    copyFeedback = true;
    setTimeout(() => copyFeedback = false, 2000);
  }

  onMount(() => {
    if (ddlState.isLoading && !ddlState.ddl) fetchDdl();
  });
</script>

<div class="ddl-viewer">
  <div class="ddl-toolbar">
    <span class="ddl-title">{ddlState.schema ? `${ddlState.schema}.${ddlState.tableName}` : ddlState.tableName}</span>
    <div class="ddl-actions">
      <button class="ddl-btn" onclick={fetchDdl} disabled={ddlState.isLoading}>Refresh</button>
      <button class="ddl-btn" onclick={handleCopy} disabled={!ddlState.ddl}>
        {copyFeedback ? "Copied!" : "Copy"}
      </button>
    </div>
  </div>

  {#if ddlState.isLoading && !ddlState.ddl}
    <div class="ddl-loading"><div class="spinner"></div> Loading DDL...</div>
  {:else if ddlState.error}
    <div class="ddl-error">
      <p>{ddlState.error}</p>
      <button class="ddl-btn" onclick={fetchDdl}>Retry</button>
    </div>
  {:else if ddlState.ddl}
    <div class="ddl-editor" bind:this={editorContainer}></div>
  {/if}
</div>

<style>
  .ddl-viewer {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .ddl-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
    flex-shrink: 0;
  }

  .ddl-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text);
    font-family: monospace;
  }

  .ddl-actions {
    display: flex;
    gap: 6px;
  }

  .ddl-btn {
    font-size: 12px;
    padding: 4px 12px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: var(--color-surface-2);
    color: var(--color-text);
    cursor: pointer;
    transition: background-color 0.1s;
  }

  .ddl-btn:hover:not(:disabled) {
    background: var(--color-surface);
  }

  .ddl-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .ddl-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    flex: 1;
    color: var(--color-text-muted);
    font-size: 13px;
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

  .ddl-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    flex: 1;
    padding: 24px;
  }

  .ddl-error p {
    font-size: 13px;
    color: var(--color-error);
    max-width: 400px;
    text-align: center;
    margin: 0;
  }

  .ddl-editor {
    flex: 1;
    overflow: auto;
  }

  .ddl-editor :global(.cm-editor) {
    height: 100%;
  }

  .ddl-editor :global(.cm-scroller) {
    font-family: monospace;
  }
</style>
