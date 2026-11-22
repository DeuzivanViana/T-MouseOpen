# TMouseOpen

Linux configurator for the **Titorion YDX-GM009** mouse (2.4G receiver `04f3:026f`, PAW3311 sensor).
Shows polling rate, active DPI stage, battery, charge state and sensor; changes the polling rate and the 7 DPI stages.

![TMouseOpen main window](docs/screenshot.png)

| Settings applied, log open | Confirming an untested polling rate | No receiver connected |
|---|---|---|
| ![applied](docs/screenshot-applied.png) | ![confirm](docs/screenshot-confirm.png) | ![no receiver](docs/screenshot-no-receiver.png) |

*Screenshots taken with `--demo` (simulated mouse) on Xvfb; no hardware was touched.*

## Interface
- **Stat tiles** at the top: polling rate (with the receiver's maximum), active DPI and stage, battery level and charge state, sensor and firmware.
- **Polling rate**: segmented control (125 / 250 / 500 / 1000 Hz). Values marked `*` have not been tested on the mouse yet and ask for confirmation before being sent.
- **DPI stages**: one row per stage with an on/off switch, a numbered badge (click it to make that stage active), the DPI picker and a bar showing the value relative to the 12000 DPI maximum. Nothing is sent until you press **Apply DPI**.
- **Device** card: receiver, sensor, firmware and maximum rate; when the mouse is not connected it shows the last applied polling rate.
- **Status bar**: connection pill in the header, result of the last command in the footer and a collapsible log with every frame sent and acknowledged.

## Prebuilt binary (Debian 11, 12, 13)
Download `tmouseopen-<version>-linux-x86_64.tar.gz`, then:
```bash
tar xzf tmouseopen-*-linux-x86_64.tar.gz
cd tmouseopen-*-linux-x86_64
./tmouseopen --demo      # try the UI without the mouse
```
The binary needs glibc 2.30 or newer (Debian 11 ships 2.31) and no extra packages beyond a normal desktop.
See **Permissions** below to access the receiver.

## Build (Debian/Ubuntu)

```bash
sudo apt install build-essential curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust >= 1.95 (the apt rustc is too old)
cargo build --release
# binary: target/release/tmouseopen
```

To build a binary that also runs on older distributions (glibc 2.31, Debian 11), use [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild):
```bash
pip install ziglang && cargo install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31
# binary: target/x86_64-unknown-linux-gnu/release/tmouseopen
```

Rust crates (fetched by cargo): `eframe`/`egui` 0.37 (UI), `libc` (hidraw ioctls), `serde` + `serde_json` (saved settings).
No `hidapi`, `libudev-dev` or graphics `-dev` packages are needed: the binary links only against libc and loads the
graphics libraries at runtime. Any desktop already has them: `libgl1`/`libegl1`, `libxkbcommon0` and
`libwayland-client0` (Wayland) or `libx11-6`, `libx11-xcb1`, `libxcursor1`, `libxi6`, `libxkbcommon-x11-0` (X11).

## Permissions

Pick one:

**A. udev rule (recommended, one time)**
```bash
sudo cp packaging/99-tmouseopen.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```
Then unplug and replug the receiver and run `tmouseopen` as your user.

**B. `sudo -E tmouseopen` (no system changes at all)**
As root, the program only finds and opens the receiver, then permanently drops to your user
(`SUDO_UID`/`SUDO_GID`) before any window or file is touched:
- all user/group IDs are reset and supplementary groups cleared; `NO_NEW_PRIVS` is set, so root cannot be regained;
- `HOME` and the XDG directories are pointed at your own home (from the password database), so nothing is written under `/root`;
- the only file it ever writes is your own `~/.config/tmouseopen/state.json`, owned by you, mode 0600.

`-E` keeps `DISPLAY`/`WAYLAND_DISPLAY` so the window can open. Running as root without `sudo` is refused.
Limitation: since root is gone after startup, a receiver unplugged while running can only be reopened by restarting
with `sudo -E` (or with the udev rule installed).

## Usage

```bash
./target/release/tmouseopen           # UI
./target/release/tmouseopen --demo    # UI with a simulated mouse
./target/release/tmouseopen --list    # list the receiver's hidraw nodes
cargo test                            # protocol tests (including the captured frames)
```

## DPI and the mouse button
The mouse DPI button **only cycles through enabled stages**. Enable every stage you want to reach with the button.
Each stage's DPI is picked from a list (100–12000 in steps of 100) and is sent only when you press **Apply DPI**.

## Reading the current settings
The mouse has no "read settings" command (the official software doesn't either). The status report arrives every ~5 s
while the mouse is awake. The mouse does not report the DPI of each stage, so the app stores the last applied values in
`~/.config/tmouseopen/state.json` (only after the mouse acknowledges them). On first run, settings from the previous
version (`~/.config/ydx-mouse/`) are picked up automatically.

## Security
- Only the polling (0x04) and DPI (0x02) commands can be built, and writes are restricted to report 0x06. The firmware reports (0xBC/0xBF) do not exist in the code.
- Only `/dev/hidrawN` is opened, and after opening the descriptor itself is checked: character device, VID:PID `04f3:026f` and a 252-byte report descriptor (control interface).
- Reads use fixed-size buffers and timeouts; invalid commands are rejected before they reach the device.
- `state.json` is treated as untrusted (size-limited, fully validated) and written atomically with mode 0600, without following symlinks.
- Under `sudo -E`, root is used only to open the receiver and is dropped immediately (see Permissions).
- All I/O runs on a separate thread; the UI never blocks waiting for the mouse.

## Layout
| file | purpose |
|---|---|
| `src/protocol.rs` | framing, checksum, commands, status (pure, with tests) |
| `src/device.rs` | sysfs discovery, verified open, ioctls; demo mode |
| `src/worker.rs` | device thread: sends commands, waits for acks, reads status, reconnects |
| `src/state.rs` | saved settings |
| `src/privs.rs` | `sudo -E` support: open the receiver, then drop root |
| `src/ui.rs` | egui UI (dark monochrome theme, stat tiles, segmented control, DPI stage list) |
| `PROTOCOL.md` | what is known about the protocol, with confidence levels |
