# Linux Sound Checker

A small native Linux desktop app for testing microphones and other audio inputs:
detect connected devices, see their levels live, spot common problems, and hear
what the mic actually captures.

Targets: Arch Linux and Fedora, desktop-environment agnostic, PipeWire or PulseAudio.
Open tasks are in [TODO.md](TODO.md).

![Linux Sound Checker showing the device list and live analysis](docs/screenshot.png)

## Design decisions

| Topic        | Choice                                                                 |
| ------------ | ---------------------------------------------------------------------- |
| Language     | Rust                                                                   |
| UI           | GTK4 + libadwaita                                                      |
| Audio API    | PulseAudio client API (`libpulse-binding` + `libpulse-glib-binding`)   |
| Distribution | Flatpak first; AUR PKGBUILD later; no RPM/COPR for the MVP             |

**Why the Pulse API:** Arch and Fedora both default to PipeWire, which serves the
Pulse protocol via `pipewire-pulse`. One backend covers both PipeWire and real
PulseAudio. `libpulse-glib-binding` runs Pulse on the GLib main loop, so server
events arrive on the GTK thread without extra plumbing.

The app connects to the user's existing audio server and never starts its own.

## Architecture

```
GTK main loop (UI thread)
├── src/pulse.rs    Pulse context (glib mainloop): sources, cards, profiles,
│                   mute/volume, subscribe events -> device model
├── src/capture.rs  Pulse record stream for the selected source -> analyzer;
│                   fixed-length recording and playback stream
├── src/activity.rs one tiny peak-detect stream per unmuted source, used for the
│                   green/grey row colouring (like pavucontrol's level bars)
├── src/analysis.rs per-block peak/RMS, clip/silence/dead-channel detection,
│                   waveform decimation (pure, unit-tested)
└── src/ui.rs       device list, meters, waveform; redraws from a
                    frame-synced tick callback
```

Analysis runs per audio block, outside the drawing path. The UI only reads the
latest summary and a decimated waveform; it never redraws per sample. Pulse
callbacks and the UI share one thread, so state is `Rc<RefCell<..>>` with no
locking (no ring buffer needed).

## Notes and limitations

- Muted inputs are not monitored: the audio server silences them before any
  client can see the signal, so they always show red.

## Prerequisites

At runtime the app needs a running PipeWire (with `pipewire-pulse`) or
PulseAudio server, GTK ≥ 4.10 and libadwaita ≥ 1.4.

Build packages:

| Distro | Packages |
| ------ | -------- |
| Arch   | `sudo pacman -S --needed base-devel rust gtk4 libadwaita libpulse pkgconf` |
| Fedora | `sudo dnf install gcc rust cargo gtk4-devel libadwaita-devel pulseaudio-libs-devel pkgconf-pkg-config` |

Optional tools, called by some features (the app runs without them; only that
feature fails):

| Tool           | Used for                                              | Arch          | Fedora              |
| -------------- | ----------------------------------------------------- | ------------- | ------------------- |
| `wireplumber`  | System-wide disable (PipeWire only)                   | `wireplumber` | `wireplumber`       |
| `systemctl`    | Restarting WirePlumber after a disable/enable         | `systemd`     | `systemd`           |
| `pactl`        | Restoring defaults and streams after that restart     | `libpulse`    | `pulseaudio-utils`  |
| `bluetoothctl` | Reconnecting Bluetooth devices after that restart     | `bluez-utils` | `bluez`             |

## Build

    cargo run --release
    cargo test

## AppImage

To build and run it:

1. Install the build packages from [Prerequisites](#prerequisites).
2. Install `appimagetool` (Arch: AUR `appimagetool-bin`; Fedora: download from
   [AppImage/appimagetool](https://github.com/AppImage/appimagetool/releases)
   and put it on your `PATH`). `glib-compile-schemas` (from `glib2`) is also
   needed; GTK normally pulls it in.
3. From the repo root:

       packaging/build-appimage.sh

4. Run the result:

       ./Sound-Checker-x86_64.AppImage

The script does a `cargo build --release --locked`, assembles
`build-appimage/AppDir` (binary, the GTK4/libadwaita libraries copied from the
build host, GDK-Pixbuf/GIO modules, GSettings schemas, icon themes, desktop
entry and icon) and writes `Sound-Checker-x86_64.AppImage` to the repo root.
Build on the oldest distro you want to support, since the bundle uses the host's
libraries but relies on the target's glibc and GPU drivers.

Running the AppImage needs FUSE (`fuse3`; older AppImage runtimes want `fuse2`
on Arch / `fuse-libs` on Fedora), or run it with `--appimage-extract-and-run`.
Launching it a second time raises the running window instead of opening another.

## Flatpak

    flatpak-cargo-generator.py Cargo.lock -o packaging/cargo-sources.json
    flatpak-builder --user --install --force-clean build-dir packaging/dev.soundchecker.SoundChecker.yml

Needs `flatpak-builder`, `org.gnome.Platform`/`Sdk` and
`org.freedesktop.Sdk.Extension.rust-stable`. Uses `--socket=pulseaudio`.

## Testing matrix

The audio stack and hardware matter more than the desktop environment.

- PipeWire (`pipewire-pulse`) and real PulseAudio
- Built-in mic, USB headset/mic, webcam mic, Bluetooth headset (A2DP and HFP)
- Hot-plug while capturing (unplug the selected device)
- Muted/zero-volume source, mono and stereo sources, 44.1 vs 48 kHz
- Run as Flatpak and as a native build on both Arch and Fedora
