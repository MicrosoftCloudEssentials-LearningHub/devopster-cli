FROM rust:1.85-bookworm AS base

RUN rustup component add clippy rustfmt

# ── System packages ──────────────────────────────────────────────────────────
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        gnupg \
        lsb-release \
        make \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# ── GitHub CLI (gh) ──────────────────────────────────────────────────────────
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# ── Azure CLI (az) ────────────────────────────────────────────────────────────
RUN curl -sL https://aka.ms/InstallAzureCLIDeb | bash \
    && rm -rf /var/lib/apt/lists/*

# ── GitLab CLI (glab) ─────────────────────────────────────────────────────────
RUN ARCH=$(dpkg --print-architecture) \
    && GLAB_VERSION=$(curl -fsSL https://gitlab.com/api/v4/projects/34675721/releases \
        | grep -o '"tag_name":"[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/^v//') \
    && curl -fsSL "https://gitlab.com/gitlab-org/cli/-/releases/v${GLAB_VERSION}/downloads/glab_${GLAB_VERSION}_linux_${ARCH}.deb" \
        -o /tmp/glab.deb \
    && dpkg -i /tmp/glab.deb \
    && rm /tmp/glab.deb

WORKDIR /app
COPY . .
RUN cargo fetch

# ── Build & install the binary ────────────────────────────────────────────────
# `cargo install` places `devopster` in ~/.cargo/bin which is on PATH in the
# official Rust image, so `devopster <cmd>` works directly after `make setup`.
RUN cargo install --path . --locked

FROM base AS test
RUN cargo test

FROM base AS dev
# Binary is already installed from base — start a shell so the user can run
# `devopster login github` and any other command right away.
CMD ["bash"]
