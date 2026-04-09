import { api } from "$lib/api";

type ThemeMode = "light" | "dark" | "system";

let mode = $state<ThemeMode>("system");
let resolved = $state<"light" | "dark">("light");

function applyTheme() {
  if (mode === "system") {
    resolved = window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  } else {
    resolved = mode;
  }
  document.documentElement.classList.toggle("dark", resolved === "dark");
}

export const theme = {
  get mode() {
    return mode;
  },
  get resolved() {
    return resolved;
  },
  get isDark() {
    return resolved === "dark";
  },

  async init() {
    // Load persisted preference
    try {
      const saved = await api.invoke<string>("get_theme");
      if (saved === "light" || saved === "dark" || saved === "system") {
        mode = saved;
      }
    } catch {
      // Default to system
    }

    applyTheme();

    // Listen for OS theme changes
    window
      .matchMedia("(prefers-color-scheme: dark)")
      .addEventListener("change", () => {
        if (mode === "system") applyTheme();
      });
  },

  async toggle() {
    if (mode === "system") {
      mode = resolved === "dark" ? "light" : "dark";
    } else if (mode === "dark") {
      mode = "light";
    } else {
      mode = "dark";
    }
    applyTheme();
    try {
      await api.invoke("save_theme", { theme: mode });
    } catch {
      // Non-critical
    }
  },

  async setMode(newMode: ThemeMode) {
    mode = newMode;
    applyTheme();
    try {
      await api.invoke("save_theme", { theme: mode });
    } catch {
      // Non-critical
    }
  },
};
