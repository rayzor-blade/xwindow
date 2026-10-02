# xwindow API and browser specification input

[`window.api.rs`](window.api.rs) declares the `window` API every runtime
adapter exposes. xwindow-bindgen appends `KeyCode`, `NamedKey` and
`CursorIcon` to it from xwindow-core's lists.

## `spec/window.idl`

The browser APIs the page agent reaches. These are whole definitions,
verbatim from each specification's IDL as W3C's
[webref](https://github.com/w3c/webref) collects it: the curated branch at
revision `89e7d68c6612fbedb740c1632d36e482ca39e999` (2026-09-22). Each
section of the file names its specification and licence. It is the file
Caribou's window plugin vendored, and the one x-idl's wire tests parse.

The wire generated from it carries every operation and attribute whose
values it can encode. The program uses two directly:

- `Document.title`
- `CSSStyleDeclaration.setProperty`, for the cursor

## `spec/agent.idl`

xwindow's own definitions, which the wire carries beside the DOM's:

- `XwAgent` holds what a page does that no single DOM call can. It sizes
  the canvas, and makes requests that need a user's input. It also builds
  a cursor from pixels and runs an input method.
- `XwEvent` is one event as the agent posts it to the program's queue. Its
  `kind` says which members it has.
- `CSSOMString` is aliased to `DOMString`. CSSOM leaves the choice to
  implementations, and the wire carries either as UTF-8.

## Where the members map in a page

| Window member | In a page |
|---|---|
| `open` | The page's canvas; a page has one window |
| `setTitle` | `Document.title` |
| `setSize`, `setMinSize`, `setMaxSize` | The canvas's CSS size; the drawing buffer is that times `devicePixelRatio` |
| `width`, `height` | `ResizeObserverEntry.devicePixelContentBoxSize` |
| `scaleFactor` | `Window.devicePixelRatio` |
| `setVisible`, `close` | `HTMLElement.hidden` |
| `setFullscreen` | `Element.requestFullscreen`, waiting for the user's next input |
| `setCursorIcon`, `setCursorVisible` | `style.cursor` |
| `setCursorImage` | An `OffscreenCanvas` blob as `style.cursor` |
| `setCursorGrab` | `Element.requestPointerLock`, waiting for the user's next input; a page cannot confine |
| `focus` | `HTMLElement.focus` |
| `setImeAllowed`, `setImeCursorArea` | `EditContext`, or a hidden textarea where a browser has none |
| `requestRedraw` | A word the agent reads each animation frame |
| `currentMonitor` | `Window.screen` |
| Placement, decorations, level, blur, attention, activation tokens | Nothing; a page cannot |

Every event a page has an equivalent for maps as Ash's page did:

- pointer, wheel, key, focus, visibility, theme, scale, resize, drag and
  drop, and IME events;
- a wheel with control as a trackpad pinch;
- pointer-locked movement as `MouseMotion`.

A page learns a dropped file's name, never its path.

## Updating the snapshot

Take the same definitions from a newer webref revision, record the revision
here and in the file's header, and run `cargo test --workspace`. The build
reads these files; it never rewrites them.
