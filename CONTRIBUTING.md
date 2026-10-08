# Contributing to xwindow

The user-facing contract is the `window` namespace. Keep API changes in the
shared declaration so every runtime sees the same classes, records, enums,
events and method names.

## Repository layout

- `api/window.api.rs` is the typed API declaration.
- `api/spec/window.idl` holds the browser APIs the page agent reaches,
  verbatim from W3C's webref. `api/spec/agent.idl` holds the agent's own
  operations and the event record it posts.
- `crates/xwindow-core` has the runtime-neutral parts:
  - winit's key and cursor lists, as macros the declaration and both
    backends expand;
  - generational handles;
  - the surface contract, `raw::split` and `raw::join`.
- `crates/xwindow-bindgen` runs x-idl over the declaration for each runtime.
  It generates the browser wire and builds `xwindow-haxe`.
- `crates/xwindow-backend` carries the winit backend, the browser backend
  and the page agent, which adapters compile beneath their own ABI.
- `crates/xwindow-check` is a stand-in adapter. It compiles the generated
  model with both backends against carriers that only have a runtime's
  shapes.
- Runtime adapters stay in their host repositories. Text, buffers, errors,
  roots, allocation and exported symbols follow the host runtime's ABI.

## The declaration

x-idl reads Rust-shaped declarations:

- A trait is a resource class whose objects are integer handles.
- A struct is a record that a program fills in.
- A fieldless enum is a set of integer codes.
- An enum whose variants have named fields is a set of variants, such as
  `Event`:
  - Its fields are numbers, `bool`, `Text`, `Buffer`, `Enum<T>`, or other
    variants, which nest.
  - Its first variant, with its fields at their defaults, is what a failed
    call returns.

How each runtime takes variants:

- **Caribou** takes them whole, as nested `PluginEnum`s.
- **HashLink and Rayzor** take the variant's index. Each class keeps the
  last value of each variants type it returns. Every field, at any depth,
  has a getter over that value. A generated `read<Type>(index)` in the
  Haxe surface builds the enum from these getters.

A `Buffer` field is held as the generated `VariantBytes(Vec<u8>)`.

`xwindow_bindgen::window_api()` appends `KeyCode`, `NamedKey` and
`CursorIcon` from xwindow-core's lists. Their variants share winit's names,
so the backends convert by name, and a winit upgrade that renames a key
fails to compile. `Unrecognized` stands for a key a newer winit names.
Update the lists in `xwindow-core` when upgrading winit.

## Adapter boundary

An adapter's build script calls:

```rust
let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
xwindow_backend::install(&out)?; // xwindow_backend/{native,web}.rs, page/xwindow.mjs
fs::write(out.join("window.rs"), xwindow_bindgen::generate(Runtime::Caribou)?)?;
let wire = xwindow_bindgen::browser_wire()?;
fs::write(out.join("window_wire.rs"), wire.rust)?;
fs::write(out.join("page").join(xwindow_backend::WIRE_MODULE), wire.js)?;
if browser {
    let backend = xwindow_bindgen::web_backend(Runtime::Caribou, xwindow_backend::WEB)?;
    fs::write(out.join("window_web_backend.rs"), backend)?;
}
```

It then includes the generated model at its crate root (or under a scope,
with `install_scoped`):

- `mod backend` is `native.rs` natively.
- In a browser, `mod web` is `web.rs`, `mod backend` is the generated
  forwarder, and `mod wire` is the wire.

`crate::runtime` must provide:

- `Text`, `Buffer` (with `new`, `NULL`, `len` and `as_ptr`) and `ErrorKind`;
- `host::raise`, and in a browser `host::agent`;
- natively, `host::blocking(bool)`. The backend calls it with true before
  it pumps the platform and with false after, so a runtime with a
  collector can collect while the thread waits. Nothing between the two
  touches the runtime's heap or calls the program. HashLink's is
  `hl_blocking`.

The adapter depends on `xwindow-core`. Natively it also depends on `winit`
0.30, `raw-window-handle` 0.6 and `xwindow-clipboard`, which reaches each
platform's clipboard through crates winit already links.
`crates/xwindow-check` is the smallest working shape.

Per runtime:

