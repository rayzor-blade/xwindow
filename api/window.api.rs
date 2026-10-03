// The `window` API every xwindow adapter exposes: windows, their events and
// the monitors they are on. Positions and sizes a window reports are in
// physical pixels; sizes and positions a program asks for are logical, as
// winit takes them. Events have the shapes winit gives them.
// xwindow-bindgen appends `KeyCode`, `NamedKey` and `CursorIcon` from
// xwindow-core's lists.

enum WindowLevel {
    Normal,
    AlwaysOnBottom,
    AlwaysOnTop,
}

enum Theme {
    Light,
    Dark,
}

/// How a window's size follows a change of scale: `Logical` keeps its
/// logical size, as winit suggests; `Physical` keeps its size in pixels.
enum ScaleSizing {
    Logical,
    Physical,
}

/// The edge or corner `dragResizeWindow` resizes from.
enum ResizeDirection {
    East,
    North,
    NorthEast,
    NorthWest,
    South,
    SouthEast,
    SouthWest,
    West,
}

/// What an input method is told the text is for.
enum ImePurpose {
    Normal,
    Password,
    Terminal,
}

/// When the platform delivers raw device events (`Event.Device`).
enum DeviceEvents {
    Always,
    WhenFocused,
    Never,
}

enum CursorGrab {
    None,
    Confined,
    Locked,
}

enum Attention {
    None,
    Informational,
    Critical,
}

/// Whether a key or button is down.
enum MouseElementState {
    Pressed,
    Released,
}

enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other { button: i32 },
}

/// Lines and rows of text, or pixels. Positive `y` scrolls the content
/// down the window, as winit reports.
enum MouseScrollDelta {
    LineDelta { x: f64, y: f64 },
    PixelDelta { x: f64, y: f64 },
}

enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
}

/// Text, or none, which is not the same as empty.
enum OptionalText {
    None,
    Some { text: Text },
}

/// Bytes, or none, which is not the same as empty.
enum OptionalBytes {
    None,
    Some { bytes: Buffer },
}

enum OptionalFloat {
    None,
    Some { value: f64 },
}

/// UTF-8 byte offsets of an IME's cursor in its text.
enum CursorRange {
    None,
    Range { start: i64, end: i64 },
}

enum Ime {
    Enabled,
    Preedit { text: Text, cursor: CursorRange },
    Commit { text: Text },
    Disabled,
}

/// A path as Unicode when it is; otherwise its exact bytes: a Unix path's,
/// or a Windows path's UTF-16 code units, little-endian. In a page, a
/// dropped file's name.
enum FilePath {
    Utf8 { path: Text },
    UnixBytes { bytes: Buffer },
    WindowsWide { utf16le: Buffer },
}

enum TouchForce {
    None,
    Calibrated {
        force: f64,
        max_possible_force: f64,
        altitude_angle: OptionalFloat,
    },
    Normalized { force: f64 },
}

/// A platform's code for a physical key winit does not name.
enum NativeKeyCode {
    Unidentified,
    Android { code: i64 },
    MacOS { code: i32 },
    Windows { code: i32 },
    Xkb { code: i64 },
}

/// A platform's code or name for a logical key winit does not name.
enum NativeKey {
    Unidentified,
    Android { code: i64 },
    MacOS { code: i32 },
    Windows { code: i32 },
    Xkb { code: i64 },
    Web { key: Text },
}

enum PhysicalKey {
    Code { code: Enum<KeyCode> },
    Unidentified { code: NativeKeyCode },
}

enum Key {
    Named { key: Enum<NamedKey> },
    Character { text: Text },
    Unidentified { key: NativeKey },
    Dead { character: OptionalText },
}

/// What the key would be without modifiers, and the text it types with all
/// of them, where the platform says.
enum KeySupplement {
    Unavailable,
    Supplement {
        key_without_modifiers: Key,
        text_with_all_modifiers: OptionalText,
    },
}

enum KeyEvent {
    Input {
        physical_key: PhysicalKey,
        logical_key: Key,
        text: OptionalText,
        location: Enum<KeyLocation>,
        state: Enum<MouseElementState>,
        repeat: bool,
        supplement: KeySupplement,
    },
}

