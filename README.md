# xwindow

xwindow gives Caribou, Ash and Rayzor one `window` API: windows, their
events, monitors, cursors and text input. Natively it is backed by winit: on
a desktop, and on Android and iOS through a hook the host calls (see
CONTRIBUTING.md). In a page the window is the page's canvas, driven by a browser agent that
xwindow generates and ships.

## When to use it

Use xwindow when you are building a runtime adapter, and want:

- the same window classes, events and enum values on every supported Haxe
  runtime;
- bindings generated from one declaration, rather than externs maintained by
  hand;
- a window that hands a GPU library what it needs for a surface; or
- the same API in a browser.

Application developers use their runtime's xwindow adapter. They do not call
the generator or depend on xwindow's crates.

## The API

```haxe
import window.*;

var attributes = new WindowAttributes();
attributes.title("Hello");
attributes.width(800);
attributes.height(600);
var window = Window.open(attributes);

var running = true;
while (running) {
	switch (window.wait(0.5)) {
		case Closed:
			running = false;
		case Resized(width, height):
			trace('${width}x${height}');
		case KeyboardInput(_, Input(Code(Escape), _, _, _, Pressed, _, _), _):
			running = false;
		case _:
	}
}
window.close();
```

`Window.poll()` returns the next event without waiting, and `wait(seconds)`
waits for one. Both return `Event.None` when there is none.

Events have the shapes cb_window gave them, which are winit's:

- A key press is `KeyboardInput(device, KeyEvent.Input(...), synthetic)`.
  The physical key is a `KeyCode` or the platform's own code. The logical
  key is named, a character, dead or the platform's. The press also
  carries its text, location, state and repeat, and the key without
  modifiers where the platform says.
- `Modifiers` holds the side each modifier is held on.
- `FilePath` keeps the exact bytes of a path that is not Unicode.
- `TouchForce` keeps calibrated force.
- `OptionalText` keeps none distinct from empty.

Every runtime gets `Event` as a real enum with nested payloads:

- Caribou builds it natively.
- Ash and Rayzor read each field through a generated getter, and their
  generated externs build the enum from them.

[`examples/events/Main.hx`](examples/events/Main.hx) builds unchanged against
each runtime's package.

Sizes and positions a window reports are in physical pixels. Sizes and
positions a program asks for are logical, as winit takes them.

The declaration is [`api/window.api.rs`](api/window.api.rs). It covers:

| Area | What it has |
|---|---|
| Window | Size, position, inner position, scale, title, icon, theme, visibility, focus, fullscreen, exclusive fullscreen, decorations, enabled buttons, resize increments, level, transparency, blur, content protection, dragging, the window menu |
| Cursor | Icons, RGBA images, visibility, grab, position, hit testing |
| Input methods | IME allowance, purpose and cursor area |
| Monitors | Name, size, position, scale, refresh rate, video modes |
| Clipboard | Plain text, read and written; a page writes only |
| Activation tokens | Requests, answered by event |
| Events | All 28 of winit 0.30's window events, its 7 raw device events, and its application lifecycle |

`KeyCode` and `NamedKey` list winit's 194 physical and 306 named keys. Those are
the W3C names a browser reports too.

## Runtime support

| Runtime | Native | Browser | Adapter |
|---|---|---|---|
| Caribou | winit | The page agent, started through `host::agent` | `caribou/plugins/cb_window` moves onto xwindow, as `cb_gpu` did onto xgpu |
| Ash / HashLink | winit, loaded as `xwindow.hdll` | The page agent, started through `ash_host_agent` | [hlwindow](https://github.com/rayzor-blade/hlwindow), built as `xwindow.hdll` and `xwindow.wasm` |
| Rayzor | winit, in an `.rpkg` | Needs a Rayzor agent hook | A window package beside `rayzor-gpu.rpkg` |

Adapters stay in their host repositories, as xgpu's do.

## A window for a GPU surface

A window reaches a GPU library as a platform code and four integers.
xgpu's `GpuInstance.surface` takes them directly:

```haxe
var surface = instance.surface(window.platform(), window.raw(0), window.raw(1), window.raw(2), window.raw(3));
```

The codes are xgpu's: 1 AppKit, 2 Win32, 3 Xlib, 4 Wayland, 5 Android,
6 UIKit, 7 web canvas and 8 web OffscreenCanvas. `xwindow_core::raw` splits
and joins them. In a page, the window and the GPU both use the page's
canvas.

## Browser host contract

xwindow generates the program's half of the browser wire. Its page agent
(`xwindow.mjs`, beside the generated `xwindow_wire.mjs`) runs on the page's
thread, where the canvas's events fire. The consuming runtime provides the
harness. It must:

- run a threaded wasm build over shared `WebAssembly.Memory`;
- receive the adapter's request to start its `xwindow` agent;
- serve `xwindow.mjs` and `xwindow_wire.mjs` beside the program;
- import `xwindow.mjs` on the page's thread and call
  `start({ memory, address, canvas })`; and
- serve the page with the isolation headers `SharedArrayBuffer` needs.

What the program asks of the window crosses the generated wire's mailbox.
Some requests are the DOM's own operations, such as `Document.title` and
`CSSStyleDeclaration.setProperty`; the rest are the agent's, declared in
[`api/spec/agent.idl`](api/spec/agent.idl). The agent posts each event to a
queue in the program's memory, which `poll` reads without a call. A page has
one window. Placing or decorating it does nothing, and asking for
fullscreen, pointer lock or focus waits for the user's next input, as
browsers require.

## Versioning

xwindow is consumed by pinned Git revisions. Pin one revision for the
generator, core and backend so the declared API and its backends stay in
step.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the layout, the adapter boundary
and the validation commands.