- **Caribou** uses `caribou_abi`'s carriers. The plugin is named `window`.
- **HashLink/Ash** uses an hl_abi runtime module: hlwgpu's `hl_xidl`,
  which the [hlwindow](https://github.com/rayzor-blade/hlwindow) adapter
  uses. Its primitives load from
  `xwindow.hdll`, or from the side module `xwindow.wasm` in a wasm
  build, and records are `hl.Abstract<"xwindow_*">`.
  - A `Buffer` argument arrives as a `haxe.io.Bytes`: its length, then
    its data. The runtime module defines that layout as `HlBytes`, and
    `Buffer::from_hl` takes `*mut HlBytes`.
  - The generated primitives copy `Buffer` results out. They need the
    carrier's `len` and `as_ptr`.
- **Rayzor** uses Rayzor's carriers, like rayzor-gpu's. It exports
  `export_abi_version!()` and
  `rpkg_entry!(XIDL_METHODS, xidl_runtime_symbols)`. Every symbol is
  `xwindow_*`, and every class `window::*`, matching the externs.

The native backend shares one winit event loop among all windows, on the
thread that opened the first. Each window's events are sorted into that
window's queue. Raw device events go to the focused window.

### Integrating another event loop

An adapter already owning a main-thread I/O loop can enable
`backend::external_pump(true)` after creating a pumped loop. Window `poll` then
only drains queued events. The adapter calls `pump_external(timeout)` once when
it is ready to wait; it returns when platform events, the timeout, or an
`external_waker()` proxy signal arrives. `external_pending()` checks queued
window work without pumping. Disable the integration before releasing the host.
These hooks do not apply to `Drive::Turns`.

Keep all window operations on the owning thread. A helper thread may watch the
runtime's I/O descriptor and send a proxy wake; it must not pump the window loop
or run application callbacks. On macOS, a proxy wake also interrupts AppKit's
wait even when no mouse or window event arrives. The adapter must service its
own timers, flush pending I/O registrations, and drain every managed window.

`XWINDOW_DESKTOP=1 XWINDOW_EXTERNAL_PUMP=1 cargo test -p xwindow-check --test desktop`
checks the desktop operations through these hooks. The default desktop test
continues to exercise ordinary window polling and waiting.

### The host's hook

By default the program pumps the loop inside `open`, `poll` and `wait`,
through winit's `pump_events`. That works on Windows, macOS, X11 and
Wayland. A host that has to build the loop, or run it, hands it to the
backend instead, before the first window opens:

```rust
attach(events, Drive::Pump)?;           // the program pumps the host's loop
attach(events, Drive::Turns(&mut turn))?; // the host runs it; returns when it ends
```

- **Android:** the host builds the loop with the `AndroidApp` its
  `android_main` receives, using `EventLoop::builder().with_android_app(app)`.
  Either drive works. `open` waits for the activity to resume.
- **iOS:** winit cannot pump, so only `Drive::Turns` works. The host
  calls `attach` from `main`, and it never returns.

Under `Drive::Turns`, `turn` is the program's turn. The loop calls it each
time events have come, until it returns false. In a turn:

- a window can open, and only in a turn;
- `poll` and `wait` return what has come without blocking;
- the last of them to find nothing says when the next turn is: at once
  after `poll`, within the timeout after `wait`, and when events come after
  a negative `wait`.

An adapter that runs natively must:

- re-export `backend::{attach, Drive}`, and `winit` so a host builds the
  loop from the same crate;
- on Android, enable one of winit's activity features, which cannot both
  be on. Its own `android-native-activity` feature is on by default and
  forwards to winit's. A host on a GameActivity turns default features off
  and enables `android-game-activity`.

`cargo run -p xwindow-check --example host -- pump|turns` runs a program
both ways on a desktop and checks what it saw.

A host in C, such as an app that links HashLink statically on a phone,
reaches the hook through two symbols every native adapter exports:

```c
/* Drive::Pump; 0 when attached, -1 when not (always on iOS). */
int xwindow_attach_pump(void);
/* Drive::Turns: calls turn(data) each turn until it returns 0. Returns 0
   when the loop ends, which on iOS it never does, and -1 when it cannot run. */
int xwindow_run_turns(int (*turn)(void *data), void *data);
```

On Android the `AndroidApp` comes from `android_main`, which only Rust
defines. An adapter built for a host in C defines it behind an
`android-main` feature: it passes the app to `backend::android_app` and
calls `xwindow_main()`, the host's program entry. With the app kept, a
program that opens a window without either call gets a pumped loop.

## Browser boundary

`web.rs` encodes each request onto the generated wire. Requests are batched
and sent through the mailbox when the program next polls or waits. The
mailbox is at the address the host's agent hook receives. The queue is 24
bytes on, and a redraw word 44 bytes on, which the agent reads every frame.

`agent.mjs` keeps the page's objects under fixed handles:

| Handle | Object |
|---|---|
| 1 | `window` |
| 2 | `document` |
| 3 | The canvas |
| 4 | The canvas's `style` |
| 5 | The agent, which implements `XwAgent` |

The agent posts `XwEvent`s through the generated `post_XwEvent`.

Keep runtime-specific wasm loading, side-module discovery, COOP/COEP serving
and canvas transfer out of xwindow.

## Validation

```sh
cargo test --workspace
cargo build -p xwindow-check --target wasm32-wasip1-threads
```

The `page_agent` test runs the real agent under Node against a stand-in DOM.
It needs `rustc` and `node`. Generate both Haxe surfaces when changing the
declaration:

```sh
cargo run -p xwindow-bindgen --bin xwindow-haxe -- ash /tmp/xwindow-ash
cargo run -p xwindow-bindgen --bin xwindow-haxe -- rayzor /tmp/xwindow-rayzor
haxe -cp /tmp/xwindow-ash -cp examples/events -main Main -hl /tmp/events.hl
```

`XWINDOW_DESKTOP=1 cargo test -p xwindow-check --test desktop` opens real
windows and drives them with synthetic input. It drives X11 with xdotool,
sway and GNOME on Wayland, Windows, and macOS through Quartz events, which
need the Accessibility permission for the terminal; CI runs it on X11,
sway and Windows. The macOS run moves the pointer and types into its
windows, so keep the mouse and keyboard still while it runs. With Stage
Manager on and another app in front, a new window waits in the side strip
as a small tilted thumbnail until it is clicked, which input aimed at it
misses. On a machine with GNOME, `scripts/gnome_session.py` runs it in a
private headless GNOME Shell, beside any session already open, and can
screenshot the monitor while it runs:

```sh
scripts/gnome_session.py -- env XWINDOW_DESKTOP=1 cargo test -p xwindow-check --test desktop
scripts/gnome_session.py --x11 --shot 2:frame.png -- cargo run -p xwindow-check --example host
```
