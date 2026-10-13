# TODO

## Done (implemented, not yet verified on hardware)

- [x] Device discovery, hot-plug, mute/volume, device details
- [x] Live capture, per-channel meters, waveform, diagnostics, noise floor
- [x] Record & play back
- [x] Bluetooth A2DP -> headset profile hint and switch
- [x] Flatpak manifest and desktop file
- [x] Device list split into Unmuted / Muted
- [x] Activity colouring: green for active, dimmed for idle, red for muted
- [x] Output devices: list, mute, volume reset, set as default, Muted category, green while playing
- [x] Recording (inputs) and Sound (outputs) tabs replace the Inputs / Outputs headings
- [x] Mute button on every device
- [x] Per-output system-wide disable (same WirePlumber rule; hidden for Bluetooth)
- [x] Set as default for inputs and outputs
- [x] Rename devices (app-only, `names.json`)
- [x] Meters, waveform and recording for outputs (monitor sources); every input and output is monitored automatically, selecting only changes the view
- [x] Confirmation dialogs: system-wide disable, Bluetooth headset/music mode
- [x] Restart side effects: after the WirePlumber restart, restore default devices, move streams back, restore card profiles (reconnecting a Bluetooth headset that lost its music link) and log to `restart.log`
- [x] Muted devices clear their meters and waveform instead of freezing the last picture


## System-wide mic disable

Goal: disabling a mic hides it from every app using PipeWire (Discord, browsers, OBS),
not just from this app's list. Uses WirePlumber `node.disabled` rules, applied per input
node (e.g. Scarlett Mic2 only), so the device's other inputs and outputs keep working.
Not a hardware-level guarantee: direct ALSA access (`arecord -D hw:N`) still works.

Design:

- State: `~/.config/sound-checker/disabled.json` (`node_name`, `device_name`, `description`,
  `bus`, `disabled_at`) is the source of truth; disabled nodes vanish from the server, so
  the app cannot read their names from it
- Config: regenerate `~/.config/wireplumber/wireplumber.conf.d/90-sound-checker.conf` from
  the state on every change (`monitor.alsa.rules` and `monitor.bluez.rules`, matching
  `node.name`, `update-props = { node.disabled = true }`). WirePlumber 0.5 appends these to
  the user's own rules
- Apply: `systemctl --user restart wireplumber` after a confirmation dialog (briefly
  interrupts all audio); stop capture first if the target is the selected source
- Verify: after the restart, wait for the source list refresh. Disabled = node absent and
  card present; enabled = node back. Show Applying… / Active / Failed (timeout about 5 s)
- UI: "Disable system-wide…" button in the details pane; a "Disabled" category at the
  bottom of the list, built from the state, showing "Connected — blocked" (card present)
  or "Not connected", with an Enable button; one-line ALSA caveat
- Hide the feature on plain PulseAudio (server name without "PipeWire") or when there is no
  `wireplumber` unit; without a systemd user session, write the config and say it applies
  after logging in again
- Flag saved entries whose card is present but whose node name no longer appears
  (e.g. after a profile change): "rule no longer matches"

Steps:

- [ ] Manual spike: hand-write a rule for the webcam mic, restart WirePlumber, confirm the
      node disappears from `pactl list sources` and from apps; confirm a restart is required
- [x] `src/wireplumber.rs`: state load/save, config generation (escape `"` and `\`),
      apply/restart; unit tests for config generation. Add `serde` + `serde_json`
- [x] `pulse.rs`: expose `node.name` and `device.name` on `SourceEntry`, `device.name` on `CardEntry`
- [x] `ui.rs`: Disabled category, button and confirmation dialog, status display
- [x] README feature row
- [ ] Bluetooth headsets: "Switch to music mode (no mic)" replaces the node rule (disabling a BT mic node cut playback; cause unconfirmed)
- [ ] Test cases: hot-plug while disabled, BT headset, disabling the default source

Open decisions:

- [ ] Whole-device disable (all inputs of a card at once): proposed per-node only for now

## Known issues

- [ ] The WI-C100 headphones sometimes lose their A2DP link on their own (`Failure in Bluetooth audio transport`) and fall back to the headset profile. Not caused by the app as far as seen; check whether the continuous capture on every device or the restart makes it worse
- [ ] The restart-and-restore for system-wide disable is unverified; read `restart.log` after a real disable/enable
- [ ] Applying a rule always restarts WirePlumber (brief audio interruption); look for a way to avoid it, or add "Save, apply later"

## Stretch

- [ ] Rename system-wide (WirePlumber `node.description` rule; needs the same restart)
- [ ] Test tone for outputs (the monitor shows what plays, but nothing generates a signal)

- [ ] Live listen: mic -> output with mute control and a "use headphones" warning
      (module-loopback, or own playback stream if latency control is needed)
- [ ] Spectrum view (FFT)


## Later

- [ ] Check that continuous capture on every device doesn't wake suspended hardware or change Bluetooth profiles (`DONT_INHIBIT_AUTO_SUSPEND` is set)
- [ ] Tune the activity threshold (currently about -45 dBFS) on quiet and noisy mics
- [ ] Show muted devices as green when they receive input. Unconfirmed whether this is
      possible: the Pulse API only delivers post-mute audio. A test with a loopback source
      was inconclusive; may need PipeWire-native monitoring (not the Pulse API)

- [ ] Tabs: check the header switcher in a narrow window (only the wide layout was considered; may need an adw::ViewSwitcherBar)
- [ ] Verify against the testing matrix in README.md (hot-plug, BT HFP, 44.1 kHz, real PulseAudio)
- [ ] Build and test the Flatpak (generate `packaging/cargo-sources.json`; confirm GNOME runtime version)
- [ ] Test on Fedora
- [ ] Check the app ID (`dev.soundchecker.SoundChecker`) and add an icon / metainfo
- [ ] AUR PKGBUILD