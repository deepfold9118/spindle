#!/bin/sh
# Installs (or uninstalls) Spindle for the current user on Linux and macOS.
#
# Copies the Spindle programs to ~/.local/share/spindle, links spindle, spindle_app and
# spindle-mcp into ~/.local/bin, adds an application launcher (Linux) and starts Spindle.
# Run it from an unpacked release folder to install that release; run it on its own to
# download the latest release from GitHub, checked against the release's SHA256SUMS.
#
#   curl -fsSL https://github.com/deepfold9118/spindle/releases/latest/download/install.sh | sh
#   sh install.sh --start-at-sign-in
#   sh install.sh --uninstall
#
# Upgrading is the same command. Your library, profiles and settings live elsewhere and are
# never touched, not even by --uninstall.
set -eu

REPO="deepfold9118/spindle"
DIR="${SPINDLE_INSTALL_DIR:-$HOME/.local/share/spindle}"
VERSION="latest"
NO_PATH=0 NO_START=0 AUTOSTART=0 UNINSTALL=0
PROGRAMS="spindle spindle_app spindle-mcp"
DOCS="LICENSE NOTICE THIRD-PARTY-LICENSES.html README.md spindle.svg install.sh"
LAUNCHER="${XDG_DATA_HOME:-$HOME/.local/share}/applications/spindle.desktop"

usage() {
    cat <<EOF
usage: install.sh [--dir DIR] [--version vX.Y.Z] [--no-path] [--no-start] [--start-at-sign-in] [--uninstall]
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) DIR="$2"; shift ;;
        --version) VERSION="$2"; shift ;;
        --no-path) NO_PATH=1 ;;
        --no-start) NO_START=1 ;;
        --start-at-sign-in) AUTOSTART=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done

case "$DIR" in /*) ;; *) DIR="$PWD/$DIR" ;; esac
DIR="${DIR%/}"
OS="$(uname -s)"

stop_spindle() {
    pkill -f "^$DIR/spindle_app" 2>/dev/null || true
    i=0
    while [ $i -lt 30 ] && pgrep -f "^$DIR/spindle_app" >/dev/null 2>&1; do sleep 0.3; i=$((i + 1)); done
}

if [ "$UNINSTALL" = 1 ]; then
    stop_spindle
    if [ -x "$DIR/spindle" ]; then
        off="--path off"
        status="$("$DIR/spindle" desktop --app "$DIR/spindle_app" 2>/dev/null || true)"
        if printf '%s\n' "$status" | grep -q "^start at sign-in: *on ($DIR/"; then off="$off --autostart off"; fi
        # shellcheck disable=SC2086
        "$DIR/spindle" desktop --app "$DIR/spindle_app" $off >/dev/null
    fi
    if [ -f "$LAUNCHER" ] && grep -q "^Exec=\"\{0,1\}$DIR/" "$LAUNCHER"; then rm -f "$LAUNCHER"; fi
    for f in $PROGRAMS $DOCS; do rm -f "$DIR/$f"; done
    rmdir "$DIR" 2>/dev/null || true
    echo "Spindle is uninstalled. Your library, profiles and settings were left alone."
    exit 0
fi

# --- the release to install: next to this script, or downloaded --------------------------
WORK=""
cleanup() { if [ -n "$WORK" ]; then rm -rf "$WORK"; fi; }
trap cleanup EXIT

HERE=""
case "$0" in */install.sh|install.sh) HERE="$(cd "$(dirname "$0")" && pwd)" ;; esac
if [ -n "$HERE" ] && [ -x "$HERE/spindle_app" ]; then
    SRC="$HERE"
