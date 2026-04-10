#!/usr/bin/env bash
set -euo pipefail

# unified-search-mcp installer
# Usage: curl -fsSL https://raw.githubusercontent.com/ganesh-tt/unified-search-mcp/master/install.sh | bash

REPO="https://github.com/ganesh-tt/unified-search-mcp.git"
INSTALL_DIR="${UNIFIED_SEARCH_DIR:-$HOME/.unified-search}"
BIN_DIR="${INSTALL_DIR}/bin"
CONFIG_DIR="${INSTALL_DIR}"

info()  { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
ok()    { printf "\033[1;32m OK\033[0m %s\n" "$1"; }
err()   { printf "\033[1;31mERR\033[0m %s\n" "$1" >&2; exit 1; }

# --- Check / install Rust ---
if command -v cargo &>/dev/null; then
    ok "Rust found: $(rustc --version)"
else
    info "Installing Rust toolchain..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    ok "Rust installed: $(rustc --version)"
fi

# --- Check minimum Rust version ---
RUST_VER=$(rustc --version | grep -oE '[0-9]+\.[0-9]+')
MAJOR=$(echo "$RUST_VER" | cut -d. -f1)
MINOR=$(echo "$RUST_VER" | cut -d. -f2)
if [ "$MAJOR" -lt 1 ] || { [ "$MAJOR" -eq 1 ] && [ "$MINOR" -lt 80 ]; }; then
    info "Updating Rust (need >= 1.80, have $RUST_VER)..."
    rustup update stable
fi

# --- Clone or update ---
if [ -d "${INSTALL_DIR}/src" ]; then
    info "Updating existing installation..."
    git -C "${INSTALL_DIR}/src" pull --ff-only
else
    info "Cloning unified-search-mcp..."
    mkdir -p "$INSTALL_DIR"
    git clone "$REPO" "${INSTALL_DIR}/src"
fi

# --- Build ---
info "Building release binary (this takes ~30s on first build)..."
cargo build --release --manifest-path "${INSTALL_DIR}/src/Cargo.toml"

# --- Install binary ---
mkdir -p "$BIN_DIR"
cp "${INSTALL_DIR}/src/target/release/unified-search-mcp" "$BIN_DIR/"
ok "Binary installed: ${BIN_DIR}/unified-search-mcp"

# --- Copy example config if no config exists ---
if [ ! -f "${CONFIG_DIR}/config.yaml" ]; then
    cp "${INSTALL_DIR}/src/config.example.yaml" "${CONFIG_DIR}/config.yaml"
    info "Config created: ${CONFIG_DIR}/config.yaml (edit to enable sources)"
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
