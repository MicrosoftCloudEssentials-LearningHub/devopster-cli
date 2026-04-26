#!/usr/bin/env bash
set -euo pipefail

APP_NAME="DevOpster.app"
MOUNT_DEVICE=""

print_usage() {
  cat <<'EOF'
Usage:
  ./install-macos-app.sh [SOURCE_APP_PATH] [INSTALL_DIR]

Examples:
  ./install-macos-app.sh
  ./install-macos-app.sh "$HOME/Downloads/DevOpster_0.1.0_aarch64.dmg"
  ./install-macos-app.sh "/Volumes/devopster-cli/DevOpster.app"
  ./install-macos-app.sh "/Volumes/devopster-cli/DevOpster.app" "$HOME/Applications"

Behavior:
  - SOURCE_APP_PATH can be a mounted DevOpster.app path or a .dmg file.
  - If SOURCE_APP_PATH is omitted, the script tries to auto-detect DevOpster.app
    first, then a DevOpster*.dmg in the current directory or Downloads.
  - If INSTALL_DIR is omitted, it uses /Applications when writable, otherwise
    $HOME/Applications.
EOF
}

detect_source_app() {
  local candidate

  if [[ -d "./${APP_NAME}" ]]; then
    printf '%s\n' "./${APP_NAME}"
    return 0
  fi

  for candidate in /Volumes/*/"${APP_NAME}"; do
    if [[ -d "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  for candidate in ./DevOpster*.dmg "$HOME"/Downloads/DevOpster*.dmg; do
    if [[ -f "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  return 1
}

cleanup_mount() {
  if [[ -n "${MOUNT_DEVICE}" ]]; then
    hdiutil detach "${MOUNT_DEVICE}" -quiet || true
  fi
}

trap cleanup_mount EXIT

SOURCE_APP_PATH="${1:-}"
INSTALL_DIR="${2:-}"

if [[ -z "${SOURCE_APP_PATH}" ]]; then
  if ! SOURCE_APP_PATH="$(detect_source_app)"; then
    echo "Could not find ${APP_NAME} automatically."
    print_usage
    exit 1
  fi
fi

if [[ -f "${SOURCE_APP_PATH}" && "${SOURCE_APP_PATH}" == *.dmg ]]; then
  echo "Detected DMG source: ${SOURCE_APP_PATH}"
  xattr -dr com.apple.quarantine "${SOURCE_APP_PATH}" || true
  ATTACH_OUTPUT="$(hdiutil attach "${SOURCE_APP_PATH}" -nobrowse)"
  MOUNT_DEVICE="$(printf '%s\n' "${ATTACH_OUTPUT}" | awk '/^\/dev\// {print $1; exit}')"
  MOUNT_POINT="$(printf '%s\n' "${ATTACH_OUTPUT}" | awk -F '\t' '/\/Volumes\// {print $NF; exit}')"
  if [[ -z "${MOUNT_POINT}" ]]; then
    echo "Could not determine mounted DMG path."
    exit 1
  fi
  SOURCE_APP_PATH="${MOUNT_POINT}/${APP_NAME}"
fi

if [[ ! -d "${SOURCE_APP_PATH}" ]]; then
  echo "Source app not found: ${SOURCE_APP_PATH}"
  print_usage
  exit 1
fi

if [[ -z "${INSTALL_DIR}" ]]; then
  if [[ -w "/Applications" ]]; then
    INSTALL_DIR="/Applications"
  else
    INSTALL_DIR="$HOME/Applications"
  fi
fi

mkdir -p "${INSTALL_DIR}"
TARGET_APP_PATH="${INSTALL_DIR}/${APP_NAME}"

echo "Installing ${APP_NAME} from: ${SOURCE_APP_PATH}"
echo "Target location: ${TARGET_APP_PATH}"

if [[ -d "${TARGET_APP_PATH}" ]]; then
  echo "Replacing existing app at ${TARGET_APP_PATH}"
  rm -rf "${TARGET_APP_PATH}"
fi

# ditto preserves app bundle metadata and permissions better than cp -R.
ditto "${SOURCE_APP_PATH}" "${TARGET_APP_PATH}"

# Clear quarantine so unsigned prerelease builds can launch.
xattr -cr "${TARGET_APP_PATH}" || true

echo "Install complete."
echo "You can launch it with: open \"${TARGET_APP_PATH}\""
