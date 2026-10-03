#!/usr/bin/env python3
"""A private headless GNOME Shell for window tests.

    scripts/gnome_session.py [--x11] [--shot SECONDS:FILE]... -- COMMAND...

Starts its own D-Bus session, PipeWire, WirePlumber and gnome-shell
--headless with one 1280x800 virtual monitor, runs COMMAND against it
(Wayland, or Xwayland with --x11), optionally screenshots the monitor through
mutter's ScreenCast API while COMMAND runs, then tears everything down.
Exits with COMMAND's status, or 125 when the shell keeps freezing as it
starts. XWINDOW_GNOME_LOGS=DIR keeps the session's logs there; safe to set.

Needs gnome-shell, dbus-daemon, pipewire, wireplumber, GStreamer's
PipeWire source and python3-gi. Runs as an ordinary user, beside any
session the user already has.
"""
import glob, os, re, shutil, signal, subprocess, sys, time

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

run = f"/tmp/xw-run-{os.getuid()}"
BASE = dict(os.environ)
children = []


def spawn(args, env, log):
    p = subprocess.Popen(args, env=env, stdout=open(f"{run}/{log}", "w"),
                         stderr=subprocess.STDOUT, start_new_session=True)
    children.append(p)
    return p


def teardown():
    for p in reversed(children):
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    time.sleep(1)
    # Whatever the session's bus started, by the runtime dir it inherited.
    mark = f"XDG_RUNTIME_DIR={run}".encode()
    for proc in glob.glob("/proc/[0-9]*"):
        try:
            if int(proc[6:]) != os.getpid() and mark in open(f"{proc}/environ", "rb").read():
                os.kill(int(proc[6:]), signal.SIGKILL)
        except (OSError, ValueError):
            pass
    # A killed Xwayland leaves its lock and socket, and the next cannot start.
    for lock in glob.glob("/tmp/.X*-lock"):
        try:
            if os.stat(lock).st_uid != os.getuid():
                continue
            pid = int(open(lock).read().strip())
            if os.path.exists(f"/proc/{pid}"):
                continue
            number = lock[len("/tmp/.X"):-len("-lock")]
            os.remove(lock)
            if os.path.exists(f"/tmp/.X11-unix/X{number}"):
                os.remove(f"/tmp/.X11-unix/X{number}")
        except (OSError, ValueError):
            pass
    logs = os.environ.get("XWINDOW_GNOME_LOGS")
    if logs:
        os.makedirs(logs, exist_ok=True)
        for log in glob.glob(f"{run}/*.log"):
            shutil.copy(log, logs)
    # The document portal and gvfs mount FUSE filesystems in the runtime dir.
    for mount in ("doc", "gvfs"):
        subprocess.run(["fusermount3", "-u", "-z", f"{run}/{mount}"], stderr=subprocess.DEVNULL)
    shutil.rmtree(run, ignore_errors=True)


def call(bus, obj, iface, method, args, reply):
    return bus.call_sync("org.gnome.Mutter.ScreenCast", obj, iface, method, args,
                         GLib.VariantType(reply), Gio.DBusCallFlags.NONE, 5000, None).unpack()


def record(bus, folder):
    """Records the virtual monitor through ScreenCast and PipeWire: a PNG per
    frame into `folder`, the last resent twice a second while nothing changes.
    Only the latest few frames are kept, however fast the monitor changes."""
    (session,) = call(bus, "/org/gnome/Mutter/ScreenCast", "org.gnome.Mutter.ScreenCast",
                      "CreateSession", GLib.Variant("(a{sv})", ({},)), "(o)")
    (stream,) = call(bus, session, "org.gnome.Mutter.ScreenCast.Session", "RecordMonitor",
                     GLib.Variant("(sa{sv})", ("", {"cursor-mode": GLib.Variant("u", 0)})), "(o)")
    node = []
    loop = GLib.MainLoop()

    def added(conn, sender, obj, iface, sig, params):
        node.append(params.unpack()[0])
        loop.quit()

    sub = bus.signal_subscribe(None, "org.gnome.Mutter.ScreenCast.Stream", "PipeWireStreamAdded",
                               stream, None, Gio.DBusSignalFlags.NONE, added)
    call(bus, session, "org.gnome.Mutter.ScreenCast.Session", "Start", None, "()")
    GLib.timeout_add(5000, loop.quit)
    loop.run()
    bus.signal_unsubscribe(sub)
    if not node:
        raise RuntimeError("no PipeWire stream")
    os.makedirs(folder, exist_ok=True)
    return subprocess.Popen(["gst-launch-1.0", "-q", "pipewiresrc", f"target-object={node[0]}",
                             "keepalive-time=500", "!", "videoconvert", "!", "pngenc",
                             "snapshot=false", "!", "multifilesink", "max-files=8",
                             f"location={folder}/frame-%05d.png"],
                            stdout=subprocess.DEVNULL, start_new_session=True)


def frame_now(folder):
    """The newest recorded frame that has finished being written."""
    frames = []
    for name in glob.glob(f"{folder}/frame-*.png"):
        try:
            frames.append((os.path.getmtime(name), name))
        except OSError:
            pass  # dropped as newer frames came
    done = [f for f in frames if f[0] <= time.time() - 0.05]
    return max(done)[1] if done else None