else
    case "$OS-$(uname -m)" in
        Linux-x86_64) PLATFORM="linux-x64" ;;
        Darwin-arm64) PLATFORM="macos-arm64" ;;
        Darwin-x86_64) PLATFORM="macos-x64" ;;
        *) echo "no Spindle release for $OS $(uname -m); build it from source" >&2; exit 1 ;;
    esac
    if command -v curl >/dev/null 2>&1; then
        fetch() { curl -fsSL -H "User-Agent: spindle-install" "$1"; }
    else
        fetch() { wget -qO- --header="User-Agent: spindle-install" "$1"; }
    fi
    if [ "$VERSION" = latest ]; then api="https://api.github.com/repos/$REPO/releases/latest"
    else api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"; fi
    tag="$(fetch "$api" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$tag" ] || { echo "couldn't find the release" >&2; exit 1; }
    name="spindle-$tag-$PLATFORM.tar.gz"
    base="https://github.com/$REPO/releases/download/$tag"
    WORK="$(mktemp -d "${TMPDIR:-/tmp}/spindle-install.XXXXXX")"
    echo "Downloading Spindle $tag..."
    fetch "$base/$name" > "$WORK/$name"
    fetch "$base/SHA256SUMS" > "$WORK/SHA256SUMS"
    want="$(grep " \*\{0,1\}$name\$" "$WORK/SHA256SUMS" | cut -d' ' -f1)"
    if command -v sha256sum >/dev/null 2>&1; then got="$(sha256sum "$WORK/$name" | cut -d' ' -f1)"
    else got="$(shasum -a 256 "$WORK/$name" | cut -d' ' -f1)"; fi
    if [ -z "$want" ] || [ "$want" != "$got" ]; then echo "SHA-256 mismatch for $name" >&2; exit 1; fi
    mkdir "$WORK/x"
    tar -xzf "$WORK/$name" -C "$WORK/x"
    SRC="$(dirname "$(find "$WORK/x" -name spindle_app -type f | head -n 1)")"
fi

# --- copy ---------------------------------------------------------------------------------
mkdir -p "$DIR"
if [ "$(cd "$SRC" && pwd)" != "$(cd "$DIR" && pwd)" ]; then
    stop_spindle
    for f in $PROGRAMS $DOCS; do
        # Replace by rename, so a running spindle-mcp keeps its old file.
        if [ -f "$SRC/$f" ]; then cp "$SRC/$f" "$DIR/.$f.new" && mv -f "$DIR/.$f.new" "$DIR/$f"; fi
    done
    if [ ! -f "$SRC/install.sh" ] && [ -f "$0" ]; then cp "$0" "$DIR/install.sh"; fi
    chmod +x "$DIR/spindle" "$DIR/spindle_app" "$DIR/spindle-mcp"
fi
if [ "$OS" = Darwin ]; then xattr -dr com.apple.quarantine "$DIR" 2>/dev/null || true; fi

# --- launcher, PATH, sign-in ------------------------------------------------------------------
if [ "$OS" = Linux ]; then
    mkdir -p "$(dirname "$LAUNCHER")"
    cat > "$LAUNCHER" <<EOF
[Desktop Entry]
Type=Application
Name=Spindle
Comment=Blender build, profile and add-on manager
Exec="$DIR/spindle_app" %f
Icon=$DIR/spindle.svg
MimeType=application/x-blender;
Categories=Graphics;3DGraphics;
StartupWMClass=spindle_app
Terminal=false
EOF
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(dirname "$LAUNCHER")" >/dev/null 2>&1 || true
fi
desk=""
[ "$NO_PATH" = 1 ] || desk="$desk --path on"
[ "$AUTOSTART" = 1 ] && desk="$desk --autostart on"
# shellcheck disable=SC2086
[ -z "$desk" ] || "$DIR/spindle" desktop --app "$DIR/spindle_app" $desk >/dev/null

echo "Spindle $("$DIR/spindle" --version | sed 's/^spindle //') is installed in $DIR"
[ "$OS" = Linux ] && echo "  application menu: Spindle"
if [ "$NO_PATH" = 0 ]; then
    case ":$PATH:" in
        *":$HOME/.local/bin:"*) echo "  spindle, spindle_app and spindle-mcp are linked in ~/.local/bin" ;;
        *) echo "  linked in ~/.local/bin, which isn't on your PATH: add  export PATH=\"\$HOME/.local/bin:\$PATH\"  to your shell profile" ;;
    esac
fi
[ "$AUTOSTART" = 1 ] && echo "  starts in the tray when you sign in"
if [ "$NO_START" = 0 ]; then
    (cd "$DIR" && nohup "$DIR/spindle_app" >/dev/null 2>&1 &)
fi
