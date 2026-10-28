#!/usr/bin/env python3
"""Prints the UDID of an iPhone simulator on the newest iOS runtime >= 17.

Reads `xcrun simctl list devices available -j` on stdin. Exits 1 with a message when
no such simulator exists, so CI fails loudly instead of testing on an old runtime.
"""
import json
import re
import sys

MIN_IOS = (17, 0)

best = None
for runtime, devices in json.load(sys.stdin)["devices"].items():
    # e.g. "com.apple.CoreSimulator.SimRuntime.iOS-18-2"
    match = re.search(r"\.iOS-(\d+)-(\d+)$", runtime)
    if not match:
        continue
    version = (int(match[1]), int(match[2]))
    if version < MIN_IOS:
        continue
    for device in devices:
        if device["name"].startswith("iPhone") and device.get("isAvailable", True):
            if best is None or version > best[0]:
                best = (version, device["udid"])
            break

if best is None:
    sys.exit("no available iPhone simulator with iOS >= %d.%d" % MIN_IOS)
print(best[1])
