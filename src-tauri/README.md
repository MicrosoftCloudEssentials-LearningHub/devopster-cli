# DevOpster Desktop (Tauri)

This is the desktop shell for DevOpster. It is a thin Tauri v2 application that
wraps the existing `devopster` CLI as a sidecar binary and exposes a small
allow-listed set of commands to the renderer.

## Layout

- `Cargo.toml` — Tauri Rust crate (`devopster-desktop`).
- `tauri.conf.json` — Tauri configuration (window, bundle, sidecar).
- `src/lib.rs` — Tauri setup + `run_devopster` / `stream_devopster` commands.
- `src/main.rs` — entry point.
- `capabilities/default.json` — capability set for the main window.
- `icons/` — generated platform icons (gitignored; regenerate via Tauri CLI).
- `binaries/` — sidecar binary copies (gitignored; produced by release pipeline).

## One-time setup

```bash
cargo install tauri-cli --locked --version "^2"
```

## Generate icons (one-time, or whenever the brand asset changes)

```bash
cargo tauri icon ../assets/devopster-icon.png
```

This populates `src-tauri/icons/` with the PNG / ICO / ICNS variants referenced
by `tauri.conf.json`.

## Build the sidecar

The desktop app expects a `devopster` binary alongside it. For local dev, build
the CLI from the workspace root:

```bash
cargo build -p devopster-cli --release
```

Then copy it where Tauri expects the sidecar (per target triple):

```bash
mkdir -p src-tauri/binaries
# macOS arm64 example
cp target/release/devopster src-tauri/binaries/devopster
```

In production this is automated by the `desktop-app.yml` workflow (Unified App Release).

## Run

```bash
cargo tauri dev      # development window
cargo tauri build    # native installer (.dmg / .msi / .deb / .AppImage)
```