def start(x11):
    """A fresh session: its bus, PipeWire and the shell. Returns its env and X display."""
    teardown()
    children.clear()
    os.makedirs(run, mode=0o700)
    env = {k: v for k, v in BASE.items()
           if k not in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XAUTHORITY")}
    env["XDG_RUNTIME_DIR"] = run
    daemon = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address"],
                              env=env, stdout=subprocess.PIPE, stderr=open(f"{run}/dbus.log", "w"),
                              start_new_session=True, text=True)
    children.append(daemon)
    env["DBUS_SESSION_BUS_ADDRESS"] = daemon.stdout.readline().strip()
    os.environ.clear()
    os.environ.update(env)
    spawn(["pipewire"], env, "pipewire.log")
    time.sleep(0.5)
    spawn(["wireplumber"], env, "wireplumber.log")
    spawn(["gnome-shell", "--headless", "--wayland", "--virtual-monitor", "1280x800",
           "--wayland-display", "xw-test"] + ([] if x11 else ["--no-x11"]), env, "shell.log")
    display = None
    for _ in range(300):
        m = re.search(r"Using public X11 display (:\d+)", open(f"{run}/shell.log").read())
        display = m.group(1) if m else None
        if os.path.exists(f"{run}/xw-test") and (display or not x11):
            break
        time.sleep(0.1)
    return env, display


def session_bus():
    """This session's bus; Gio's shared one would outlive a restart."""
    return Gio.DBusConnection.new_for_address_sync(
        os.environ["DBUS_SESSION_BUS_ADDRESS"],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
        | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)


def responsive():
    """Whether the shell has started and its main loop answers."""
    bus = session_bus()
    for _ in range(300):
        names = bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus",
                              "org.freedesktop.DBus", "ListNames", None,
                              GLib.VariantType("(as)"), Gio.DBusCallFlags.NONE, 2000,
                              None).unpack()[0]
        if "org.gnome.Shell" in names and "org.gnome.Mutter.ScreenCast" in names:
            break
        time.sleep(0.1)
    time.sleep(3)
    # Xwayland's start can freeze the headless shell; a frozen one answers nothing.
    try:
        bus.call_sync("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
                      "org.gnome.Mutter.DisplayConfig", "GetCurrentState", None, None,
                      Gio.DBusCallFlags.NONE, 3000, None)
        return True
    except GLib.Error:
        return False


def main():
    argv = sys.argv[1:]
    x11 = "--x11" in argv
    shots = []
    for i, arg in enumerate(argv[:argv.index("--")]):
        if arg == "--shot":
            when, path = argv[i + 1].split(":", 1)
            shots.append((float(when), os.path.abspath(path)))
    shots.sort()
    command = argv[argv.index("--") + 1:]

    for attempt in range(4):
        env, display = start(x11)
        if responsive():
            break
        print(f"gnome: the shell froze starting up; again ({attempt + 1})", flush=True)
        teardown()
    else:
        print("gnome: the shell kept freezing", flush=True)
        return 125
    # The shell starts in the overview, where a new window is not shown.
    try:
        session_bus().call_sync("org.gnome.Shell", "/org/gnome/Shell",
                                "org.freedesktop.DBus.Properties", "Set",
                                GLib.Variant("(ssv)", ("org.gnome.Shell", "OverviewActive",
                                                       GLib.Variant("b", False))),
                                None, Gio.DBusCallFlags.NONE, 3000, None)
        time.sleep(1)
    except GLib.Error as error:
        print(f"gnome: the overview stays: {error.message}", flush=True)
    try:
        test_env = dict(env)
        if x11:
            test_env["DISPLAY"] = display
            auth = glob.glob(f"{run}/.mutter-Xwaylandauth.*")
            if auth:
                test_env["XAUTHORITY"] = auth[0]
            print(f"gnome: Xwayland {display}", flush=True)
        else:
            test_env["WAYLAND_DISPLAY"] = "xw-test"
            print("gnome: Wayland xw-test", flush=True)
        folder = f"{run}/frames"
        # The cast lives as long as the connection that asked for it.
        cast_bus = session_bus()
        recorder = record(cast_bus, folder) if shots else None
        if recorder:
            children.append(recorder)
            time.sleep(0.5)
        program = subprocess.Popen(command, env=test_env)
        started = time.time()
        for when, path in shots:
            time.sleep(max(0.0, started + when - time.time()))
            # Copied at its moment: the recorder keeps only the latest frames.
            frame = frame_now(folder)
            try:
                shutil.copy(frame, path)
                print(f"gnome: screenshot at {when:g}s {path}", flush=True)
            except (OSError, TypeError):
                print(f"gnome: no frame by {when:g}s", flush=True)
        try:
            return program.wait(timeout=600)
        except subprocess.TimeoutExpired:
            program.kill()
            return 124
    finally:
        teardown()


def stop(signum, frame):
    raise SystemExit(128 + signum)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGHUP, stop)
    sys.exit(main())
