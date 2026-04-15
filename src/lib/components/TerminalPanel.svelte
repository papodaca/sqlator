<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { Channel } from "@tauri-apps/api/core";
  import { invoke } from "@tauri-apps/api/core";
  import { theme } from "$lib/stores/theme.svelte";
  import { terminalStore } from "$lib/stores/terminal.svelte";
  import "@xterm/xterm/css/xterm.css";

  let {
    connectionId,
    dbType,
  }: {
    connectionId: string;
    dbType: string;
  } = $props();

  // Panel height — resizable
  let panelHeight = $state(280);
  const MIN_HEIGHT = 120;
  const MAX_HEIGHT_RATIO = 0.6;

  let containerEl = $state<HTMLDivElement | undefined>();
  let termEl = $state<HTMLDivElement | undefined>();
  let terminal: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let terminalId: string | null = null;
  let errorMsg = $state<string | null>(null);
  let sessionEnded = $state(false);

  // ── xterm theme derived from app theme ───────────────────────────────────
  function xtermTheme(dark: boolean) {
    return dark
      ? {
          background: "#1e1e1e",
          foreground: "#d4d4d4",
          cursor: "#d4d4d4",
          selectionBackground: "#264f78",
          black: "#000000",
          red: "#cd3131",
          green: "#0dbc79",
          yellow: "#e5e510",
          blue: "#2472c8",
          magenta: "#bc3fbc",
          cyan: "#11a8cd",
          white: "#e5e5e5",
          brightBlack: "#666666",
          brightRed: "#f14c4c",
          brightGreen: "#23d18b",
          brightYellow: "#f5f543",
          brightBlue: "#3b8eea",
          brightMagenta: "#d670d6",
          brightCyan: "#29b8db",
          brightWhite: "#e5e5e5",
        }
      : {
          background: "#ffffff",
          foreground: "#333333",
          cursor: "#333333",
          selectionBackground: "#add6ff",
          black: "#000000",
          red: "#cd3131",
          green: "#00bc00",
          yellow: "#949800",
          blue: "#0451a5",
          magenta: "#bc05bc",
          cyan: "#0598bc",
          white: "#555555",
          brightBlack: "#666666",
          brightRed: "#cd3131",
          brightGreen: "#14ce14",
          brightYellow: "#b5ba00",
          brightBlue: "#0451a5",
          brightMagenta: "#bc05bc",
          brightCyan: "#0598bc",
          brightWhite: "#a5a5a5",
        };
  }

  async function spawnTerminal() {
    if (!termEl || !terminal) return;

    errorMsg = null;
    sessionEnded = false;

    const cols = terminal.cols || 80;
    const rows = terminal.rows || 24;

    const channel = new Channel<string>();
    channel.onmessage = (chunk: string) => {
      if (!terminal) return;
      // Null byte sentinel = session ended
      if (chunk === "\x00") {
        sessionEnded = true;
        terminal.writeln("\r\n\x1b[33mTerminal session ended. Press Enter to restart.\x1b[0m");
        return;
      }
      const bytes = atob(chunk);
      terminal.write(bytes);
    };

    try {
      terminalId = await invoke<string>("spawn_db_terminal", {
        connectionId,
        cols,
        rows,
        onData: channel,
      });
      terminalStore.setTerminalId(terminalId);
    } catch (e) {
      errorMsg = String(e);
    }
  }

  onMount(() => {
    if (!termEl) return;

    terminal = new Terminal({
      fontFamily: '"Cascadia Code", "Fira Code", "JetBrains Mono", "Menlo", monospace',
      fontSize: 13,
      theme: xtermTheme(theme.isDark),
      cursorBlink: true,
      allowTransparency: false,
    });

    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(termEl);
    fitAddon.fit();

    // Forward user input to PTY
    terminal.onData((data) => {
      if (!terminalId) return;
      // Restart on Enter if session ended
      if (sessionEnded && data === "\r") {
        spawnTerminal();
        return;
      }
      invoke("send_terminal_input", { terminalId, data }).catch(() => {});
    });

    // Resize PTY when container resizes
    resizeObserver = new ResizeObserver(() => {
      if (!fitAddon || !terminal || !terminalId) return;
      fitAddon.fit();
      invoke("resize_terminal", {
        terminalId,
        cols: terminal.cols,
        rows: terminal.rows,
      }).catch(() => {});
    });
    if (containerEl) resizeObserver.observe(containerEl);

    spawnTerminal();
  });

  // Sync xterm theme when app theme changes
  $effect(() => {
    if (terminal) {
      terminal.options.theme = xtermTheme(theme.isDark);
    }
  });

  onDestroy(() => {
    resizeObserver?.disconnect();
    if (terminalId) {
      invoke("close_terminal", { terminalId }).catch(() => {});
      terminalStore.setTerminalId(null);
    }
    terminal?.dispose();
  });

  // ── Drag-to-resize handle ─────────────────────────────────────────────────
  function handleDragStart(e: PointerEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startH = panelHeight;
    const maxH = window.innerHeight * MAX_HEIGHT_RATIO;

    function onMove(ev: PointerEvent) {
      const delta = startY - ev.clientY;
      panelHeight = Math.max(MIN_HEIGHT, Math.min(maxH, startH + delta));
    }
    function onUp() {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      // Fit after resize drag
      fitAddon?.fit();
      if (terminalId && terminal) {
        invoke("resize_terminal", {
          terminalId,
          cols: terminal.cols,
          rows: terminal.rows,
        }).catch(() => {});
      }
    }
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }
</script>

<div
  class="terminal-panel"
  style="height: {panelHeight}px"
  bind:this={containerEl}
>
  <!-- Drag handle -->
  <div
    class="drag-handle"
    role="separator"
    aria-orientation="horizontal"
    aria-label="Resize terminal panel"
    onpointerdown={handleDragStart}
  ></div>

  <!-- Header bar -->
  <div class="panel-header">
    <span class="panel-title">
      {dbType} CLI Terminal
    </span>
    <button
      class="close-btn"
      onclick={() => terminalStore.close()}
      aria-label="Close terminal"
      title="Close terminal (Ctrl+`)"
    >✕</button>
  </div>

  <!-- Terminal or error -->
  {#if errorMsg}
    <div class="terminal-error">
      <span class="error-icon">⚠</span>
      <span>{errorMsg}</span>
    </div>
  {:else}
    <div class="xterm-container" bind:this={termEl}></div>
  {/if}
</div>

<style>
  .terminal-panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg);
    overflow: hidden;
  }

  .drag-handle {
    height: 4px;
    cursor: ns-resize;
    background: transparent;
    flex-shrink: 0;
  }

  .drag-handle:hover {
    background: var(--color-accent);
    opacity: 0.5;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
    flex-shrink: 0;
    height: 28px;
  }

  .panel-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-muted);
    text-transform: lowercase;
    letter-spacing: 0.03em;
  }

  .close-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 14px;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--color-text);
  }

  .xterm-container {
    flex: 1;
    overflow: hidden;
    padding: 4px;
  }

  /* Let xterm take full height inside container */
  .xterm-container :global(.xterm) {
    height: 100%;
  }

  .xterm-container :global(.xterm-viewport) {
    overflow-y: scroll !important;
  }

  .terminal-error {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 16px;
    color: var(--color-error);
    font-size: 13px;
  }

  .error-icon {
    font-size: 16px;
    flex-shrink: 0;
  }
</style>
