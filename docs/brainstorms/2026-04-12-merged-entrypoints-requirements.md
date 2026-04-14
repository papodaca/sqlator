---
date: 2026-04-12
topic: merged-entrypoints
---

# Merged Entrypoints

## Problem Frame

SQLator ships as three separate binaries (`sqlator-app`, `sqlator-tui`, `sqlator-web`) that share a core library. Distributing three binaries complicates installation and adds cognitive overhead for users. A single `sqlator` binary with subcommands would simplify distribution and make the tool feel like one product.

## Requirements

- R1. A single `sqlator` binary that supports subcommands: `tui` and `web`, with no subcommand launching the Tauri GUI
- R2. `sqlator` (no arguments) launches the Tauri desktop GUI
- R3. `sqlator tui` launches the terminal UI with the same behavior as the current `sqlator-tui` binary
- R4. `sqlator web` launches the web server with the same flags as the current `sqlator-web` binary (`--port`, `--host`, `--config`, `--static-dir`)
- R5. The Tauri packaging pipeline (`.app` bundle on macOS, AppImage on Linux) continues to work for GUI distribution
- R6. A CLI wrapper or symlink is provided so `sqlator`, `sqlator tui`, and `sqlator web` work from a terminal when installed alongside the Tauri package
- R7. Running `sqlator --help` displays usage information listing all available subcommands
- R8. The standalone `sqlator-tui` and `sqlator-web` binaries are removed; the merged binary replaces them

## Success Criteria

- Users install one binary and access all three modes via subcommands
- Tauri packaging produces a working GUI app
- `sqlator tui` and `sqlator web` behave identically to the current standalone binaries
- No functionality is lost in the merge

## Scope Boundaries

- Windows support is out of scope for this iteration (the `windows_subsystem` attribute is a compile-time-only setting that conflicts with mixed GUI/console usage)
- No changes to the SvelteKit frontend or Tauri command surface
- No shared runtime state between modes (each mode runs independently, same as today)

## Key Decisions

- **Always GUI by default**: `sqlator` with no args launches Tauri, even from a terminal. No auto-detection of display availability.
- **Binary size is acceptable**: All three modes in one binary; no feature-flag-based trimming.
- **Tauri package + CLI wrapper**: The binary lives inside the Tauri package for GUI distribution, with a CLI wrapper/symlink providing terminal access for subcommands.
- **Primary platforms: macOS + Linux**: Windows is deferred due to the console-subsystem constraint.

## Dependencies / Assumptions

- Tauri's build pipeline can compile a binary that does CLI argument parsing before calling `tauri::Builder`
- The `tui-app` and `web-server` crates can expose library entrypoints (`run()` functions) that the unified binary calls into

## Outstanding Questions

### Deferred to Planning

- [Affects R5][Technical] How should the `sqlator web --static-dir` default path work when the binary is inside a Tauri package bundle? The current default `./build` won't resolve correctly from within a `.app` or AppImage.
- [Affects R6][Technical] What form does the CLI wrapper take — a symlink, a small shell script, or a post-install step? This depends on the packaging format per platform.
- [Affects R1][Technical] Should the unified entrypoint crate be a new workspace member, or should `src-tauri` absorb the dispatch logic? Affects how Tauri's build pipeline is configured.

## Next Steps

→ `/ce:plan` for structured implementation planning
