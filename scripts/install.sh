#!/bin/sh
# Install peanutbutter from GitHub releases.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/calamity-m/peanutbutter/main/scripts/install.sh | sh
#
# Flags (pass through the pipe with: ... | sh -s -- --dry-run):
#   --dry-run       show what would be downloaded and installed, then exit
#
# Options (env vars):
#   PB_VERSION      release tag to install, e.g. "v0.23.0" (default: latest release)
#   PB_INSTALL_DIR  directory to place the binary in (default: ~/.local/bin)
#   PB_TARGET       release target triple override, e.g. "x86_64-unknown-linux-gnu"
#                   (default: x86_64-unknown-linux-musl, the static build)

set -eu

REPO="calamity-m/peanutbutter"
BINARY="peanutbutter"

INSTALL_DIR="${PB_INSTALL_DIR:-"$HOME/.local/bin"}"
VERSION="${PB_VERSION:-latest}"

DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        *)
            printf 'install.sh: unknown option: %s (supported: --dry-run)\n' "$arg" >&2
            exit 1
            ;;
    esac
done

# Only emit escapes when stdout is a tty and NO_COLOR is unset.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$(printf '\033[1m')
    RED=$(printf '\033[31m')
    GREEN=$(printf '\033[32m')
    YELLOW=$(printf '\033[33m')
    CYAN=$(printf '\033[36m')
    RESET=$(printf '\033[0m')
else
    BOLD=""
    RED=""
    GREEN=""
    YELLOW=""
    CYAN=""
    RESET=""
fi

err() {
    printf '%sinstall.sh: %s%s\n' "$RED" "$1" "$RESET" >&2
    exit 1
}

detect_target() {
    if [ -n "${PB_TARGET:-}" ]; then
        printf '%s' "$PB_TARGET"
        return
    fi
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
        Linux) ;;
        Darwin) err "no macOS binaries are published yet; build from source with: cargo install --git https://github.com/$REPO" ;;
        MINGW* | MSYS* | CYGWIN*) err "on Windows, download $BINARY-x86_64-pc-windows-msvc.zip from https://github.com/$REPO/releases/latest" ;;
        *) err "unsupported OS: $os" ;;
    esac
    case "$arch" in
        x86_64 | amd64) ;;
        *) err "no $arch Linux binaries are published yet; build from source with: cargo install --git https://github.com/$REPO" ;;
    esac
    # musl is statically linked and runs on any x86_64 Linux.
    printf 'x86_64-unknown-linux-musl'
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        err "need curl or wget to download releases"
    fi
}

target="$(detect_target)"
asset="$BINARY-$target.tar.gz"
if [ "$VERSION" = "latest" ]; then
    url="https://github.com/$REPO/releases/latest/download/$asset"
else
    url="https://github.com/$REPO/releases/download/$VERSION/$asset"
fi

if [ "$DRY_RUN" -eq 1 ]; then
    printf '%sDry run%s — nothing will be downloaded or installed.\n' "$BOLD" "$RESET"
    printf '  version:  %s\n' "$VERSION"
    printf '  target:   %s\n' "$target"
    printf '  download: %s\n' "$url"
    printf '  install:  %s\n' "$INSTALL_DIR/$BINARY"
    exit 0
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT INT TERM

printf '%sDownloading %s (%s)...%s\n' "$CYAN" "$BINARY" "$VERSION" "$RESET"
fetch "$url" "$tmpdir/$asset" || err "download failed: $url (does release '$VERSION' exist?)"
tar -xzf "$tmpdir/$asset" -C "$tmpdir"
[ -f "$tmpdir/$BINARY" ] || err "archive did not contain the '$BINARY' binary"

mkdir -p "$INSTALL_DIR"
install -m 755 "$tmpdir/$BINARY" "$INSTALL_DIR/$BINARY"

installed_version="$("$INSTALL_DIR/$BINARY" --version 2>/dev/null || true)"
printf '%s%sInstalled %s to %s%s\n' "$GREEN" "$BOLD" "${installed_version:-$BINARY}" "$INSTALL_DIR/$BINARY" "$RESET"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) printf '%sNote: %s is not on your PATH.%s Add it, e.g.:\n  export PATH="%s:$PATH"\n' "$YELLOW" "$INSTALL_DIR" "$RESET" "$INSTALL_DIR" ;;
esac

# $SHELL is the user's login shell even when this script runs under plain sh
# via `curl | sh`, so use it to show the right integration snippet.
case "$(basename "${SHELL:-}")" in
    bash)
        printf '%sNext:%s add to ~/.bashrc, then press Ctrl+b:\n  eval "$(%s completions bash)"\n' "$BOLD" "$RESET" "$BINARY"
        ;;
    zsh)
        printf '%sNext:%s add to ~/.zshrc, then press Ctrl+b:\n  eval "$(%s completions zsh)"\n' "$BOLD" "$RESET" "$BINARY"
        ;;
    fish)
        printf '%sNext:%s add to ~/.config/fish/config.fish, then press Ctrl+b:\n  %s completions fish | source\n' "$BOLD" "$RESET" "$BINARY"
        ;;
    *)
        printf '%sNext:%s add shell integration (see https://github.com/%s#quick-start), e.g. for bash:\n  eval "$(%s completions bash)"\n' "$BOLD" "$RESET" "$REPO" "$BINARY"
        ;;
esac
