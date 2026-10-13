# Linux Sound Checker

A small native Linux desktop app for testing microphones and other audio inputs:
detect connected devices, see their levels live, spot common problems, and hear
what the mic actually captures.

Targets: Arch Linux and Fedora, desktop-environment agnostic, PipeWire or PulseAudio.
Open tasks are in [TODO.md](TODO.md).

## Features

| Feature                 | Behaviour                                                                 |
| ----------------------- | ------------------------------------------------------------------------- |
| Device discovery        | List input sources; hide monitor sources (`monitor_of_sink` is set)       |
| Hot-plug                | Refresh on Pulse subscribe events (source/card add, remove, change)       |
| Mute & volume           | Show server-side mute and volume; one-click unmute / reset to 100%        |
| Per-channel meters      | Peak and RMS in dBFS for each channel, with peak hold                     |
| Waveform                | Scrolling waveform of the selected input                                  |
| Diagnostics             | Silence, clipping, dead/imbalanced channel, disconnect, capture errors    |
| Noise floor             | Quietest RMS window over the last 10 s                                    |
| Record & play back      | Record 5 s, play it through the default output                            |
| Bluetooth profile hint  | Detect a BT card in A2DP with an available HFP/HSP profile; offer to switch |
| Device details          | Sample rate, format, channel map, driver, card/bus info                   |
| Switching               | Change input without restarting                                           |

## UI sketch

```
┌ Linux Sound Checker ─────────────────────────────────────────┐
│ Input devices                 │ Live analysis                │
│ ● Built-in Microphone   [sel] │  L ████████████░░░░  -6.2 dB │
│   Internal · 48 kHz · 2ch     │  R ███████████░░░░░  -7.0 dB │
│   ⚠ Muted  [Unmute]  vol 100% │  RMS -21.4 dBFS  Noise -62   │
│ ○ USB Headset Microphone      │  ~~~~~ waveform ~~~~~        │
│ ○ BT Headset (no mic: A2DP)   │  [● Record 5s] [▶ Play back] │
│   [Switch to headset mode]    │  Status: OK / Clipping / ... │
└───────────────────────────────┴──────────────────────────────┘
```

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
├── src/analysis.rs per-block peak/RMS, clip/silence/dead-channel detection,
│                   waveform decimation (pure, unit-tested)
└── src/ui.rs       device list, meters, waveform; redraws from a
                    frame-synced tick callback
```

Analysis runs per audio block, outside the drawing path. The UI only reads the
latest summary and a decimated waveform; it never redraws per sample. Pulse
callbacks and the UI share one thread, so state is `Rc<RefCell<..>>` with no
locking (no ring buffer needed).

## Build

    cargo run --release
    cargo test

Dependencies — Arch: `rust gtk4 libadwaita libpulse pkgconf`;
Fedora: `rust cargo gtk4-devel libadwaita-devel pulseaudio-libs-devel pkgconf`.

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
