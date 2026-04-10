#!/usr/bin/env bash
set -euo pipefail

if [[ -t 1 ]]; then
    RESET=$'\033[0m'
    CYAN=$'\033[36m'
    GREEN=$'\033[32m'
    YELLOW=$'\033[33m'
    DIM=$'\033[2m'
else
    RESET=''
    CYAN=''
    GREEN=''
    YELLOW=''
    DIM=''
fi

info() {
    printf "%s%s%s %s\n" "$CYAN" "info" "$RESET" "$1"
}

success() {
    printf "%s%s%s %s\n" "$GREEN" "done" "$RESET" "$1"
}

warn() {
    printf "%s%s%s %s\n" "$YELLOW" "warn" "$RESET" "$1"
}

show_menu() {
    local prompt="$1"
    shift
    local options=("$@")
    local selected=0
    local rendered=0

    [[ ${#options[@]} -gt 0 ]] || return 1

    while true; do
        if (( rendered > 0 )); then
            printf '\033[%dA' "$rendered"
        fi
        printf '\033[J'
        printf "%s>%s %s\n" "$CYAN" "$RESET" "$prompt"
        for index in "${!options[@]}"; do
            if [[ $index -eq $selected ]]; then
                printf "  %s> %s%s\n" "$YELLOW" "${options[$index]}" "$RESET"
            else
                printf "  %s%s%s\n" "$DIM" "${options[$index]}" "$RESET"
            fi
        done
        rendered=$(( ${#options[@]} + 1 ))

        IFS= read -rsn1 key || true
        if [[ $key == $'\x1b' ]]; then
            IFS= read -rsn2 rest || true
            key+="$rest"
        fi

        case "$key" in
            $'\x1b[A') selected=$(( (selected - 1 + ${#options[@]}) % ${#options[@]} )) ;;
            $'\x1b[B') selected=$(( (selected + 1) % ${#options[@]} )) ;;
            ''|$'\n'|$'\r') break ;;
        esac
    done

    printf '\033[%dA\033[J' "$rendered"
    printf "%s>%s %s %s%s%s\n" "$CYAN" "$RESET" "$prompt" "$YELLOW" "${options[$selected]}" "$RESET"
    REPLY=$selected
}

is_interactive() {
    [[ -t 0 && -t 1 ]]
}

info "devopster setup"

# ── Install Docker if missing ─────────────────────────────────────────────────
if command -v docker > /dev/null 2>&1; then
    success "Docker already installed: $(docker --version)"
else
    if [[ "$(uname -s)" == "Darwin" ]]; then
        info "Installing Docker Desktop via Homebrew..."
        if ! command -v brew > /dev/null 2>&1; then
            info "Installing Homebrew first..."
            /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        fi
        brew install --cask docker
        success "Docker Desktop installed."
        warn "Please start Docker Desktop from Applications, then re-run: make setup"
        exit 0
    elif [[ -f /etc/debian_version ]]; then
        info "Installing Docker Engine via apt..."
        sudo apt-get update -qq
        sudo apt-get install -y ca-certificates curl gnupg lsb-release
        sudo install -m 0755 -d /etc/apt/keyrings
        curl -fsSL https://download.docker.com/linux/debian/gpg \
            | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
        echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] \
https://download.docker.com/linux/debian $(lsb_release -cs) stable" \
            | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
        sudo apt-get update -qq
        sudo apt-get install -y docker-ce docker-ce-cli containerd.io \
            docker-buildx-plugin docker-compose-plugin
        sudo usermod -aG docker "$USER"
        success "Docker installed."
        warn "You may need to log out and back in for group changes."
    else
        warn "Unsupported OS. Install Docker manually: https://docs.docker.com/get-docker/"
        exit 1
    fi
fi

# ── Build image ──────────────────────────────────────────────────────────────
info "Building devopster container image..."
make container-build

# ── Host browser proxy ───────────────────────────────────────────────────────
# devopster writes a URL to .devopster_open_url (inside the mounted /app dir)
# when it needs to open the browser from inside the container.  We watch that
# file here on the host and call open/xdg-open so the browser pops up on the
# host machine automatically — no manual URL copying needed.
OPEN_FILE="$(pwd)/.devopster_open_url"
: > "$OPEN_FILE"
_last_url=""
(
    while true; do
        _url=$(cat "$OPEN_FILE" 2>/dev/null | tr -d '[:space:]')
        if [[ -n "$_url" && "$_url" != "$_last_url" ]]; then
            _last_url="$_url"
            if [[ "$(uname -s)" == "Darwin" ]]; then
                open "$_url"
            else
                xdg-open "$_url" 2>/dev/null || true
            fi
        fi
        sleep 0.3
    done
) &
_BROWSER_PID=$!

# Clean up the watcher and temp file when the container session ends.
trap 'kill "$_BROWSER_PID" 2>/dev/null || true; rm -f "$OPEN_FILE"' EXIT INT TERM

docker_command=(bash)
docker_args=(run --rm)
if is_interactive; then
    docker_args+=( -it )
    show_menu "Choose how to start inside Docker" \
        "Open an interactive shell" \
        "Run devopster launcher" \
        "Run devopster init" \
        "Run devopster login status" \
        "Show devopster help"

    case "$REPLY" in
        1) docker_command=(devopster) ;;
        2) docker_command=(devopster init) ;;
        3) docker_command=(devopster login status) ;;
        4) docker_command=(devopster --help) ;;
    esac
else
    info "Non-interactive shell detected. Skipping setup menu and opening bash."
fi

success "Starting Docker runtime — browser links will still open on the host when needed."
docker "${docker_args[@]}" \
    -v "$HOME/.config/devopster:/root/.config/devopster" \
    -v "$(pwd):/app" \
    -w /app \
    devopster-cli-dev \
    "${docker_command[@]}"
