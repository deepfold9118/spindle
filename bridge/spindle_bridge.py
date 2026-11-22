"""Spindle bridge.

Spindle installs this file into each profile's ``scripts/startup`` folder, so it runs
in every Blender that Spindle launches. It writes a small heartbeat file describing
this running Blender (version, binary, open file, enabled MCP add-ons and their
ports) so that ``spindle-mcp`` can tell agents which Blenders are actually running,
instead of guessing from what is installed.

It does nothing when Blender was not started by Spindle (no SPINDLE_INSTANCES_DIR)
or runs in background mode.
"""

import atexit
import json
import os
import time

import bpy

_DIR = os.environ.get("SPINDLE_INSTANCES_DIR")
_INTERVAL = 5.0
_STARTED = time.time()


def _path():
    return os.path.join(_DIR, f"{os.getpid()}.json")


def _mcp_endpoints():
    endpoints = []
    for addon in bpy.context.preferences.addons:
        if "mcp" not in addon.module.lower():
            continue
        prefs = getattr(addon, "preferences", None)
        port = getattr(prefs, "port", None)
        endpoints.append({
            "addon": addon.module,
            "host": getattr(prefs, "host", None),
            "port": int(port) if isinstance(port, int) else None,
        })
    return endpoints


def _heartbeat():
    try:
        data = {
            "pid": os.getpid(),
            "blender_version": bpy.app.version_string,
            "binary": bpy.app.binary_path,
            "profile": os.environ.get("SPINDLE_PROFILE"),
            "file": bpy.data.filepath,
            "started": _STARTED,
            "heartbeat": time.time(),
            "mcp": _mcp_endpoints(),
        }
        tmp = _path() + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f)
        os.replace(tmp, _path())
    except Exception as ex:  # never break Blender over a heartbeat
        print("spindle_bridge:", ex)
    return _INTERVAL


def _remove():
    try:
        os.remove(_path())
    except OSError:
        pass


def register():
    if not _DIR or bpy.app.background:
        return
    os.makedirs(_DIR, exist_ok=True)
    if not bpy.app.timers.is_registered(_heartbeat):
        bpy.app.timers.register(_heartbeat, first_interval=0.5, persistent=True)
    atexit.register(_remove)


def unregister():
    if bpy.app.timers.is_registered(_heartbeat):
        bpy.app.timers.unregister(_heartbeat)
    _remove()
