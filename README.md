# devopster-cli

Atlanta, USA

[![GitHub](https://img.shields.io/badge/--181717?logo=github&logoColor=ffffff)](https://github.com/)
[brown9804](https://github.com/brown9804)

Last updated: 2026-03-25

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
- a command tree for `init`, `repo`, `catalog`, `topics`, and `stats`
- a provider abstraction for GitHub, Azure DevOps, and GitLab
- YAML configuration loading through `devopster-config.yaml`
- a dev container and Docker-based workflow so the host machine does not need Rust installed
- a CI workflow that builds and tests through containers

## Container-First Workflow

> This project is designed to run inside a container. Host-level development dependencies are intentionally minimal: install Docker once on the host, then run everything else in containers.

### What `make setup` does

| Tool | Where | How |
|---|---|---|
| Docker availability check | Host | Verifies Docker CLI + daemon |
| `gh`, `az`, `glab`, Rust, `devopster` | Inside the container | Dockerfile |

> `make setup` does not install host package managers or Docker for you; it validates Docker and then starts the containerized runtime.

> If you want zero local installs, run this repo in GitHub Codespaces (or another cloud dev container runtime) so Docker and tooling are provided remotely.

### Zero-local-install path (recommended for new developers)

1. Open the repository in GitHub.
2. Click **Code** > **Codespaces** > **Create codespace on main**.
3. Wait for the dev container to finish bootstrapping (post-create runs `make bootstrap`).
4. Start onboarding:

```bash
devopster setup
```

This path needs only a browser and GitHub access.

### VS Code Dev Container

1. Clone the repository.
2. Open it in VS Code.
3. Reopen the folder in the Dev Container.
4. The post-create step installs `devopster` inside the dev container.

### Local Commands

```bash
# First time on a new machine -- with Docker already installed/running,
# this builds the image and drops you into the container shell:
make setup

# Inside the container:
devopster
```

### In-container bootstrap command

```bash
make bootstrap
```

This command is safe to re-run and performs:

- dependency fetch
- local install of the `devopster` binary
- full test run

> To reopen the container shell after exiting, run `make setup` again or `make container-run`.

## Commands Overview

> After `make setup`, run `devopster` with no subcommand to open the interactive launcher. For most users, the only top-level command you need to remember is `setup`.

### Interactive launcher

```bash
devopster
```

The launcher lets you choose actions with the keyboard for:

- init
- login
- repo actions
- catalog generation
- topic alignment
- stats

### Primary direct commands

These are the direct commands most people need:

| Command | Options / Variants | Purpose |
|---|---|---|
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
make setup

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
| `devopster-config.template.yaml` | Committed template: all supported keys with comments. Copy this to get started. |
| `devopster-config.yaml` | Your active config: generated by `devopster init` or copied from the template. Git-ignored. |

- Start from the template file:

    ```bash
    cp devopster-config.template.yaml devopster-config.yaml
    ```

- Then set the provider and token environment variables you want to use.

> [!TIP]
> Configure a blueprint source repo for `devopster repo sync --from-blueprint`:

```yaml
blueprint:
	repo: MicrosoftCloudEssentials-LearningHub/org-repo-template
	branch: main
	paths:
		- .github
```

<!-- START BADGE -->
<div align="center">
  <img src="https://img.shields.io/badge/Total%20views-1580-limegreen" alt="Total views">
  <p>Refresh Date: 2026-02-25</p>
</div>
<!-- END BADGE -->