enum ModifiersKeyState {
    Unknown,
    Pressed,
}

/// The modifiers in effect, and which side of each is held.
enum Modifiers {
    State {
        shift: bool,
        control: bool,
        alt: bool,
        super_key: bool,
        left_shift: Enum<ModifiersKeyState>,
        right_shift: Enum<ModifiersKeyState>,
        left_control: Enum<ModifiersKeyState>,
        right_control: Enum<ModifiersKeyState>,
        left_alt: Enum<ModifiersKeyState>,
        right_alt: Enum<ModifiersKeyState>,
        left_super: Enum<ModifiersKeyState>,
        right_super: Enum<ModifiersKeyState>,
    },
}

/// A device's raw input, whichever window has focus.
enum DeviceEvent {
    Added,
    Removed,
    MouseMotion { x: f64, y: f64 },
    MouseWheel { delta: MouseScrollDelta },
    Motion { axis: i64, value: f64 },
    Button { button: i64, state: Enum<MouseElementState> },
    Key {
        physical_key: PhysicalKey,
        state: Enum<MouseElementState>,
    },
}

/// What `Window.poll` and `Window.wait` return: the next event, or `None`.
/// `device_id` is a positive id per input device. Raw device events go to
/// the focused window, or the first one open when none has focus.
enum Event {
    None,
    /// The window is asked to close. It stays open until `close`.
    Closed,
    Destroyed,
    Resized { width: i64, height: i64 },
    Moved { x: i32, y: i32 },
    Focused { focused: bool },
    Occluded { occluded: bool },
    ScaleFactorChanged { scale_factor: f64 },
    ThemeChanged { theme: Enum<Theme> },
    RedrawRequested,
    CursorEntered { device_id: i32 },
    CursorLeft { device_id: i32 },
    CursorMoved { x: f64, y: f64, device_id: i32 },
    MouseInput {
        state: Enum<MouseElementState>,
        button: MouseButton,
        device_id: i32,
    },
    MouseWheel {
        delta: MouseScrollDelta,
        phase: Enum<TouchPhase>,
        device_id: i32,
    },
    KeyboardInput {
        device_id: i32,
        event: KeyEvent,
        is_synthetic: bool,
    },
    ModifiersChanged { modifiers: Modifiers },
    Ime { event: Ime },
    DroppedFile { path: FilePath },
    HoveredFile { path: FilePath },
    HoveredFileCancelled,
    PinchGesture {
        device_id: i32,
        delta: f64,
        phase: Enum<TouchPhase>,
    },
    PanGesture {
        device_id: i32,
        x: f64,
        y: f64,
        phase: Enum<TouchPhase>,
    },
    DoubleTapGesture { device_id: i32 },
    RotationGesture {
        device_id: i32,
        delta: f64,
        phase: Enum<TouchPhase>,
    },
    TouchpadPressure {
        device_id: i32,
        pressure: f64,
        stage: i64,
    },
    AxisMotion {
        device_id: i32,
        axis: i64,
        value: f64,
    },
    /// All 64 bits of the touch's id.
    Touch {
        device_id: i32,
        phase: Enum<TouchPhase>,
        x: f64,
        y: f64,
        force: TouchForce,
        id: i64,
    },
    /// The token for the request `requestActivationToken` numbered `serial`.
    ActivationTokenDone { serial: i64, token: Text },
    Device { device_id: i32, event: DeviceEvent },
    Resumed,
    Suspended,
    MemoryWarning,
}

/// What a window opens with; whatever is left unset is the platform's
/// default. Sizes and positions are logical.
struct WindowAttributes {
    title: Option<Text>,
    width: Option<i32>,
    height: Option<i32>,
    minWidth: Option<i32>,
    minHeight: Option<i32>,
    maxWidth: Option<i32>,
    maxHeight: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
    resizable: Option<bool>,
    maximized: Option<bool>,
    visible: Option<bool>,
    decorations: Option<bool>,
    transparent: Option<bool>,
    blur: Option<bool>,
    contentProtected: Option<bool>,
    fullscreen: Option<bool>,
    active: Option<bool>,
    windowLevel: Option<Enum<WindowLevel>>,
    theme: Option<Enum<Theme>>,
    scaleSizing: Option<Enum<ScaleSizing>>,
    /// RGBA pixels, `iconWidth` by `iconHeight`.
    icon: Option<Buffer>,
    iconWidth: Option<i32>,
    iconHeight: Option<i32>,
    resizeIncrementWidth: Option<i32>,
    resizeIncrementHeight: Option<i32>,
    closeButton: Option<bool>,
    minimizeButton: Option<bool>,
    maximizeButton: Option<bool>,
}

