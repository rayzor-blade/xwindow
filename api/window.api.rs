// The `window` API every xwindow adapter exposes: windows, their events and
// the monitors they are on. Positions and sizes a window reports are in
// physical pixels; sizes and positions a program asks for are logical, as
// winit takes them. xwindow-bindgen appends `KeyCode`, `Key` and
// `CursorIcon` from xwindow-core's lists.

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

enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other,
}

enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Lines and pages of text, or pixels.
enum ScrollUnit {
    Line,
    Pixel,
}

enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
}

/// What `Window.poll` and `Window.wait` return: the next event, or `None`.
/// `device` is a positive id per input device, the same for a device's
/// window and raw events.
enum Event {
    None,
    /// The window is asked to close. It stays open until `close`.
    CloseRequested,
    Destroyed,
    Resized {
        width: i32,
        height: i32,
    },
    Moved {
        x: i32,
        y: i32,
    },
    Focused {
        focused: bool,
    },
    Occluded {
        occluded: bool,
    },
    ScaleFactorChanged {
        scaleFactor: f64,
    },
    ThemeChanged {
        theme: Enum<Theme>,
    },
    RedrawRequested,
    CursorEntered {
        device: i32,
    },
    CursorLeft {
        device: i32,
    },
    CursorMoved {
        device: i32,
        x: f64,
        y: f64,
    },
    /// `code` is the platform's number for an `Other` button.
    MouseInput {
        device: i32,
        button: Enum<MouseButton>,
        code: i32,
        pressed: bool,
    },
    /// Positive `y` scrolls the content down the window, as winit reports.
    MouseWheel {
        device: i32,
        unit: Enum<ScrollUnit>,
        x: f64,
        y: f64,
        phase: Enum<TouchPhase>,
    },
    /// `code` is the physical key, with the platform's `scancode` when
    /// it is `Unidentified`. `key` is the logical key; for `Character` and
    /// `Dead` its text is `character`. `text` is what the press types.
    KeyboardInput {
        device: i32,
        code: Enum<KeyCode>,
        scancode: i32,
        key: Enum<Key>,
        character: Text,
        text: Text,
        location: Enum<KeyLocation>,
        pressed: bool,
        repeat: bool,
        synthetic: bool,
    },
    ModifiersChanged {
        shift: bool,
        control: bool,
        alt: bool,
        superKey: bool,
    },
    ImeEnabled,
    /// `start` and `end` are UTF-8 byte offsets of the cursor in `text`,
    /// or -1 with no cursor.
    ImePreedit {
        text: Text,
        start: i32,
        end: i32,
    },
    ImeCommit {
        text: Text,
    },
    ImeDisabled,
    /// A path as Unicode; in a page, the file's name.
    HoveredFile {
        path: Text,
    },
    DroppedFile {
        path: Text,
    },
    HoveredFileCancelled,
    /// `force` is normalized to 0..1, or -1 when the device reports none.
    Touch {
        device: i32,
        id: i64,
        phase: Enum<TouchPhase>,
        x: f64,
        y: f64,
        force: f64,
    },
    PinchGesture {
        device: i32,
        delta: f64,
        phase: Enum<TouchPhase>,
    },
    PanGesture {
        device: i32,
        x: f64,
        y: f64,
        phase: Enum<TouchPhase>,
    },
    RotationGesture {
        device: i32,
        delta: f64,
        phase: Enum<TouchPhase>,
    },
    DoubleTapGesture {
        device: i32,
    },
    TouchpadPressure {
        device: i32,
        pressure: f64,
        stage: i64,
    },
    AxisMotion {
        device: i32,
        axis: i32,
        value: f64,
    },
    /// The token for the request `requestActivationToken` numbered `serial`.
    ActivationTokenDone {
        serial: i64,
        token: Text,
    },
    Resumed,
    Suspended,
    MemoryWarning,
    // Raw device events, which go to the focused window, or the first one
    // open when none has focus.
    DeviceAdded {
        device: i32,
    },
    DeviceRemoved {
        device: i32,
    },
    MouseMotion {
        device: i32,
        x: f64,
        y: f64,
    },
    DeviceWheel {
        device: i32,
        unit: Enum<ScrollUnit>,
        x: f64,
        y: f64,
    },
    DeviceAxis {
        device: i32,
        axis: i32,
        value: f64,
    },
    DeviceButton {
        device: i32,
        button: i32,
        pressed: bool,
    },
    DeviceKey {
        device: i32,
        code: Enum<KeyCode>,
        scancode: i32,
        pressed: bool,
    },
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
}

trait Window {
    /// A new window; one whose `valid` is false when the platform refuses.
    #[native(window_open)]
    fn open(attributes: &WindowAttributes) -> Box<Window>;
    #[native(window_valid)]
    fn valid(this: &Window) -> bool;
    /// The next event, without waiting.
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
    #[native(window_request_redraw)]
    fn requestRedraw(this: &Window);
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
    /// Whether the platform moved the cursor.
    #[native(window_set_cursor_position)]
    fn setCursorPosition(this: &Window, x: f64, y: f64) -> bool;

    #[native(window_set_ime_allowed)]
    fn setImeAllowed(this: &Window, allowed: bool);
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
}
