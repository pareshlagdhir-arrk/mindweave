#!/usr/bin/env bash
# Builds Mindweave from source and installs it for the current user
# (macOS and Linux). Windows: scripts/install.ps1.
#
#   scripts/install.sh              build and install the app and the MCP server
#   scripts/install.sh --skip-mcp   the app only
#   scripts/install.sh --skip-app   the MCP server only
#
# Installs, without sudo:
#   macOS  ~/Applications/Mindweave.app
#   Linux  ~/.local/bin/mindweave, a menu entry and an icon
#   both   ~/.cargo/bin/mindweave-mcp (the MCP server for AI agents)
#
# MINDWEAVE_APP_DIR and MINDWEAVE_BIN_DIR override where the app and the
# MCP server go.
set -euo pipefail

skip_app=0
skip_mcp=0
for arg in "$@"; do
  case "$arg" in
    --skip-app) skip_app=1 ;;
    --skip-mcp) skip_mcp=1 ;;
    -h | --help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg (see --help)" >&2
      exit 2
      ;;
  esac
done
if [[ $skip_app == 1 && $skip_mcp == 1 ]]; then
  echo "Nothing to do: both --skip-app and --skip-mcp were given." >&2
  exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

os="$(uname -s)"
case "$os" in
  Darwin | Linux) ;;
  *) fail "this script is for macOS and Linux; on Windows run scripts/install.ps1." ;;
esac

# --- Prerequisites -----------------------------------------------------

command -v cargo > /dev/null ||
  fail "Rust is not installed. Install it from https://rustup.rs:
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
then open a new terminal and run this script again."

if [[ $os == Darwin ]]; then
  xcode-select -p > /dev/null 2>&1 ||
    fail "the Xcode command-line tools are missing. Run: xcode-select --install"
elif [[ $skip_app == 0 ]]; then
  # What gpui and the file dialogs build against.
  missing=()
  command -v pkg-config > /dev/null || missing+=(pkg-config)
  if command -v pkg-config > /dev/null; then
    for lib in xkbcommon xkbcommon-x11 wayland-client x11-xcb fontconfig freetype2 gtk+-3.0; do
      pkg-config --exists "$lib" || missing+=("$lib")
    done
  fi
  command -v cc > /dev/null || missing+=("a C compiler")
  if ((${#missing[@]})); then
    echo "Missing build dependencies: ${missing[*]}" >&2
    echo "Debian or Ubuntu:" >&2
    echo "  sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev \\" >&2
    echo "    libwayland-dev libvulkan-dev libx11-xcb-dev libxcb1-dev libfontconfig1-dev libfreetype6-dev \\" >&2
    echo "    libssl-dev libgtk-3-dev" >&2
    echo "Fedora:" >&2
    echo "  sudo dnf install gcc pkgconf libxkbcommon-devel libxkbcommon-x11-devel wayland-devel \\" >&2
    echo "    vulkan-loader-devel libxcb-devel fontconfig-devel freetype-devel openssl-devel gtk3-devel" >&2
    exit 1
  fi
fi

# --- Build -------------------------------------------------------------

packages=()
[[ $skip_app == 0 ]] && packages+=(-p mindweave-desktop)
[[ $skip_mcp == 0 ]] && packages+=(-p mindweave-mcp)
say "Building (release; the first build takes several minutes)"
cargo build --release --locked "${packages[@]}"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
bin_dir="${MINDWEAVE_BIN_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}"

# --- Install the app ---------------------------------------------------

if [[ $skip_app == 0 ]]; then
  if [[ $os == Darwin ]]; then
    app_dir="${MINDWEAVE_APP_DIR:-$HOME/Applications}"
    app="$app_dir/Mindweave.app"
    say "Installing $app"
    mkdir -p "$app_dir"
    stage="$(mktemp -d "$app_dir/.Mindweave.XXXXXX")"
    mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
    cp target/release/mindweave "$stage/Contents/MacOS/mindweave"
    cp crates/mindweave-desktop/assets/app-icon.icns "$stage/Contents/Resources/app-icon.icns"
    cat > "$stage/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Mindweave</string>
  <key>CFBundleDisplayName</key><string>Mindweave</string>
  <key>CFBundleIdentifier</key><string>io.github.pareshlagdhir-arrk.mindweave</string>
  <key>CFBundleExecutable</key><string>mindweave</string>
  <key>CFBundleIconFile</key><string>app-icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
    # Replace an older copy only once the new one is complete.
    rm -rf "$app"
    mv "$stage" "$app"
    # Let Finder, Spotlight and Launchpad pick up the new bundle.
    touch "$app"
  else
    app_dir="${MINDWEAVE_APP_DIR:-$HOME/.local}"
    say "Installing $app_dir/bin/mindweave"
    install -Dm755 target/release/mindweave "$app_dir/bin/mindweave"
    install -Dm644 crates/mindweave-desktop/assets/app-icon.svg \
      "$app_dir/share/icons/hicolor/scalable/apps/mindweave.svg"
    mkdir -p "$app_dir/share/applications"
    cat > "$app_dir/share/applications/mindweave.desktop" << EOF
[Desktop Entry]
Type=Application
Name=Mindweave
Comment=Mind maps and diagrams
Exec=$app_dir/bin/mindweave
Icon=mindweave
Terminal=false
Categories=Office;Graphics;
EOF
    command -v update-desktop-database > /dev/null &&
      update-desktop-database "$app_dir/share/applications" > /dev/null 2>&1 || true
    case ":$PATH:" in
      *":$app_dir/bin:"*) ;;
      *) echo "Note: $app_dir/bin is not on your PATH; start Mindweave from the app menu, or add it." ;;
    esac
  fi
fi

# --- Install the MCP server ---------------------------------------------

if [[ $skip_mcp == 0 ]]; then
  say "Installing $bin_dir/mindweave-mcp"
  mkdir -p "$bin_dir"
  # Copy then rename, so a running copy is never half-written.
  cp target/release/mindweave-mcp "$bin_dir/.mindweave-mcp.new"
  chmod 755 "$bin_dir/.mindweave-mcp.new"
  mv "$bin_dir/.mindweave-mcp.new" "$bin_dir/mindweave-mcp"
fi

# --- Done ----------------------------------------------------------------

echo
say "Done (Mindweave $version)"
if [[ $skip_app == 0 ]]; then
  if [[ $os == Darwin ]]; then
    echo "  Open Mindweave from Spotlight or Launchpad, or: open \"$app\""
  else
    echo "  Open Mindweave from your app menu, or run: $app_dir/bin/mindweave"
  fi
fi
if [[ $skip_mcp == 0 ]]; then
  echo "  To let an AI agent use Mindweave, register the MCP server once, for example:"
  echo "    claude mcp add --scope user mindweave -- \"$bin_dir/mindweave-mcp\""
  echo "    codex mcp add mindweave -- \"$bin_dir/mindweave-mcp\""
  echo "  Other clients: docs/agents.md"
fi
