# Gamepads

A player plugs in a controller, holds a stick, and switches to a menu. Device
discovery should not depend on which window reads a keyboard event. xwindow
provides one application-wide controller source; the game or UI maps its
controls to actions.

## Find a controller

```haxe
import window.Gamepad;

if (Gamepad.available()) {
    for (i in 0...Gamepad.count()) {
        var pad = Gamepad.at(i);
        Sys.println(pad.name());
    }
} else {
    Sys.println(Gamepad.error());
}
```

Native discovery works without opening a window. In a browser, open the
canvas window first; the page's permissions and controller interaction
determine which devices the browser exposes. An unavailable backend is
different from an available backend with zero connected controllers.

## Read positional controls

The named buttons follow the W3C standard gamepad's positions: `South`,
`East`, `West`, `North`, shoulders, analog triggers, menu buttons, stick
presses, D-pad and `Guide`. Names describe positions; a game can display
device-specific labels separately.

`pad.button(LeftTrigger)` reads a value from zero to one.
`pad.pressed(South)` reads the button's pressed state. `pad.axis(LeftX)` and
`pad.axis(LeftY)` read values from minus one to one; positive Y points down.
Use `hasButton` and `hasAxis` before binding a control the device might lack.
xwindow applies no application dead zone or sensitivity curve.

The native backend uses gilrs' mapped controls, including SDL controller
mappings. Browser devices without the `standard` mapping remain enumerable
but have no named controls: xwindow does not guess their raw layout.
Native Android and iOS controller backends are not implemented by this API
yet. Desktop Linux/FreeBSD, Windows and macOS use gilrs; Linux requires libudev
development files to build. Runtime capability queries describe the actual
backend and connected device.

## PlayStation controllers

Recognized DualShock and DualSense controllers expose the same positional
controls as other standard pads:

| Control | PlayStation label |
| --- | --- |
| `South`, `East`, `West`, `North` | Cross, Circle, Square, Triangle |
| `LeftShoulder`, `RightShoulder` | L1, R1 |
| `LeftTrigger`, `RightTrigger` | L2, R2 (analog) |
| `LeftStick`, `RightStick` | L3, R3 clicks |
| `Select`, `Start`, `Guide` | Share/Create, Options, PS, when exposed |

A game binds an action such as jump to `South`; its UI can display the label
appropriate for the controller. Printed button labels do not change the
positional binding. Device mappings can vary between USB, Bluetooth and
platforms; the native backend uses gilrs' driver mappings and bundled
[SDL controller database](https://github.com/gabomdq/SDL_GameControllerDB).
It also honors `SDL_GAMECONTROLLERCONFIG` for native mapping overrides.
Browsers provide their own
[standard mapping](https://www.w3.org/TR/gamepad/#remapping).

The current API exposes the device name and control capabilities. It does
not yet expose controller-family metadata or a button-glyph library. Player
rebinding belongs in the application's action layer. Touchpad coordinates,
gyro data and adaptive-trigger effects require additional device capabilities
beyond this standard control API.

## Sample one fixed step

Individual getters are useful for inspection. For a game loop, reserve a
buffer once and obtain the entire controller state in one call:

```haxe
var state = haxe.io.Bytes.alloc(176);
// Reuse state at every fixed simulation boundary.
if (pad.snapshot(state)) {
    var leftX = state.getDouble(136);
    var leftY = state.getDouble(144);
    var pressed = state.getInt32(168);
    var south = (pressed & 1) != 0;
}
```

The buffer contains 17 little-endian doubles for buttons in the order of
`GamepadButton`, followed by four doubles for `LeftX`, `LeftY`, `RightX`,
`RightY`. A 32-bit pressed mask starts at byte 168; a 32-bit change sequence
at byte 172. A short buffer or invalid handle returns false without writing
any bytes. The native snapshot holds one state lock while copying into the
caller-owned buffer. A program can build its own typed input frame from it
and reuse both storages.

The browser samples on animation frames and forwards complete changed
snapshots through the declared wire. A game reads the latest available
sample at its fixed boundary; recording those input frames defines replay.

## Handle hot-plugging

`Gamepad.poll()` drains a separate application-wide queue. `Connected`
includes a `device_id`; `Gamepad.fromId(device_id)` obtains its handle.
`Disconnected` invalidates that handle. Reconnecting creates a new handle,
even when a platform reuses its device index. A program can keep handles and
snapshot buffers for connected devices, rebuilding bindings on hot-plug
events. One owner drains this queue and distributes input to the game and UI.

For a fixed-step input layer, `Gamepad.drain(events)` copies queued events
into a reused byte buffer and returns the number of complete records. It
consumes the same queue as `poll`, preserving short press/release pairs
without constructing Haxe event objects. Each 32-byte record contains
little-endian fields:

| Offset | Field |
| --- | --- |
| 0 | u32 kind: connected 1, disconnected 2, button 3, axis 4, overflow 5 |
| 4 | i32 device handle |
| 8 | u32 button or axis index |
| 12 | u32 pressed flag (button events) |
| 16 | f64 button or axis value |
| 24 | u64 dropped count (overflow events) |

Unused fields are zero. The unwritten tail is untouched. A buffer shorter
than one record consumes no events. Allocate enough records for a batch and
call again while it fills; `poll` remains useful for readable inspection.

`Button` carries its value and pressed state; `Axis` carries its value. The
queue is bounded. `Overflow` reports lost events; snapshots still contain
current state. An application needing every short button tap should drain
events regularly and preserve their edges until its fixed step consumes
them. Per-step pressed/released edges and action contexts belong in the
application's input layer.

Native changes wake xwindow's loop through its existing wake proxy, so an
idle `Window.wait` returns without periodic polling on the main thread.
Read the gamepad queue after that wake. Window keyboard and pointer queues
remain independent.

## Rumble

```haxe
if (pad.rumbleSupported()) pad.rumble(0.6, 0.2, 0.15);
```

Strong and weak magnitudes range from zero to one; duration is zero to five
seconds. A request replaces the previous effect. Zero duration stops it.
The return value reports whether the backend accepted the request, not
whether the hardware physically vibrated. Support is queried per controller;
gilrs does not supply native macOS rumble. Browsers use a dual-rumble
vibration actuator when available.

## Try it

The generated [controller example](../examples/gamepads/Main.hx) prints
hot-plug/control events and samples a reused snapshot. Build it against an
updated runtime adapter's `window` package. It can be compiled for Ash with:

```sh
haxe -cp ../hlwindow/haxe -cp examples/gamepads -main Main -hl /tmp/gamepads.hl
```

Run the bytecode beside the updated `xwindow.hdll`, with that file available
to Ash's native library loader. Applications use the adapter package;
`api/window.api.rs` and the browser `.idl` files remain the sources for
generated surfaces.
