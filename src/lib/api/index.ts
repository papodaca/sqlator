import type { ApiAdapter } from "./adapter";
import { tauriAdapter } from "./tauri-adapter";
import { webAdapter } from "./web-adapter";

// VITE_TARGET=web → webAdapter; anything else (including undefined) → tauriAdapter.
// Vite/Rollup replaces import.meta.env at build time, enabling tree-shaking of
// the unused adapter from the final bundle.
export const api: ApiAdapter =
  (import.meta.env.VITE_TARGET as string) === "web" ? webAdapter : tauriAdapter;

export type { ApiAdapter, StreamEvent } from "./adapter";
