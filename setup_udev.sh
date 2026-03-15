#!/bin/bash
# streamApp AOA udev rules installer
# This grants your local user permission to read/write to Android devices in Accessory mode ( VID 18d1 )
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="18d1", MODE="0666", GROUP="plugdev"' | sudo tee /etc/udev/rules.d/51-android-aoa.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
echo "Udev rules for Android Accessory Mode installed and activated."
echo "Please unplug and re-plug your Android device."
