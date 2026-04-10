#!/usr/bin/env bash
set -euo pipefail

# unified-search-mcp installer
# Usage: curl -fsSL https://raw.githubusercontent.com/ganesh-tt/unified-search-mcp/master/install.sh | bash
#
# Downloads a pre-built binary if available, otherwise falls back to building from source.

REPO_OWNER="ganesh-tt"
REPO_NAME="unified-search-mcp"
REPO_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}.git"
INSTALL_DIR="${UNIFIED_SEARCH_DIR:-$HOME/.unified-search}"
BIN_DIR="${INSTALL_DIR}/bin"
CONFIG_DIR="${INSTALL_DIR}"

info()  { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
ok()    { printf "\033[1;32m OK\033[0m %s\n" "$1"; }
warn()  { printf "\033[1;33mWRN\033[0m %s\n" "$1"; }
err()   { printf "\033[1;31mERR\033[0m %s\n" "$1" >&2; exit 1; }

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)      os="" ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)             arch="" ;;
    esac

    if [ -n "$os" ] && [ -n "$arch" ]; then
        echo "${REPO_NAME}-${os}-${arch}"
    fi
}

get_latest_tag() {
    if command -v gh &>/dev/null; then
        gh release view --repo "${REPO_OWNER}/${REPO_NAME}" --json tagName -q .tagName 2>/dev/null || true
    elif command -v curl &>/dev/null; then
        curl -fsSL "https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest" 2>/dev/null \
            | grep '"tag_name"' | head -1 | cut -d'"' -f4 || true
    fi
}

try_download_binary() {
    local artifact="$1" tag="$2"
    local url="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/download/${tag}/${artifact}.tar.gz"

    info "Downloading pre-built binary (${artifact}, ${tag})..."
    local tmpdir
    tmpdir="$(mktemp -d)"
    if curl -fsSL "$url" -o "${tmpdir}/${artifact}.tar.gz" 2>/dev/null; then
        tar xzf "${tmpdir}/${artifact}.tar.gz" -C "$tmpdir"
        mkdir -p "$BIN_DIR"
        mv "${tmpdir}/unified-search-mcp" "${BIN_DIR}/unified-search-mcp"
        chmod +x "${BIN_DIR}/unified-search-mcp"
        rm -rf "$tmpdir"
        return 0
    else
        rm -rf "$tmpdir"
        return 1
    fi
}

build_from_source() {
    # --- Check / install Rust ---
    if command -v cargo &>/dev/null; then
        ok "Rust found: $(rustc --version)"
    else
        info "Installing Rust toolchain..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
        ok "Rust installed: $(rustc --version)"
    fi

    # --- Check minimum Rust version ---
    local rust_ver major minor
    rust_ver=$(rustc --version | grep -oE '[0-9]+\.[0-9]+')
    major=$(echo "$rust_ver" | cut -d. -f1)
    minor=$(echo "$rust_ver" | cut -d. -f2)
    if [ "$major" -lt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -lt 80 ]; }; then
        info "Updating Rust (need >= 1.80, have $rust_ver)..."
        rustup update stable
    fi

    # --- Clone or update ---
    if [ -d "${INSTALL_DIR}/src" ]; then
        info "Updating existing source..."
        git -C "${INSTALL_DIR}/src" pull --ff-only
    else
        info "Cloning unified-search-mcp..."
        mkdir -p "$INSTALL_DIR"
        git clone "$REPO_URL" "${INSTALL_DIR}/src"
    fi

    # --- Build ---
    info "Building release binary (this takes ~30s on first build)..."
    cargo build --release --manifest-path "${INSTALL_DIR}/src/Cargo.toml"

    mkdir -p "$BIN_DIR"
    cp "${INSTALL_DIR}/src/target/release/unified-search-mcp" "$BIN_DIR/"
}

# ==========================================================================
# Main
# ==========================================================================

mkdir -p "$INSTALL_DIR"

INSTALLED=false
ARTIFACT="$(detect_platform)"
TAG="$(get_latest_tag)"

# Try pre-built binary first
if [ -n "$ARTIFACT" ] && [ -n "$TAG" ]; then
    if try_download_binary "$ARTIFACT" "$TAG"; then
        ok "Binary installed: ${BIN_DIR}/unified-search-mcp"
        INSTALLED=true
    else
        warn "Pre-built binary not available for ${ARTIFACT}. Building from source..."
    fi
else
    info "No pre-built binary for this platform. Building from source..."
fi

# Fall back to source build
if [ "$INSTALLED" = false ]; then
    build_from_source
    ok "Binary installed: ${BIN_DIR}/unified-search-mcp"
fi

# --- Copy example config if no config exists ---
if [ ! -f "${CONFIG_DIR}/config.yaml" ]; then
    # Try to get config from source clone or download it
    if [ -f "${INSTALL_DIR}/src/config.example.yaml" ]; then
        cp "${INSTALL_DIR}/src/config.example.yaml" "${CONFIG_DIR}/config.yaml"
    else
        curl -fsSL "https://raw.githubusercontent.com/${REPO_OWNER}/${REPO_NAME}/master/config.example.yaml" \
            -o "${CONFIG_DIR}/config.yaml" 2>/dev/null || warn "Could not download example config"
    fi
    [ -f "${CONFIG_DIR}/config.yaml" ] && info "Config created: ${CONFIG_DIR}/config.yaml (edit to enable sources)"
else
    ok "Config exists: ${CONFIG_DIR}/config.yaml (not overwritten)"
fi

# --- Print next steps ---
BINARY="${BIN_DIR}/unified-search-mcp"
CONFIG="${CONFIG_DIR}/config.yaml"

cat <<DONE

$(printf '\033[1;32m%s\033[0m' '--- Installation complete ---')

Binary:  ${BINARY}
Config:  ${CONFIG}

Next steps:

  1. Edit config to enable sources:
     \$ ${EDITOR:-vi} ${CONFIG}

  2. Set credentials:
     export SLACK_USER_TOKEN="xoxp-..."
     export ATLASSIAN_BASE_URL="https://yourorg.atlassian.net"
     export ATLASSIAN_EMAIL="you@example.com"
     export ATLASSIAN_API_TOKEN="..."

  3. Verify setup:
     \$ ${BINARY} --verify --config ${CONFIG}

  4. Add to Claude Code (~/.claude.json):

     {
       "mcpServers": {
         "unified-search": {
           "command": "${BINARY}",
           "args": ["--config", "${CONFIG}"],
           "env": {
             "SLACK_USER_TOKEN": "xoxp-...",
             "ATLASSIAN_BASE_URL": "https://yourorg.atlassian.net",
             "ATLASSIAN_EMAIL": "you@example.com",
             "ATLASSIAN_API_TOKEN": "..."
           }
         }
       }
     }

  Or register with Claude Code CLI:
     claude mcp add unified-search \\
       -s user \\
       -e SLACK_USER_TOKEN=xoxp-... \\
       -e ATLASSIAN_BASE_URL=https://yourorg.atlassian.net \\
       -e ATLASSIAN_EMAIL=you@example.com \\
       -e ATLASSIAN_API_TOKEN=... \\
       -- ${BINARY} --config ${CONFIG}

DONE
