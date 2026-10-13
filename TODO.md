# TODO

## Done (implemented, not yet verified on hardware)

- [x] Device discovery, hot-plug, mute/volume, device details
- [x] Live capture, per-channel meters, waveform, diagnostics, noise floor
- [x] Record & play back
- [x] Bluetooth A2DP -> headset profile hint and switch
- [x] Flatpak manifest and desktop file

## Next

- [ ] Verify against the testing matrix in README.md (hot-plug, BT HFP, 44.1 kHz, real PulseAudio)
- [ ] Build and test the Flatpak (generate `packaging/cargo-sources.json`; confirm GNOME runtime version)
- [ ] Test on Fedora
- [ ] Check the app ID (`dev.soundchecker.SoundChecker`) and add an icon / metainfo
- [ ] AUR PKGBUILD

## Stretch

- [ ] Live listen: mic -> output with mute control and a "use headphones" warning
      (module-loopback, or own playback stream if latency control is needed)
- [ ] Spectrum view (FFT)
