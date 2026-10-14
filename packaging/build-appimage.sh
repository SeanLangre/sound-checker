#!/usr/bin/env bash
# Build a Sound-Checker AppImage from the release binary.
# Needs: cargo, appimagetool, glib-compile-schemas, and the GTK4/libadwaita
# runtime installed on the build host (its libraries get bundled).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID=dev.soundchecker.SoundChecker
BUILD="$ROOT/build-appimage"
APPDIR="$BUILD/AppDir"
LIBDIR="$APPDIR/usr/lib"

# Libraries that must come from the host (glibc, GPU/driver stack, udev).
EXCLUDE='^(linux-vdso|ld-linux.*|libc|libm|libdl|libpthread|librt|libresolv|libutil|libnsl|libGL|libGLX|libGLdispatch|libEGL|libgbm|libdrm|libvulkan|libudev|libsystemd)\.so'

cd "$ROOT"
cargo build --release --locked

rm -rf "$BUILD"
mkdir -p "$APPDIR/usr/bin" "$LIBDIR" "$APPDIR/usr/share/glib-2.0/schemas"
install -m755 target/release/sound-checker "$APPDIR/usr/bin/sound-checker"

# Bundle shared libraries (transitively).
bundle_libs() {
    ldd "$@" 2>/dev/null | awk '/=> \//{print $3}' | while read -r lib; do
        name="$(basename "$lib")"
        [[ "$name" =~ $EXCLUDE ]] && continue
        [[ -e "$LIBDIR/$name" ]] || cp -L "$lib" "$LIBDIR/$name"
    done
}
bundle_libs "$APPDIR/usr/bin/sound-checker"

# GDK-Pixbuf loaders + GIO modules.
PIXBUF_SRC="$(pkg-config --variable=gdk_pixbuf_moduledir gdk-pixbuf-2.0)"
if [[ -d "$PIXBUF_SRC" ]]; then
    mkdir -p "$LIBDIR/gdk-pixbuf/loaders"
    cp -L "$PIXBUF_SRC"/*.so "$LIBDIR/gdk-pixbuf/loaders/" 2>/dev/null || true
fi
GIO_SRC="$(pkg-config --variable=giomoduledir gio-2.0)"
if [[ -d "$GIO_SRC" ]]; then
    mkdir -p "$LIBDIR/gio/modules"
    cp -L "$GIO_SRC"/*.so "$LIBDIR/gio/modules/" 2>/dev/null || true
fi
# Pull in whatever those plugins need.
bundle_libs "$LIBDIR"/gdk-pixbuf/loaders/*.so "$LIBDIR"/gio/modules/*.so
rm -f "$LIBDIR"/gio/modules/*.cache

# Schemas, icon theme.
cp /usr/share/glib-2.0/schemas/*.xml "$APPDIR/usr/share/glib-2.0/schemas/"
cp /usr/share/glib-2.0/schemas/*.override "$APPDIR/usr/share/glib-2.0/schemas/" 2>/dev/null || true
glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas"
mkdir -p "$APPDIR/usr/share/icons"
cp -r /usr/share/icons/Adwaita /usr/share/icons/hicolor "$APPDIR/usr/share/icons/"

# Desktop entry + icon.
sed "s/^Icon=.*/Icon=$APP_ID/" "packaging/$APP_ID.desktop" > "$APPDIR/$APP_ID.desktop"
install -Dm644 "packaging/$APP_ID.svg" "$APPDIR/$APP_ID.svg"
install -Dm644 "packaging/$APP_ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$APPDIR/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"

cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GSETTINGS_SCHEMA_DIR="$HERE/usr/share/glib-2.0/schemas"
export GIO_MODULE_DIR="$HERE/usr/lib/gio/modules"
# Rebuild the pixbuf loader cache at runtime (paths differ per mount point).
if [ -d "$HERE/usr/lib/gdk-pixbuf/loaders" ] && command -v gdk-pixbuf-query-loaders >/dev/null 2>&1; then
    CACHE="${XDG_RUNTIME_DIR:-/tmp}/sound-checker-pixbuf-$$.cache"
    gdk-pixbuf-query-loaders "$HERE"/usr/lib/gdk-pixbuf/loaders/*.so > "$CACHE" 2>/dev/null \
        && export GDK_PIXBUF_MODULE_FILE="$CACHE"
fi
exec "$HERE/usr/bin/sound-checker" "$@"
RUN
chmod +x "$APPDIR/AppRun"

OUT="$ROOT/Sound-Checker-x86_64.AppImage"
ARCH=x86_64 appimagetool "$APPDIR" "$OUT"
echo "Built $OUT"