trait Window {
    /// A new window; one whose `valid` is false when the platform refuses.
    #[native(window_open)]
    fn open(attributes: &WindowAttributes) -> Box<Window>;
    #[native(window_valid)]
    fn valid(this: &Window) -> bool;
    /// When raw device events come, for every window: by default only while
    /// one of the program's windows has focus.
    #[native(window_listen_device_events)]
    fn listenDeviceEvents(when: Enum<DeviceEvents>);
    /// The clipboard's text: none when it holds no text, no window is open,
    /// or the platform has no clipboard to read, as a page cannot.
    #[native(window_clipboard_text)]
    fn clipboardText() -> OptionalText;
    /// Puts text on the clipboard; nothing happens with no window open.
    #[native(window_set_clipboard_text)]
    fn setClipboardText(text: Text);
    /// How many types the clipboard holds now; `clipboardType` names each,
    /// best first, as a MIME type. `text/plain` is UTF-8 text and
    /// `text/uri-list` a list of files.
    #[native(window_clipboard_type_count)]
    fn clipboardTypeCount() -> i32;
    #[native(window_clipboard_type)]
    fn clipboardType(index: i32) -> Text;
    /// The clipboard's bytes for a MIME type, or none when it does not hold
    /// it, no window is open, or the platform cannot read it.
    #[native(window_clipboard_data)]
    fn clipboardData(mimeType: Text) -> OptionalBytes;
    /// The next event, without waiting. It asks the platform for more only
    /// when the window has none queued and has answered none since the
    /// platform was last asked, so draining with polls asks once.
    #[native(window_poll)]
    fn poll(this: &Window) -> Event;
    /// The next event, waiting up to `timeout` seconds for one; a negative
    /// timeout waits as long as it takes.
    #[native(window_wait)]
    fn wait(this: &Window, timeout: f64) -> Event;
    /// Close the window; its handle names nothing after.
    #[native(window_close)]
    fn close(this: &Window);

    // What the window is: physical pixels.
    #[native(window_width)]
    fn width(this: &Window) -> i32;
    #[native(window_height)]
    fn height(this: &Window) -> i32;
    #[native(window_outer_width)]
    fn outerWidth(this: &Window) -> i32;
    #[native(window_outer_height)]
    fn outerHeight(this: &Window) -> i32;
    #[native(window_x)]
    fn x(this: &Window) -> i32;
    #[native(window_y)]
    fn y(this: &Window) -> i32;
    /// Where the window's content is, without its frame.
    #[native(window_inner_x)]
    fn innerX(this: &Window) -> i32;
    #[native(window_inner_y)]
    fn innerY(this: &Window) -> i32;
    #[native(window_scale_factor)]
    fn scaleFactor(this: &Window) -> f64;
    #[native(window_title)]
    fn title(this: &Window) -> Text;
    #[native(window_has_focus)]
    fn hasFocus(this: &Window) -> bool;
    #[native(window_is_visible)]
    fn isVisible(this: &Window) -> bool;
    #[native(window_is_minimized)]
    fn isMinimized(this: &Window) -> bool;
    #[native(window_is_maximized)]
    fn isMaximized(this: &Window) -> bool;
    #[native(window_is_fullscreen)]
    fn isFullscreen(this: &Window) -> bool;
    #[native(window_is_resizable)]
    fn isResizable(this: &Window) -> bool;
    #[native(window_is_decorated)]
    fn isDecorated(this: &Window) -> bool;
    #[native(window_theme)]
    fn theme(this: &Window) -> Enum<Theme>;

    /// The window for a GPU surface: a platform code, and with `raw` the
    /// four integers xgpu's `GpuInstance.surface` takes after it. See
    /// xwindow-core's `raw` for the table.
    #[native(window_platform)]
    fn platform(this: &Window) -> i32;
    #[native(window_raw)]
    fn raw(this: &Window, which: i32) -> i64;

