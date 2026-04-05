<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { EditorView, basicSetup } from "codemirror";
  import { sql, PostgreSQL, MySQL, SQLite } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { keymap } from "@codemirror/view";
  import { Prec } from "@codemirror/state";
  import { connections } from "$lib/stores/connections.svelte";
  import { query } from "$lib/stores/query.svelte";
  import { theme } from "$lib/stores/theme.svelte";

  const dialectMap: Record<string, ReturnType<typeof PostgreSQL>> = {
    postgres: PostgreSQL,
    mysql: MySQL,
    sqlite: SQLite,
  };

  let editorEl = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;

  function getEditorSql(): string {
    return view?.state.doc.toString() ?? "";
  }

  async function executeCurrentQuery() {
    const sqlText = getEditorSql();
    if (!sqlText.trim() || !connections.activeId) return;

    // Persist query
    try {
      await invoke("save_query", {
        connectionId: connections.activeId,
        query: sqlText,
      });
    } catch {
      // Non-critical
    }

    await query.execute(connections.activeId, sqlText);
    return true;
  }

  function createEditor(
    parent: HTMLElement,
    dbType: string,
    isDark: boolean,
    initialDoc: string,
  ): EditorView {
    const extensions = [
      basicSetup,
      sql({ dialect: dialectMap[dbType] ?? PostgreSQL }),
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: () => {
              executeCurrentQuery();
              return true;
            },
          },
        ]),
      ),
      EditorView.lineWrapping,
      EditorView.theme({
        "&": {
          fontSize: "14px",
          fontFamily: "var(--font-mono)",
        },
        ".cm-content": {
          caretColor: "var(--color-text)",
        },
        ".cm-gutters": {
          backgroundColor: "var(--color-surface)",
          borderRight: "1px solid var(--color-border)",
          color: "var(--color-text-muted)",
        },
        "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
          backgroundColor: "var(--color-surface-2) !important",
        },
      }),
    ];

    if (isDark) {
      extensions.push(oneDark);
    }

    return new EditorView({
      doc: initialDoc,
      extensions,
      parent,
    });
  }

  // Rebuild editor when connection or theme changes
  let lastConnectionId: string | null = null;
  let lastIsDark: boolean | null = null;

  $effect(() => {
    const active = connections.active;
    const isDark = theme.isDark;

    if (!editorEl || !active) return;

    const connectionChanged = active.id !== lastConnectionId;
    const themeChanged = isDark !== lastIsDark;

    if (!connectionChanged && !themeChanged && view) return;

    // Save current query before switching
    if (view && lastConnectionId && connectionChanged) {
      const currentSql = getEditorSql();
      if (currentSql.trim()) {
        invoke("save_query", {
          connectionId: lastConnectionId,
          query: currentSql,
        }).catch(() => {});
      }
    }

    // Destroy old editor
    if (view) {
      view.destroy();
      view = null;
    }

    // Load saved query for this connection
    (async () => {
      let initialDoc = "";
      try {
        const saved = await invoke<string | null>("get_query", {
          connectionId: active.id,
        });
        if (saved) initialDoc = saved;
      } catch {
        // No saved query
      }

      if (editorEl) {
        view = createEditor(editorEl, active.db_type, isDark, initialDoc);
        view.focus();
      }
    })();

    lastConnectionId = active.id;
    lastIsDark = isDark;

    return () => {
      if (view) {
        view.destroy();
        view = null;
      }
    };
  });
</script>

<div class="editor-container" bind:this={editorEl}></div>

<style>
  .editor-container {
    flex: 1;
    min-height: 120px;
    overflow: auto;
    border-bottom: 1px solid var(--color-border);
  }

  .editor-container :global(.cm-editor) {
    height: 100%;
  }

  .editor-container :global(.cm-scroller) {
    overflow: auto;
  }
</style>
