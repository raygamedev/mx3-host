#!/usr/bin/env bash
set -euo pipefail

BINARY="target/release/mx3-host"
INSTALL_PATH="/usr/local/bin/mx3-host"
UDEV_RULE="/etc/udev/rules.d/99-logitech-hidraw.rules"

cd "$(dirname "$0")"

# Resolve which group to use for hidraw access.
# 'plugdev' is a Debian/Ubuntu convention; Arch/CachyOS don't ship it.
# We create it if absent, falling back to 'input' which exists everywhere.
if getent group plugdev > /dev/null 2>&1; then
    HID_GROUP="plugdev"
elif getent group input > /dev/null 2>&1; then
    HID_GROUP="input"
    echo "Note: 'plugdev' group not found; using 'input' instead."
else
    echo "Creating 'plugdev' group…"
    sudo groupadd plugdev
    HID_GROUP="plugdev"
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Binary not found. Building release…"
    cargo build --release
fi

echo "Installing $BINARY → $INSTALL_PATH"
sudo install -m 755 "$BINARY" "$INSTALL_PATH"

echo "Writing udev rule → $UDEV_RULE (group: $HID_GROUP)"
sudo tee "$UDEV_RULE" > /dev/null <<EOF
# Logitech HID devices: allow members of '${HID_GROUP}' group to access hidraw nodes.
# ENV{HID_ID} matches USB receivers (bus 0003) AND direct Bluetooth (bus 0005).
SUBSYSTEM=="hidraw", ENV{HID_ID}=="*:0000046D:*", MODE="0660", GROUP="${HID_GROUP}"
EOF

echo "Reloading udev rules…"
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=hidraw

echo ""
echo "Done. Group used: ${HID_GROUP}"
echo ""
echo "Add yourself to the group (if not already) and log out/in:"
echo "  sudo usermod -aG ${HID_GROUP} \$USER"
echo ""
echo "You may need to re-plug the USB receiver for the new permissions to apply."
