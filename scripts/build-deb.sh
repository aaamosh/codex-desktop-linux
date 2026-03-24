#!/bin/bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="$REPO_DIR/codex-app"
PKG_ROOT="$REPO_DIR/dist/deb-root"
DIST_DIR="$REPO_DIR/dist"
CONTROL_TEMPLATE="$REPO_DIR/packaging/linux/control"
DESKTOP_TEMPLATE="$REPO_DIR/packaging/linux/codex-desktop.desktop"
ICON_SOURCE="$REPO_DIR/assets/codex.png"
APP_DIR="${APP_DIR_OVERRIDE:-$APP_DIR}"
DIST_DIR="${DIST_DIR_OVERRIDE:-$DIST_DIR}"

PACKAGE_NAME="${PACKAGE_NAME:-codex-desktop}"
PACKAGE_VERSION="${PACKAGE_VERSION:-$(date +%Y.%m.%d)}"
UPDATE_BUILDER_ROOT="$PKG_ROOT/opt/$PACKAGE_NAME/update-builder"

info()  { echo "[INFO] $*" >&2; }
error() { echo "[ERROR] $*" >&2; exit 1; }

map_arch() {
    case "$(dpkg --print-architecture)" in
        amd64|arm64|armhf)
            dpkg --print-architecture
            ;;
        *)
            error "Unsupported Debian architecture: $(dpkg --print-architecture)"
            ;;
    esac
}

main() {
    [ -d "$APP_DIR" ] || error "Missing app directory: $APP_DIR. Run ./install.sh first."
    [ -x "$APP_DIR/start.sh" ] || error "Missing launcher: $APP_DIR/start.sh"
    [ -f "$CONTROL_TEMPLATE" ] || error "Missing control template: $CONTROL_TEMPLATE"
    [ -f "$DESKTOP_TEMPLATE" ] || error "Missing desktop template: $DESKTOP_TEMPLATE"
    [ -f "$ICON_SOURCE" ] || error "Missing icon: $ICON_SOURCE"
    command -v dpkg-deb >/dev/null 2>&1 || error "dpkg-deb is required"
    command -v dpkg >/dev/null 2>&1 || error "dpkg is required"

    local arch output_file
    arch="$(map_arch)"
    output_file="$DIST_DIR/${PACKAGE_NAME}_${PACKAGE_VERSION}_${arch}.deb"

    info "Preparing package root at $PKG_ROOT"
    rm -rf "$PKG_ROOT"
    mkdir -p \
        "$PKG_ROOT/DEBIAN" \
        "$PKG_ROOT/opt" \
        "$PKG_ROOT/usr/bin" \
        "$PKG_ROOT/usr/share/applications" \
        "$PKG_ROOT/usr/share/icons/hicolor/256x256/apps"

    cp -a "$APP_DIR" "$PKG_ROOT/opt/$PACKAGE_NAME"
    cp "$DESKTOP_TEMPLATE" "$PKG_ROOT/usr/share/applications/$PACKAGE_NAME.desktop"
    cp "$ICON_SOURCE" "$PKG_ROOT/usr/share/icons/hicolor/256x256/apps/$PACKAGE_NAME.png"
    mkdir -p "$UPDATE_BUILDER_ROOT/scripts" "$UPDATE_BUILDER_ROOT/packaging/linux" "$UPDATE_BUILDER_ROOT/assets"
    cp "$REPO_DIR/install.sh" "$UPDATE_BUILDER_ROOT/install.sh"
    cp "$REPO_DIR/scripts/build-deb.sh" "$UPDATE_BUILDER_ROOT/scripts/build-deb.sh"
    cp "$REPO_DIR/packaging/linux/control" "$UPDATE_BUILDER_ROOT/packaging/linux/control"
    cp "$REPO_DIR/packaging/linux/codex-desktop.desktop" "$UPDATE_BUILDER_ROOT/packaging/linux/codex-desktop.desktop"
    cp "$REPO_DIR/assets/codex.png" "$UPDATE_BUILDER_ROOT/assets/codex.png"

    cat > "$PKG_ROOT/usr/bin/$PACKAGE_NAME" <<SCRIPT
#!/bin/bash
exec /opt/$PACKAGE_NAME/start.sh "\$@"
SCRIPT
    chmod 0755 "$PKG_ROOT/usr/bin/$PACKAGE_NAME"

    sed \
        -e "s/__VERSION__/$PACKAGE_VERSION/g" \
        -e "s/__ARCH__/$arch/g" \
        "$CONTROL_TEMPLATE" > "$PKG_ROOT/DEBIAN/control"
    chmod 0644 "$PKG_ROOT/DEBIAN/control"

    mkdir -p "$DIST_DIR"
    info "Building $output_file"
    dpkg-deb --root-owner-group --build "$PKG_ROOT" "$output_file" >&2
    info "Built package: $output_file"
}

main "$@"
