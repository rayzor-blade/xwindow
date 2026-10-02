//! The cursor icons winit 0.30 names (the `cursor-icon` crate's), which are
//! CSS's `cursor` keywords in kebab case.

/// Every cursor icon, by its name in winit.
#[macro_export]
macro_rules! cursor_icons {
    ($then:ident) => {
        $then! {
            Default, ContextMenu, Help, Pointer, Progress, Wait, Cell, Crosshair, Text,
            VerticalText, Alias, Copy, Move, NoDrop, NotAllowed, Grab, Grabbing, EResize, NResize,
            NeResize, NwResize, SResize, SeResize, SwResize, WResize, EwResize, NsResize,
            NeswResize, NwseResize, ColResize, RowResize, AllScroll, ZoomIn, ZoomOut, DndAsk,
            AllResize,
        }
    };
}

macro_rules! names {
    ($($name:ident),* $(,)?) => {
        &[$(stringify!($name)),*]
    };
}

/// `cursor_icons!`'s names, in order.
pub const CURSOR_ICONS: &[&str] = cursor_icons!(names);

/// The CSS `cursor` keyword for the icon named `name`. CSS has no keyword
/// for `DndAsk` or `AllResize`, so those take their nearest.
pub fn css(name: &str) -> String {
    match name {
        "DndAsk" => return "copy".to_owned(),
        "AllResize" => return "move".to_owned(),
        _ => {}
    }
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn icons_are_css_keywords() {
        assert_eq!(super::css("NeswResize"), "nesw-resize");
        assert_eq!(super::css("EResize"), "e-resize");
        assert_eq!(super::css("Default"), "default");
        assert_eq!(super::css("DndAsk"), "copy");
        assert_eq!(super::CURSOR_ICONS.len(), 36);
    }
}
