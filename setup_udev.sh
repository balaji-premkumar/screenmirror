#!/bin/bash
# ScreenMirror AOA udev rules installer.
#
# Grants the *active local session* access to Android devices via the uaccess
# tag instead of making the device nodes world-readable/writable (MODE=0666),
# which would let any local process talk to the phone.
set -e

RULE_PATH=/etc/udev/rules.d/51-android-aoa.rules

# Matched on the AOA accessory product range only (18d1:2d00-2d0f). An
# unqualified vendor match would grant the local session access to every
# device from that vendor, which is far more than this app needs.
sudo tee "$RULE_PATH" > /dev/null <<'EOF'
# ScreenMirror — Android Open Accessory access for the active session
SUBSYSTEM=="usb", ATTR{idVendor}=="18d1", ATTR{idProduct}=="2d0?", TAG+="uaccess"
EOF
sudo chmod 0644 "$RULE_PATH"

sudo udevadm control --reload-rules
sudo udevadm trigger

echo "Udev rules for Android Accessory Mode installed and activated."
echo "Please unplug and re-plug your Android device."