    // What a program asks of the window: logical sizes and positions.
    #[native(window_set_title)]
    fn setTitle(this: &Window, title: Text);
    #[native(window_set_size)]
    fn setSize(this: &Window, width: i32, height: i32);
    /// A size of zero by zero removes the limit.
    #[native(window_set_min_size)]
    fn setMinSize(this: &Window, width: i32, height: i32);
    #[native(window_set_max_size)]
    fn setMaxSize(this: &Window, width: i32, height: i32);
    #[native(window_set_position)]
    fn setPosition(this: &Window, x: i32, y: i32);
    #[native(window_set_resizable)]
    fn setResizable(this: &Window, resizable: bool);
    #[native(window_set_minimized)]
    fn setMinimized(this: &Window, minimized: bool);
    #[native(window_set_maximized)]
    fn setMaximized(this: &Window, maximized: bool);
    #[native(window_set_fullscreen)]
    fn setFullscreen(this: &Window, fullscreen: bool);
    #[native(window_set_decorations)]
    fn setDecorations(this: &Window, decorations: bool);
    #[native(window_set_visible)]
    fn setVisible(this: &Window, visible: bool);
    #[native(window_set_window_level)]
    fn setWindowLevel(this: &Window, level: Enum<WindowLevel>);
    #[native(window_set_transparent)]
    fn setTransparent(this: &Window, transparent: bool);
    #[native(window_set_blur)]
    fn setBlur(this: &Window, blur: bool);
    #[native(window_set_content_protected)]
    fn setContentProtected(this: &Window, protected: bool);
    #[native(window_set_scale_sizing)]
    fn setScaleSizing(this: &Window, sizing: Enum<ScaleSizing>);
    /// A theme, or none to follow the system's.
    #[native(window_set_theme)]
    fn setTheme(this: &Window, theme: Option<Enum<Theme>>);
    /// RGBA pixels, `width` by `height`; an empty buffer removes the icon.
    #[native(window_set_icon)]
    fn setIcon(this: &Window, rgba: Buffer, width: i32, height: i32);
    /// The steps a user resizes the window by; zero by zero removes them.
    #[native(window_set_resize_increments)]
    fn setResizeIncrements(this: &Window, width: i32, height: i32);
    #[native(window_set_enabled_buttons)]
    fn setEnabledButtons(this: &Window, close: bool, minimize: bool, maximize: bool);
    /// Fullscreen in a video mode of one of the window's monitors.
    #[native(window_set_exclusive_fullscreen)]
    fn setExclusiveFullscreen(this: &Window, mode: &VideoMode);
    /// Move the window with the pointer while a button is held, as from an
    /// undecorated window's title bar; whether the platform started it.
    #[native(window_drag)]
    fn dragWindow(this: &Window) -> bool;
    #[native(window_drag_resize)]
    fn dragResizeWindow(this: &Window, direction: Enum<ResizeDirection>) -> bool;
    /// The system's window menu, at a logical position in the window.
    #[native(window_show_menu)]
    fn showWindowMenu(this: &Window, x: f64, y: f64);
    #[native(window_request_redraw)]
    fn requestRedraw(this: &Window);
    /// Call just before presenting a frame. On Wayland the next
    /// `RedrawRequested` then waits for the compositor's frame callback for
    /// that frame, which paces drawing to the display; elsewhere it does
    /// nothing.
    #[native(window_pre_present_notify)]
    fn prePresentNotify(this: &Window);
    #[native(window_focus)]
    fn focus(this: &Window);
    #[native(window_request_attention)]
    fn requestAttention(this: &Window, attention: Enum<Attention>);

