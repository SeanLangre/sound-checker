# TODO

## Done (implemented, not yet verified on hardware)

- [x] Device discovery, hot-plug, mute/volume, device details
- [x] Live capture, per-channel meters, waveform, diagnostics, noise floor
- [x] Record & play back
- [x] Bluetooth A2DP -> headset profile hint and switch
- [x] Flatpak manifest and desktop file
- [x] Device list split into Unmuted / Muted
- [x] Activity colouring: green (fading over 3 s) for active, dimmed for idle, red for muted

## Next

- [ ] Verify the activity colouring on screen with a real mic (green, fade, red, theme contrast)
- [ ] Tune the activity threshold (currently about -45 dBFS) on quiet and noisy mics
- [ ] Show muted devices as green when they receive input. Unconfirmed whether this is
      possible: the Pulse API only delivers post-mute audio. A test with a loopback source
      was inconclusive; may need PipeWire-native monitoring (not the Pulse API)
- [ ] Keep the 3 s fade running across list rebuilds (update rows in place instead of recreating them)

- [ ] Verify against the testing matrix in README.md (hot-plug, BT HFP, 44.1 kHz, real PulseAudio)
- [ ] Build and test the Flatpak (generate `packaging/cargo-sources.json`; confirm GNOME runtime version)
- [ ] Test on Fedora
- [ ] Check the app ID (`dev.soundchecker.SoundChecker`) and add an icon / metainfo
- [ ] AUR PKGBUILD

## Stretch

- [ ] Live listen: mic -> output with mute control and a "use headphones" warning
      (module-loopback, or own playback stream if latency control is needed)
- [ ] Spectrum view (FFT)
