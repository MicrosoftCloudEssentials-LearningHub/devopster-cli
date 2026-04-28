# DevOpster (CLI + GUI)

Costa Rica / USA

[![GitHub](https://img.shields.io/badge/--181717?logo=github&logoColor=ffffff)](https://github.com/)
[Cloud2BR](https://github.com/Cloud2BR)

Last updated: 2026-04-24

----------

> Cross-platform GitOps CLI built in Rust for managing organization repositories across GitHub, Azure DevOps, and GitLab.

## What It Is

> This project is a container-first CLI for repository governance and maintenance at scale. The goal is to give you a single tool that can blueprint repositories, audit standards, sync shared files, generate catalogs, and align metadata across multiple source-control platforms.

## Why This Repo Exists

> Managing an organization with many repositories usually leads to repeated manual work:

- keeping repo metadata aligned
- copying workflows and templates between repos
- auditing descriptions, topics, and licensing
- blueprinting new repositories from a standard template
- generating a reusable catalog of projects

`devopster` is intended to centralize that work behind one CLI and one config file.

## Current Foundation

> The repository now includes:

- a Rust CLI package with the `devopster` binary
- a command tree for setup, inventory, repo governance, catalog, topics, and stats
- a provider abstraction for GitHub, Azure DevOps, and GitLab
- YAML configuration loading through `devopster-config.yaml`
- a dev container and Docker-based workflow so the host machine does not need Rust installed
- a CI workflow that builds and tests through containers

## Container-First Workflow

> This project is designed to run inside a container. Host-level development dependencies are intentionally minimal: install Docker once on the host, then run everything else in containers.

## Release Automation

This repository now includes automated build artifacts on GitHub Actions:

- On every merge to `main`: builds downloadable CLI artifacts for Linux, Windows, and macOS.
- On version tags like `v1.0.0`: publishes those artifacts to a GitHub Release.

Current downloadable outputs:

- `devopster-linux-x86_64.tar.gz` (CLI binary + Linux desktop metadata + GUI launcher script)
- `devopster-windows-x86_64.zip` (CLI binary + GUI launcher cmd + `.ico` icon)
- `devopster-macos-x86_64.tar.gz` (CLI binary + `.icns` icon)
- `devopster-macos.dmg` (includes CLI files plus native `DevOpster GUI.app` bundle)

DevOpster now publishes one unified release (`v*` tags or manual dispatch with publish enabled) that includes both GUI installers and standalone CLI artifacts.
Download from GitHub Releases and choose either the desktop installer (`.dmg`, `-setup.exe`/`.msi`, `.deb`/`.AppImage`/`.rpm`) or the standalone CLI artifact for your platform.
Public macOS DMG releases require Apple Developer ID signing and notarization secrets in GitHub Actions; the release workflow now refuses to publish macOS installers without that configuration.

End-user launch modes now included in artifacts:

- CLI: run `devopster` directly
- GUI mode: run `devopster gui` or platform launcher wrapper from release assets

Branding asset for upcoming desktop packaging:

- Icon source (red lobster on blue background): `assets/devopster-icon.png`

## Desktop App (Tauri)

DevOpster ships a native desktop application built with [Tauri v2](https://tauri.app/).
It is a real multi-screen app — not a terminal wrapper — backed by the same
Rust CLI. Every screen calls a real `devopster` subcommand and renders the
response with proper UI.

Screens included:

- **Dashboard** — environment health (sidecar / config / OS+arch), quick links.
- **Diagnostics** — runs `devopster diagnostics` and shows pass/fail.
- **Inventory** — typed table of repositories (filter by name + provider) backed by `devopster inventory --json`.
- **Repo audit** — runs `devopster repo audit` (with optional `--report-only`) and renders results.
- **Stats / Catalog / Topics** — one-click runners for the matching subcommands.
- **Setup** — kicks off `devopster setup` with live streamed output.
- **Config** — full text editor for `devopster-config.yaml` with safe save (yaml/yml only).
- **Console** — live streaming runner for any allow-listed subcommand.

Source layout:

- `src-tauri/` — Tauri Rust crate (`devopster-desktop`) with allow-listed
  command bridge (`run_devopster`, `stream_devopster`, `run_devopster_json`,
  `env_info`, `read_config`, `write_config`).
- `frontend/` — static HTML/JS/CSS renderer (no bundler required).

Local development:

```bash
# one-time
cargo install tauri-cli --locked --version "^2"
cargo tauri icon assets/devopster-icon.png   # generates src-tauri/icons/

# build the CLI sidecar with the active host triple, then run the desktop app
TARGET=$(rustc -vV | sed -n 's/host: //p')
cargo build --release --target "$TARGET" --bin devopster
mkdir -p src-tauri/binaries
cp "target/$TARGET/release/devopster" "src-tauri/binaries/devopster-$TARGET"
cd src-tauri && cargo tauri dev
```

Native installers built in CI by the `Unified App Release (CLI + GUI)` workflow:

| OS                | Bundle                       |
| ----------------- | ---------------------------- |
| macOS Apple Silicon | `.dmg` / `.app.tar.gz`     |
| macOS Intel       | `.dmg` / `.app.tar.gz`       |
| Windows x64       | `.msi` (WiX) + NSIS `-setup.exe` |
| Linux x64         | `.deb` / `.AppImage`         |

Triggers: manual `workflow_dispatch` or pushing a `v*` tag.
Each release bundles both interfaces from the same version stream.

### Primary local workflow (recommended)

If you want local development with fully containerized tooling, use this one command from your host machine:

```bash
devopster dev-env
```

It opens the project container and runs:

- `cargo fetch && cargo install --path . --locked --force && cargo test`
- `devopster setup` (guided onboarding)

### What `devopster dev-env` does

| Tool | Where | How |
|---|---|---|
| Docker availability check | Host | Verifies Docker CLI + daemon |
| `gh`, `az`, `glab`, Rust, `devopster` | Inside the container | Dockerfile |

> `devopster dev-env` does not install host package managers or Docker for you; it validates Docker and then starts the containerized runtime.

> If you want zero local installs, run this repo in GitHub Codespaces (or another cloud dev container runtime) so Docker and tooling are provided remotely.

### Zero-local-install path (optional)

1. Open the repository in GitHub.
2. Click **Code** > **Codespaces** > **Create codespace on main**.
3. Wait for the dev container to finish bootstrapping (post-create runs Cargo bootstrap commands).
4. Start onboarding:

```bash
devopster setup
```

This path needs only a browser and GitHub access.

### Recommended: local editor + fully containerized tooling

Use this path if you want to code locally (not in the browser) while still keeping all dev tools inside the container.

1. Install Docker Desktop once.
2. Install VS Code once.
3. Install the Dev Containers extension in VS Code.
4. Clone this repository locally.
5. Open the folder in VS Code.
6. Run Reopen in Container.
7. Wait for post-create to finish (it runs Cargo bootstrap automatically).
8. Run the first-use onboarding command:

```bash
devopster setup
```

Result: code stays local in VS Code, while Rust, provider CLIs, build, and tests all run inside the dev container.

### Application modes

- CLI mode: run direct commands such as `devopster dev verify` and `devopster dev-env`.
- GUI mode (interactive launcher): run `devopster gui` or just `devopster` in an interactive terminal.

### VS Code Dev Container

1. Clone the repository.
2. Open it in VS Code.
3. Reopen the folder in the Dev Container.
4. The post-create step runs Cargo bootstrap and prepares the full toolchain in-container.

### Local Commands

```bash
# Primary local onboarding path:
devopster dev-env

# Or open a container shell only:
./scripts/setup.sh

# Inside the container:
devopster
```

### In-container bootstrap command

```bash
cargo fetch
cargo install --path . --locked --force
cargo test
```

This command is safe to re-run and performs:

- dependency fetch
- local install of the `devopster` binary
- full test run

> To reopen the container shell after exiting, run `devopster dev-env --no-onboarding` or re-run `./scripts/setup.sh`.

## Commands Overview

> Run `devopster` with no subcommand to open the interactive launcher. For local containerized onboarding, use `devopster dev-env`.

### Interactive launcher

```bash
devopster
```

The launcher lets you choose actions with the keyboard for:

- local developer environment bootstrap
- developer verification (build/test/lint)
- repository inventory view
- setup/init/login flows
- repo actions
- catalog generation
- topic alignment
- stats

### Primary direct commands

These are the direct commands most people need:

| Command | Options / Variants | Purpose |
|---|---|---|
| `devopster gui` | N/A | - Open the interactive launcher explicitly |
| `devopster diagnostics` | N/A | - Validate local Docker/runtime readiness<br/>- Check provider CLI tooling availability |
| `devopster config template` | - `--output <path>`<br/>- `--stdout` | - Generate a fresh configuration template from built-in defaults<br/>- Print template to terminal |
| `devopster inventory` | - `--json` | - Show repository inventory summary for the configured scope<br/>- Emit full inventory as JSON |
| `devopster dev` | - `bootstrap`<br/>- `build`<br/>- `test`<br/>- `lint`<br/>- `verify`<br/>- `--no-build`<br/>- `--image <tag>` | - Run developer automation tasks in the dev container from Rust code<br/>- Skip image rebuild when already available<br/>- Use a custom image tag |
| `devopster dev-env` | - `--no-build`<br/>- `--no-onboarding`<br/>- `--image <tag>` | - Start local containerized developer environment from the Rust app<br/>- Reuse an existing image without rebuilding<br/>- Run bootstrap only and skip guided onboarding<br/>- Use a custom container image tag |
| `devopster setup` | - `--login-all`<br/>- `--no-login`<br/>- `--output <path>` | - Run one-command onboarding (guided login + guided config)<br/>- Sign in to all providers first, then continue config setup<br/>- Skip provider sign-in and only run config setup<br/>- Write config to a custom path |
| `devopster init` | - `--no-login` | - Create `devopster-config.yaml` and sign in to a provider<br/>- Create `devopster-config.yaml` only, skip the sign-in prompt |
| `devopster login` | - `<github\|azure-devops\|gitlab>`<br/>- `all`<br/>- `status`<br/>- `logout <provider>` | - Sign in to that provider via browser (uses `gh`, `az`, or `glab` CLI)<br/>- Sign in to all three providers sequentially<br/>- Show authentication status for all providers<br/>- Remove stored credentials for a provider |

### Advanced direct commands

These commands still work directly when you want explicit CLI usage or scripting:

| Command | Options / Variants | Purpose |
|---|---|---|
| `devopster repo list` | - `--topic <topic>` | - List all repositories in the configured organization<br/>- Filter repositories by topic |
| `devopster repo audit` | N/A | - Audit repos for missing description, topics, license, and default branch |
| `devopster repo fix` | N/A | - Prompt for missing description, topics, and license in scoped repos that actually need fixes |
| `devopster repo blueprint` | - `--name <name> --template <template>` | - Create a new repository from a template defined in config |
| `devopster repo sync` | - `--from-blueprint` | - Push files from `.github/` to all repositories<br/>- Compare required workflow files from the blueprint repo, prompt to add any missing ones, and prompt to add the org README header and badge markers |
| `devopster catalog generate` | N/A | - Export a JSON catalog of all repositories |
| `devopster topics align` | N/A | - Add missing template topics to every matching repository |
| `devopster stats` | - `--scope-missing` | - Print org summary: config, coverage (description/topics/license/branch), compliance, and top topics<br/>- Update scoped repos to the non-compliant list |

### Quick start inside the container

```bash
# 1. build the image and install devopster
devopster dev-env --no-onboarding

# 2. open the launcher
devopster

# 2b. or run one-command onboarding directly
devopster setup

# 3. or run any command directly
devopster repo list
devopster repo audit
devopster repo blueprint --name sample-repo --template azure-overview
devopster catalog generate
devopster topics align
devopster stats
```

> To skip the sign-in prompt: `devopster init --no-login`

## Configuration

| File | Purpose |
|---|---|
| `devopster-config.yaml` | Your active config: generated by `devopster init` or by `devopster config template --output`. Git-ignored. |

- Generate a config file from built-in defaults:

  ```bash
  devopster config template --output devopster-config.yaml
  ```

- Then set the provider and token environment variables you want to use.

> [!TIP]
> Configure a blueprint source repo for `devopster repo sync --from-blueprint`:

```yaml
blueprint:
  repo: your-org/org-repo-template
	branch: main
	paths:
		- .github
```

<!-- START BADGE -->
<div align="center">
  <img src="https://img.shields.io/badge/Total%20views-278-limegreen" alt="Total views">
  <p>Refresh Date: 2026-04-28</p>
</div>
<!-- END BADGE -->