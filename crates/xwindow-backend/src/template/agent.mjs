// xwindow's page agent. A program in a page opens a window, and the window is
// the page's canvas. The program's host imports this module on the page's
// thread, where the canvas's events fire, and calls
// `start({ memory, address, canvas })` with the program's shared memory and
// the address of its block: a mailbox, then 24 bytes on a queue, then 44
// bytes on a redraw word.
//
// The agent serves the mailbox, whose commands the generated wire
// (xwindow_wire.mjs) runs against the page's objects. It posts each event it
// sees to the queue as an XwEvent, which the program reads without a call.
// Positions and sizes it posts are in physical pixels.

import { Wire, Queue, serve, post_XwEvent } from "./xwindow_wire.mjs";

const QUEUE = 24;
const REDRAW = 44;

// The page's objects, under the handles the program names them by.
const WINDOW = 1;
const DOCUMENT = 2;
const CANVAS = 3;
const STYLE = 4;
const AGENT = 5;

const PHASE = { STARTED: 0, MOVED: 1, ENDED: 2, CANCELLED: 3 };

const encoder = new TextEncoder();

// A UTF-16 index in `text` as a UTF-8 byte offset, for an IME's cursor.
const utf8Offset = (text, index) => (index < 0 ? -1 : encoder.encode(text.slice(0, index)).length);

