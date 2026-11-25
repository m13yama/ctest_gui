#!/bin/sh
# Installs ctgui from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/m13yama/ctest_gui/main/install.sh | sh
#
# Options (pass after `sh -s --` when piping):
#   --version <tag>   install a specific release (e.g. v0.1.0) instead of the latest
#   --dir <path>      install directory (default: ~/.local/bin)
#   --uninstall       remove ctgui and its desktop launcher
#
# Environment variables CTGUI_VERSION and CTGUI_INSTALL_DIR can be used instead of the options.

set -eu

REPO="${CTGUI_REPO:-m13yama/ctest_gui}"
VERSION="${CTGUI_VERSION:-latest}"
INSTALL_DIR="${CTGUI_INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
DESKTOP_FILE="$DATA_DIR/applications/ctgui.desktop"
ICON_FILE="$DATA_DIR/icons/hicolor/256x256/apps/ctgui.png"
# Base URL of the release assets; can be overridden to use a mirror.
BASE_URL="${CTGUI_BASE_URL:-https://github.com/$REPO/releases}"
UNINSTALL=0

info() { printf '\033[1;32mctgui:\033[0m %s\n' "$*"; }
error() {
    printf '\033[1;31mctgui:\033[0m %s\n' "$*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
    --version)
        [ $# -ge 2 ] || error "--version requires a value"
        VERSION="$2"
        shift 2
        ;;
    --dir)
        [ $# -ge 2 ] || error "--dir requires a value"
        INSTALL_DIR="$2"
        shift 2
        ;;
    --uninstall)
        UNINSTALL=1
        shift
        ;;
    -h | --help)
        cat <<EOF
Usage: install.sh [--version <tag>] [--dir <path>] [--uninstall]

Installs ctgui and adds CTest GUI to your applications menu.

  --version <tag>   install a specific release (e.g. v0.1.0) instead of the latest
  --dir <path>      install directory (default: ~/.local/bin)
  --uninstall       remove ctgui and its desktop launcher
EOF
        exit 0
        ;;
    *) error "unknown option: $1" ;;
    esac
done

[ "$(uname -s)" = "Linux" ] || error "only Linux is supported (detected $(uname -s))"
INSTALL_DIR="$(realpath -m -- "$INSTALL_DIR")"
# Desktop entries unescape string values before interpreting quoted Exec arguments.
launcher_arg=$(printf '%s' "$INSTALL_DIR/ctgui-desktop" | sed -z \
    -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g' -e 's/%/%%/g' -e 's/\n/\\n/g' -e 's/\r/\\r/g')
desktop_exec="Exec=/bin/sh \"$launcher_arg\""

if [ "$UNINSTALL" -eq 1 ]; then
    if [ -f "$DESKTOP_FILE" ] && grep -Fxq -- "$desktop_exec" "$DESKTOP_FILE"; then
        rm -f -- "$DESKTOP_FILE"
        info "removed $DESKTOP_FILE"
    fi
    if [ ! -e "$DESKTOP_FILE" ]; then
        rm -f -- "$ICON_FILE"
    fi
    rm -f -- "$INSTALL_DIR/ctgui-desktop"
    if [ -e "$INSTALL_DIR/ctgui" ]; then
        rm -f "$INSTALL_DIR/ctgui"
        info "removed $INSTALL_DIR/ctgui"
    else
        info "ctgui is not installed in $INSTALL_DIR"
    fi
    exit 0
fi

case "$(uname -m)" in
x86_64 | amd64) TARGET="x86_64-unknown-linux-gnu" ;;
aarch64 | arm64) TARGET="aarch64-unknown-linux-gnu" ;;
*) error "unsupported architecture: $(uname -m)" ;;
esac

if command -v curl >/dev/null 2>&1; then
    download() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    download() { wget -q -O "$2" "$1"; }
else
    error "curl or wget is required"
fi

ASSET="ctgui-$TARGET.tar.gz"
if [ "$VERSION" = "latest" ]; then
    URL="$BASE_URL/latest/download/$ASSET"
else
    URL="$BASE_URL/download/$VERSION/$ASSET"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

info "downloading $URL"
download "$URL" "$TMP_DIR/$ASSET" || error "failed to download $URL"

if download "$URL.sha256" "$TMP_DIR/$ASSET.sha256" 2>/dev/null; then
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$TMP_DIR" && sha256sum -c --status "$ASSET.sha256") || error "checksum mismatch for $ASSET"
        info "checksum verified"
    else
        info "sha256sum not found; skipping checksum verification"
    fi
else
    info "checksum file not found; skipping checksum verification"
fi

tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"
[ -f "$TMP_DIR/ctgui" ] || error "ctgui binary not found in $ASSET"

mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP_DIR/ctgui" "$INSTALL_DIR/ctgui"
installed_version=$("$INSTALL_DIR/ctgui" --version)
info "installed $installed_version to $INSTALL_DIR/ctgui"

# Older releases shipped only the binary and documentation.
if [ ! -f "$TMP_DIR/icon.png" ] && [ "$BASE_URL" = "https://github.com/$REPO/releases" ]; then
    icon_ref="$VERSION"
    [ "$icon_ref" != latest ] || icon_ref="v${installed_version#ctgui }"
    if download "https://raw.githubusercontent.com/$REPO/$icon_ref/assets/icon.png" "$TMP_DIR/legacy-icon.png"; then
        mv "$TMP_DIR/legacy-icon.png" "$TMP_DIR/icon.png"
    fi
fi
icon=applications-development
if [ -f "$TMP_DIR/icon.png" ]; then
    install -Dm644 "$TMP_DIR/icon.png" "$ICON_FILE"
fi
[ ! -f "$ICON_FILE" ] || icon=ctgui

cat > "$TMP_DIR/ctgui-desktop" <<'EOF'
#!/bin/sh
workspace=$(zenity --file-selection --directory --title="Open a CTest workspace") || exit $?
if ! message=$("${0%/*}/ctgui" "$workspace" 2>&1); then
    zenity --error --no-markup --text="$message"
    exit 1
fi
EOF
install -m755 "$TMP_DIR/ctgui-desktop" "$INSTALL_DIR/ctgui-desktop"

cat > "$TMP_DIR/ctgui.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=CTest GUI
Comment=Discover and run CTest tests in a workspace
$desktop_exec
Icon=$icon
Terminal=false
Categories=Development;
StartupWMClass=ctgui
EOF
install -Dm644 "$TMP_DIR/ctgui.desktop" "$DESKTOP_FILE"
info "installed $DESKTOP_FILE"
if ! command -v zenity >/dev/null 2>&1; then
    info "install zenity to select a workspace from the applications menu"
fi

case ":$PATH:" in
*":$INSTALL_DIR:"*) ;;
*) info "add $INSTALL_DIR to PATH to run ctgui, e.g. echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc" ;;
esac
