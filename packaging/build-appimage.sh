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

# Keep build-host details (username, home dir, project path, toolchain paths)
# out of the binary: remap every absolute path rustc embeds in panic/debug info.
REMAPS=(
    "--remap-path-prefix=$ROOT=/build/sound-checker"
    "--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/build/cargo"
    "--remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/build/rustup"
    "--remap-path-prefix=$HOME=/build/home"
)
RUSTFLAGS="${REMAPS[*]}" cargo build --release --locked

rm -rf "$BUILD"
mkdir -p "$APPDIR/usr/bin" "$LIBDIR" "$APPDIR/usr/share/glib-2.0/schemas"
install -m755 target/release/sound-checker "$APPDIR/usr/bin/sound-checker"
# Drop symbols and the toolchain/linker identification comment.
strip --strip-all --remove-section=.comment --remove-section=.note.gnu.build-id \
    "$APPDIR/usr/bin/sound-checker"

# Oldest glibc we promise to run on. Anything bundled (or the binary itself)
# that needs a newer glibc would fail with "GLIBC_x.y not found" on such hosts.
MAX_GLIBC=2.43

# Highest GLIBC_x.y symbol version an ELF file requires (empty if none).
glibc_req() {
    objdump -T "$1" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1
}
# True if $1 needs a glibc newer than MAX_GLIBC.
too_new() {
    local v
    v="$(glibc_req "$1")"
    [[ -n "$v" && "$(printf '%s\n%s\n' "$v" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]]
}

if too_new "$APPDIR/usr/bin/sound-checker"; then
    echo "error: sound-checker needs glibc $(glibc_req "$APPDIR/usr/bin/sound-checker") > $MAX_GLIBC; build on a host with glibc <= $MAX_GLIBC" >&2
    exit 1
fi

# Bundle shared libraries (transitively). Libraries needing a glibc newer than
# MAX_GLIBC are left out so the target system's own copy is used instead.
bundle_libs() {
    { ldd "$@" 2>/dev/null || true; } | awk '/=> \//{print $3}' | while read -r lib; do
        name="$(basename "$lib")"
        if [[ "$name" =~ $EXCLUDE || -e "$LIBDIR/$name" ]]; then
            continue
        fi
        if too_new "$lib"; then
            echo "note: not bundling $name (needs glibc $(glibc_req "$lib") > $MAX_GLIBC)" >&2
            continue
        fi
        cp -L "$lib" "$LIBDIR/$name"
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
# Only generic GTK/GNOME schemas -- not the host's other installed apps/desktop.
cp /usr/share/glib-2.0/schemas/org.gtk.*.xml /usr/share/glib-2.0/schemas/org.gnome.desktop.*.xml \
    "$APPDIR/usr/share/glib-2.0/schemas/"
glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas"
mkdir -p "$APPDIR/usr/share/icons"
cp -r /usr/share/icons/Adwaita "$APPDIR/usr/share/icons/"

# Desktop entry + icon.
sed "s/^Icon=.*/Icon=$APP_ID/" "packaging/$APP_ID.desktop" > "$APPDIR/$APP_ID.desktop"
install -Dm644 "packaging/$APP_ID.svg" "$APPDIR/$APP_ID.svg"
install -Dm644 "packaging/$APP_ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$APPDIR/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"

cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="$HERE"
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
# Normalise ownership/timestamps so the squashfs doesn't carry the build user.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || echo 1700000000)}"
find "$APPDIR" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
ARCH=x86_64 appimagetool --mksquashfs-opt -all-root "$APPDIR" "$OUT" 2>/dev/null \
    || ARCH=x86_64 appimagetool "$APPDIR" "$OUT"
echo "Built $OUT"