export function start({ memory, address, canvas }) {
  if (!canvas) {
    console.error("xwindow: the host gave the agent no canvas");
    return;
  }
  const queue = new Queue(memory, address + QUEUE);
  const post = (event) => post_XwEvent(queue, event);
  const ratio = () => globalThis.devicePixelRatio || 1;
  const at = (e) => [e.offsetX * ratio(), e.offsetY * ratio()];
  const modifiers = (e) =>
    (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);
  const touch = (e) => e.pointerType === "touch";

  canvas.hidden = false;
  if (canvas.tabIndex < 0) canvas.tabIndex = 0;
  canvas.style.touchAction = "none"; // pointer events for touch, not scrolling
  canvas.style.outline = "none";

  // Requests a browser takes only during a user's input wait for the next.
  let gated = [];
  const settle = (request) => {
    try {
      const done = request();
      if (done && done.catch) done.catch(() => {});
    } catch {}
  };
  const gate = (request) => {
    if (navigator.userActivation?.isActive) settle(request);
    else gated.push(request);
  };
  const runGated = () => {
    const now = gated;
    gated = [];
    now.forEach(settle);
  };

  // Size and scale. The canvas's drawing buffer may belong to a GPU agent by
  // now, which sizes it from the size posted here.
  let size = [0, 0];
  const resized = (width, height) => {
    width = Math.max(1, Math.round(width));
    height = Math.max(1, Math.round(height));
    if (width === size[0] && height === size[1]) return;
    size = [width, height];
    post({ kind: "resized", width, height, scale: ratio() });
  };
  new ResizeObserver((entries) => {
    for (const entry of entries) {
      const box = entry.devicePixelContentBoxSize?.[0];
      if (box) resized(box.inlineSize, box.blockSize);
      else resized(entry.contentRect.width * ratio(), entry.contentRect.height * ratio());
    }
  }).observe(canvas);
  const screenChanged = () => post({ kind: "screen", width: screen.width, height: screen.height });
  globalThis.addEventListener("resize", screenChanged);
  // A change of scale: the query for the current resolution stops matching.
  const watchScale = () => {
    const query = globalThis.matchMedia(`(resolution: ${ratio()}dppx)`);
    query.addEventListener("change", () => {
      post({ kind: "scale-factor-changed", scale: ratio() });
      watchScale();
    }, { once: true });
  };

  // The canvas and the IME's textarea are one window: focus moving between
  // them is no change.
  let hasFocus = null;
  const focused = (on) => {
    if (on === hasFocus) return;
    hasFocus = on;
    post({ kind: "focused", on });
  };
  const focusChange = (on) => (e) => {
    if (e.relatedTarget && (e.relatedTarget === canvas || e.relatedTarget === textarea)) return;
    focused(on);
  };
  canvas.addEventListener("focus", focusChange(true));
  canvas.addEventListener("blur", focusChange(false));
  document.addEventListener("visibilitychange", () =>
    post({ kind: "occluded", on: document.visibilityState === "hidden" }));
  globalThis.addEventListener("pagehide", (e) =>
    post({ kind: e.persisted ? "suspended" : "close-requested" }));
  globalThis.addEventListener("pageshow", (e) => {
    if (e.persisted) post({ kind: "resumed" });
  });
  const dark = globalThis.matchMedia("(prefers-color-scheme: dark)");
  dark.addEventListener("change", () => post({ kind: "theme-changed", on: dark.matches }));
  document.addEventListener("fullscreenchange", () =>
    post({ kind: "fullscreen-changed", on: document.fullscreenElement === canvas }));

  // Pointers. A touch is a pointer of type touch; its force is its pressure.
  canvas.addEventListener("pointerenter", (e) => {
    if (!touch(e)) post({ kind: "cursor-entered", device: e.pointerId });
  });
  canvas.addEventListener("pointerleave", (e) => {
    if (!touch(e)) post({ kind: "cursor-left", device: e.pointerId });
  });
  canvas.addEventListener("pointermove", (e) => {
    const coalesced = e.getCoalescedEvents ? e.getCoalescedEvents() : [];
    for (const m of coalesced.length ? coalesced : [e]) {
      const [x, y] = at(m);
      if (touch(m)) {
        post({ kind: "touch", device: m.pointerId, phase: PHASE.MOVED, x, y, pressure: m.pressure });
      } else {
        post({ kind: "cursor-moved", device: m.pointerId, x, y });
      }
      if (document.pointerLockElement === canvas) {
        post({ kind: "mouse-motion", x: m.movementX * ratio(), y: m.movementY * ratio() });
      }
    }
    // A mouse reports 0.5 while a button is down; anything else is a
    // pressure-sensitive surface.
    if (e.pointerType === "mouse" && e.pressure !== 0 && e.pressure !== 0.5) {
      post({ kind: "touchpad-pressure", pressure: e.pressure, stage: e.pressure > 0.5 ? 2 : 1 });
    }
  });
  const button = (pressed) => (e) => {
    if (pressed) {
      if (document.activeElement !== imeTarget()) imeTarget().focus();
      runGated();
    }
    const [x, y] = at(e);
    if (touch(e)) {
      const phase = pressed ? PHASE.STARTED : PHASE.ENDED;
      post({ kind: "touch", device: e.pointerId, phase, x, y, pressure: e.pressure });
    } else {
      post({ kind: "mouse-input", device: e.pointerId, button: e.button, on: pressed });
    }
  };
  canvas.addEventListener("pointerdown", button(true));
  canvas.addEventListener("pointerup", button(false));
  canvas.addEventListener("pointercancel", (e) => {
    if (touch(e)) post({ kind: "touch", device: e.pointerId, phase: PHASE.CANCELLED, x: 0, y: 0 });
  });
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  // The wheel, and the trackpad pinch browsers report as a wheel with control.
  let pinching = null;
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    if (e.ctrlKey && e.deltaMode === 0) {
      const phase = pinching ? PHASE.MOVED : PHASE.STARTED;
      clearTimeout(pinching);
      pinching = setTimeout(() => {
        pinching = null;
        post({ kind: "pinch-gesture", phase: PHASE.ENDED, scale: 0 });
      }, 150);
      post({ kind: "pinch-gesture", phase, scale: -e.deltaY / 100 });
      return;
    }
    const scale = e.deltaMode === 0 ? ratio() : 1;
    post({ kind: "mouse-wheel", unit: e.deltaMode, x: e.deltaX * scale, y: e.deltaY * scale });
  }, { passive: false });

  // Keys, and the modifiers they change.
  let lastModifiers = -1;
  const key = (pressed) => (e) => {
    if (pressed) runGated();
    post({
      kind: "keyboard-input",
      on: pressed,
      code: e.code,
      key: e.key,
      location: e.location,
      repeat: e.repeat,
      synthetic: !e.isTrusted,
    });
    const mods = modifiers(e);
    if (mods !== lastModifiers) {
      lastModifiers = mods;
      post({ kind: "modifiers-changed", modifiers: mods });
    }
    // Keys the program handles should not also scroll or navigate the page.
    // While text input is allowed they keep their default action, which is
    // how typed text reaches the input method.
    if (!e.metaKey && !e.ctrlKey && !imeAllowed) e.preventDefault();
  };
  const listenKeys = (target) => {
    target.addEventListener("keydown", key(true));
    target.addEventListener("keyup", key(false));
  };
  listenKeys(canvas);

  // Text input through an input method: EditContext where the browser has
  // it, elsewhere a textarea out of sight that takes focus from the canvas
  // while text input is allowed.
  let editContext = null;
  let textarea = null;
  let imeAllowed = false;
  let imeArea = [0, 0, 1, 1];
  const imeTarget = () => (textarea && !textarea.disabled ? textarea : canvas);
  const preedit = (text, start, end) =>
    post({ kind: "ime-preedit", text, start: utf8Offset(text, start), end: utf8Offset(text, end) });
  const commit = (text) => post({ kind: "ime-commit", text });
  const placeIme = () => {
    const rect = canvas.getBoundingClientRect();
    const [x, y, w, h] = imeArea;
    const bounds = new DOMRect(rect.left + x, rect.top + y, w, h);
    if (editContext) {
      editContext.updateControlBounds(rect);
      editContext.updateSelectionBounds(bounds);
    }
    if (textarea) {
      Object.assign(textarea.style, {
        left: `${bounds.left + globalThis.scrollX}px`,
        top: `${bounds.top + globalThis.scrollY}px`,
        height: `${Math.max(1, bounds.height)}px`,
      });
    }
  };
  const allowIme = (allowed) => {
    imeAllowed = allowed;
    if (!allowed) {
      if (canvas.editContext) canvas.editContext = null;
      if (textarea) {
        const had = document.activeElement === textarea;
        textarea.disabled = true;
        if (had) canvas.focus();
      }
      post({ kind: "ime-disabled" });
      return;
    }
    if ("EditContext" in globalThis) {
      if (!editContext) {
        editContext = new EditContext();
        let composing = false;
        const clear = () => {
          editContext.updateText(0, editContext.text.length, "");
          editContext.updateSelection(0, 0);
        };
        editContext.addEventListener("compositionstart", () => { composing = true; });
        editContext.addEventListener("compositionend", () => {
          composing = false;
          commit(editContext.text);
          clear();
        });
        editContext.addEventListener("textupdate", () => {
          const text = editContext.text;
          if (composing) preedit(text, editContext.selectionStart, editContext.selectionEnd);
          else {
            commit(text);
            clear();
          }
        });
      }
      canvas.editContext = editContext;
    } else {
      if (!textarea) {
        textarea = document.createElement("textarea");
        textarea.setAttribute("autocomplete", "off");
        textarea.setAttribute("autocapitalize", "off");
        textarea.setAttribute("spellcheck", "false");
        Object.assign(textarea.style, {
          position: "absolute", width: "1px", opacity: "0", padding: "0", border: "0",
          resize: "none", overflow: "hidden", pointerEvents: "none",
        });
        document.body.appendChild(textarea);
        listenKeys(textarea);
        textarea.addEventListener("focus", focusChange(true));
        textarea.addEventListener("blur", focusChange(false));
        textarea.addEventListener("compositionupdate", (e) => {
          const text = e.data || "";
          preedit(text, text.length, text.length);
        });
        textarea.addEventListener("compositionend", (e) => {
          commit(e.data || "");
          textarea.value = "";
        });
        textarea.addEventListener("input", (e) => {
          if (!e.isComposing && e.data) commit(e.data);
          if (!e.isComposing) textarea.value = "";
        });
      }
      textarea.disabled = false;
      if (document.activeElement === canvas) textarea.focus();
    }
    placeIme();
    post({ kind: "ime-enabled" });
  };

  // Files dragged onto the canvas. A page learns a file's name only when it
  // is dropped, and never its path.
  canvas.addEventListener("dragenter", (e) => {
    e.preventDefault();
    for (const item of e.dataTransfer?.items || []) {
      if (item.kind === "file") post({ kind: "hovered-file", text: "" });
    }
  });
  canvas.addEventListener("dragover", (e) => e.preventDefault());
  canvas.addEventListener("dragleave", (e) => {
    if (!canvas.contains(e.relatedTarget)) post({ kind: "hovered-file-cancelled" });
  });
  canvas.addEventListener("drop", (e) => {
    e.preventDefault();
    for (const file of e.dataTransfer?.files || []) post({ kind: "dropped-file", text: file.name });
  });

  const cssSize = (w, h, first, second) => {
    canvas.style[first] = w > 0 ? `${w}px` : "";
    canvas.style[second] = h > 0 ? `${h}px` : "";
  };

  // XwAgent, which the program's commands call.
  const agent = {
    setSize: (w, h) => cssSize(w, h, "width", "height"),
    setMinSize: (w, h) => cssSize(w, h, "minWidth", "minHeight"),
    setMaxSize: (w, h) => cssSize(w, h, "maxWidth", "maxHeight"),
    setVisible: (visible) => { canvas.hidden = !visible; },
    setFullscreen: (on) => {
      if (on) gate(() => canvas.requestFullscreen());
      else if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    },
    setCursorGrab: (locked) => {
      if (locked) gate(() => canvas.requestPointerLock());
      else if (document.pointerLockElement === canvas) document.exitPointerLock();
    },
    // The pixels are the program's memory, read while this command runs.
    setCursorImage: (rgba, width, height, hotX, hotY) => {
      const pixels = new Uint8ClampedArray(rgba.length);
      pixels.set(rgba);
      const image = new OffscreenCanvas(width, height);
      image.getContext("2d").putImageData(new ImageData(pixels, width, height), 0, 0);
      image.convertToBlob().then((blob) => {
        canvas.style.cursor = `url(${URL.createObjectURL(blob)}) ${hotX} ${hotY}, auto`;
      });
    },
    focus: () => imeTarget().focus(),
    setImeAllowed: allowIme,
    setImeArea: (x, y, w, h) => {
      imeArea = [x, y, w, h];
      placeIme();
    },
  };

  // A redraw the program asked for since the last frame is answered in this
  // one.
  const frame = () => {
    const words = new Int32Array(memory.buffer, address + REDRAW, 1);
    if (Atomics.exchange(words, 0, 0) !== 0) post({ kind: "redraw-requested" });
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  // The window as it is.
  screenChanged();
  watchScale();
  const rect = canvas.getBoundingClientRect();
  resized(rect.width * ratio(), rect.height * ratio());
  focused(document.activeElement === canvas);
  post({ kind: "theme-changed", on: dark.matches });

  const wire = new Wire(memory, new Map([
    [WINDOW, globalThis],
    [DOCUMENT, document],
    [CANVAS, canvas],
    [STYLE, canvas.style],
    [AGENT, agent],
  ]));
  serve(wire, address);
}
