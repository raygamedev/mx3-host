# mx3-host

A minimal Rust CLI tool that switches the active Bluetooth/Unifying host on a **Logitech MX Master 3 / 3S** mouse using raw HID++ 2.0 — no Solaar, no daemon, no D-Bus.

```
mx3-host 2   # switch to host slot 2 — instant
```

## Why

Solaar works, but spawning a Python process with full device enumeration takes ~500 ms. This binary does the same thing in ~20 ms: one `getFeature` round-trip to resolve the ChangeHost feature index, one fire-and-forget `setCurrentHost` write.

## How it works

1. Scans `/sys/class/hidraw/*/device/uevent` for Logitech vendor `046d`
2. Reads the bus type from `HID_ID` to pick the correct HID++ format:
   - **Bluetooth** (bus `0x0005`): long report `0x11`, 20 bytes, device index `0xFF`
   - **USB Unifying/Bolt receiver** (bus `0x0003`): short report `0x10`, 7 bytes, device index `0x01`
3. Sends `getFeature(0x1814)` to the root feature (index `0x00`) to discover the runtime index of the **ChangeHost** feature
4. Sends `setCurrentHost(host − 1)` as a fire-and-forget write — the mouse disconnects immediately and reconnects to the target host

Protocol sourced from [Solaar](https://github.com/pwr-Solaar/Solaar) (`settings_templates.py` → `ChangeHost.rw_options`, `write_fnid=0x10`, `no_reply=True`).

No external crate dependencies — stdlib + a single `extern "C"` call to `poll(2)`.

## Requirements

- Linux with `hidraw` support (standard on all modern kernels)
- Logitech MX Master 3 or 3S (tested on 3S, Bluetooth)
- Read/write access to `/dev/hidrawX` (see [Permissions](#permissions))
- Rust toolchain (for building)

## Installation

```bash
git clone https://github.com/rayman/mx3-host
cd mx3-host
cargo build --release
sudo bash install.sh
```

`install.sh` does three things:

1. Copies the binary to `/usr/local/bin/mx3-host`
2. Writes `/etc/udev/rules.d/99-logitech-hidraw.rules` using `ENV{HID_ID}` to match both USB and Bluetooth Logitech devices
3. Reloads udev rules

## Permissions

The udev rule grants group `input` (Arch/CachyOS) or `plugdev` (Debian/Ubuntu) read/write access to Logitech hidraw nodes.

Add yourself to the group once and log out:

```bash
# Arch / CachyOS / Manjaro
sudo usermod -aG input $USER

# Debian / Ubuntu / Fedora
sudo usermod -aG plugdev $USER
```

Verify after logging back in:

```bash
groups | grep -E 'input|plugdev'
```

## Usage

```
mx3-host <1|2|3>
```

| Argument | Effect |
|----------|--------|
| `1`      | Switch to host slot 1 |
| `2`      | Switch to host slot 2 |
| `3`      | Switch to host slot 3 |

Exits `0` on success, `1` with a message on any error.

## Hyprland keybindings

```ini
# ~/.config/hypr/hyprland.conf

bind = SUPER, F1, exec, mx3-host 1
bind = SUPER, F2, exec, mx3-host 2
bind = SUPER, F3, exec, mx3-host 3
```

Reload with `hyprctl reload`.

## Build details

```toml
[profile.release]
opt-level = "z"    # size-optimised
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Stripped binary is ~300 KB dynamically linked. No heap allocation beyond argv and the one `Vec` for the HID++ report buffer.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `permission denied opening /dev/hidrawX` | Run `install.sh`, add yourself to the group, log out/in, re-plug the receiver |
| `no Logitech HID device found` | Mouse not connected; check `ls /sys/class/hidraw/*/device/uevent` |
| `timeout waiting for getFeature response` | Mouse is asleep — move it, then retry |
| `device does not support ChangeHost (0x1814)` | Wrong device, or firmware too old |
| Switch accepted but mouse stays on current host | Target host slot has no paired device — the mouse falls back automatically |

## HID++ 2.0 wire format

Short report (USB, 7 bytes):

```
[0x10] [dev_idx] [feat_idx] [(fn<<4)|sw_id] [p0] [p1] [p2]
```

Long report (Bluetooth, 20 bytes):

```
[0x11] [dev_idx] [feat_idx] [(fn<<4)|sw_id] [p0] [p1] ... [p15]
```

Packets used:

| Step | feat_idx | fn | Payload | Note |
|------|----------|----|---------|------|
| getFeature | `0x00` | `0` | `0x18 0x14` | resolves ChangeHost index |
| setCurrentHost | `<runtime>` | `1` | `host−1` | no reply expected |

## License

MIT