    #[native(window_set_cursor_icon)]
    fn setCursorIcon(this: &Window, icon: Enum<CursorIcon>);
    /// A cursor of `width` by `height` RGBA pixels, its hot spot at
    /// `hotX`, `hotY`.
    #[native(window_set_cursor_image)]
    fn setCursorImage(this: &Window, rgba: Buffer, width: i32, height: i32, hotX: i32, hotY: i32);
    #[native(window_set_cursor_visible)]
    fn setCursorVisible(this: &Window, visible: bool);
    /// Whether the platform took the grab.
    #[native(window_set_cursor_grab)]
    fn setCursorGrab(this: &Window, grab: Enum<CursorGrab>) -> bool;
    /// Whether pointer input reaches the window rather than passing
    /// through it; whether the platform took the setting.
    #[native(window_set_cursor_hittest)]
    fn setCursorHittest(this: &Window, hittest: bool) -> bool;
    /// Whether the platform moved the cursor.
    #[native(window_set_cursor_position)]
    fn setCursorPosition(this: &Window, x: f64, y: f64) -> bool;

    #[native(window_set_ime_allowed)]
    fn setImeAllowed(this: &Window, allowed: bool);
    #[native(window_set_ime_purpose)]
    fn setImePurpose(this: &Window, purpose: Enum<ImePurpose>);
    #[native(window_set_ime_cursor_area)]
    fn setImeCursorArea(this: &Window, x: f64, y: f64, width: f64, height: f64);
    /// A request's serial, which its `ActivationTokenDone` carries; zero
    /// where the platform has no activation tokens.
    #[native(window_request_activation_token)]
    fn requestActivationToken(this: &Window) -> i64;

    /// A monitor whose `valid` is false when there is none.
    #[native(window_current_monitor)]
    fn currentMonitor(this: &Window) -> Box<Monitor>;
    #[native(window_primary_monitor)]
    fn primaryMonitor(this: &Window) -> Box<Monitor>;
    #[native(window_monitor_count)]
    fn monitorCount(this: &Window) -> i32;
    #[native(window_monitor)]
    fn monitor(this: &Window, index: i32) -> Box<Monitor>;
}

/// Representations of one thing, by MIME type, which `write` puts on the
/// clipboard together so that a reader takes the richest it understands:
/// `text/plain` beside `text/html`, or `image/png` beside `text/plain`.
trait ClipboardItems {
    #[native(clipboard_items_create)]
    fn create() -> Box<ClipboardItems>;
    #[native(clipboard_items_add)]
    fn add(this: &ClipboardItems, mimeType: Text, bytes: Buffer);
    /// Replaces the clipboard's contents with the items; whether the
    /// platform took them. The handle names nothing after.
    #[native(clipboard_items_write)]
    fn write(this: &ClipboardItems) -> bool;
}

/// A display. The same display is the same handle.
trait Monitor {
    #[native(monitor_valid)]
    fn valid(this: &Monitor) -> bool;
    #[native(monitor_name)]
    fn name(this: &Monitor) -> Text;
    #[native(monitor_width)]
    fn width(this: &Monitor) -> i32;
    #[native(monitor_height)]
    fn height(this: &Monitor) -> i32;
    #[native(monitor_x)]
    fn x(this: &Monitor) -> i32;
    #[native(monitor_y)]
    fn y(this: &Monitor) -> i32;
    #[native(monitor_scale_factor)]
    fn scaleFactor(this: &Monitor) -> f64;
    /// Millihertz; zero when the platform does not say.
    #[native(monitor_refresh_rate)]
    fn refreshRate(this: &Monitor) -> i32;
    #[native(monitor_video_mode_count)]
    fn videoModeCount(this: &Monitor) -> i32;
    /// A video mode whose `valid` is false past the last.
    #[native(monitor_video_mode)]
    fn videoMode(this: &Monitor, index: i32) -> Box<VideoMode>;
}

/// A size, depth and refresh rate a monitor can run at, for exclusive
/// fullscreen. The same mode is the same handle.
trait VideoMode {
    #[native(video_mode_valid)]
    fn valid(this: &VideoMode) -> bool;
    #[native(video_mode_width)]
    fn width(this: &VideoMode) -> i32;
    #[native(video_mode_height)]
    fn height(this: &VideoMode) -> i32;
    #[native(video_mode_bit_depth)]
    fn bitDepth(this: &VideoMode) -> i32;
    /// Millihertz.
    #[native(video_mode_refresh_rate)]
    fn refreshRate(this: &VideoMode) -> i32;
    #[native(video_mode_monitor)]
    fn monitor(this: &VideoMode) -> Box<Monitor>;
}
