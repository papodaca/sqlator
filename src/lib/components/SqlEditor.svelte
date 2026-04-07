<script lang="ts">
  import { onMount } from "svelte";
  import { EditorView, basicSetup } from "codemirror";
  import { sql, PostgreSQL, MySQL, SQLite, type SQLDialect } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { keymap } from "@codemirror/view";
  import { Prec } from "@codemirror/state";
  import { tabs } from "$lib/stores/tabs.svelte";
  import { theme } from "$lib/stores/theme.svelte";

  let {
    connectionId,
    queryTabId,
    sql: initialSql = "",
    dbType = "postgres",
  }: {
    connectionId: string;
    queryTabId: string;
    sql?: string;
    dbType: string;
  } = $props();

  const dialectMap: Record<string, SQLDialect> = {
    postgres: PostgreSQL,
    mysql: MySQL,
    mariadb: MySQL,
    sqlite: SQLite,
  };

  let editorEl = $state<HTMLDivElement | null>(null);
  let view: EditorView | null = null;
  // Track which tab+connection the editor is currently showing
  let currentKey = "";

  function getEditorSql(): string {
    return view?.state.doc.toString() ?? "";
  }

  async function executeCurrentQuery() {
    const sqlText = getEditorSql();
    if (!sqlText.trim()) return;
    await tabs.executeQuery(connectionId, queryTabId, sqlText);
  }

  function createEditor(
    parent: HTMLElement,
    dialect: SQLDialect,
    isDark: boolean,
    doc: string,
  ): EditorView {
    const extensions = [
      basicSetup,
      sql({ dialect }),
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
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          const newSql = update.state.doc.toString();
          tabs.updateSql(connectionId, queryTabId, newSql);
        }
      }),
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

    return new EditorView({ doc, extensions, parent });
  }

  $effect(() => {
    // Key: rebuild editor when tab, connection, or theme changes
    const key = `${connectionId}:${queryTabId}:${theme.isDark}`;
    const isDark = theme.isDark;
    const dialect = dialectMap[dbType] ?? PostgreSQL;

    if (!editorEl) return;

    if (key === currentKey && view) {
      // Only theme changed — handled by key change which rebuilds
      return;
    }

    // Destroy old editor
    if (view) {
      view.destroy();
      view = null;
    }

    // Create new editor with the current tab's SQL
    view = createEditor(editorEl, dialect, isDark, initialSql);
    view.focus();
    currentKey = key;

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
