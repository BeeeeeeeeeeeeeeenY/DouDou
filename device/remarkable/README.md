# Riddle device launcher

`riddle-launcher.qmd` adds a direct **Riddle** item to the stock reMarkable
sidebar on Paper Pro OS 3.27.

It uses Xovi's `qt-resource-rebuilder` and `qt-command-executor` extensions,
but intentionally keeps the full AppLoad extension disabled. This avoids the
full-screen AppLoad overlay that intercepted finger input on this device.

The item starts the existing AppLoad-packaged application through:

`/home/root/xovi/exthome/appload/riddle/appload-launch.sh`

The Riddle package, API configuration, font, and takeover recovery behavior
remain unchanged.

`xovi-tripletap-config` is used with the upstream `xovi-tripletap` service.
Three quick power-button presses stop a stale Riddle takeover, if present, and
start the launcher without making Xovi an unsafe automatic boot dependency.
