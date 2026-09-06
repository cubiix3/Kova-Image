use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Previous,
    Next,
    First,
    Last,
    ZoomIn,
    ZoomOut,
    Fit,
    FitWidth,
    Actual,
    Fullscreen,
    Escape,
    Pause,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    CopyImage,
    CopyPath,
    Delete,
    Info,
    Settings,
    Reveal,
    OpenWith,
    Minimize,
    Maximize,
    Close,
}
pub fn shortcut(key: &Key, modifiers: ModifiersState) -> Option<Action> {
    use Action::*;
    if modifiers.alt_key() || modifiers.super_key() {
        return None;
    }
    if modifiers.control_key() {
        return match key {
            Key::Character(s) if s.eq_ignore_ascii_case("o") => Some(Open),
            Key::Character(s) if s.eq_ignore_ascii_case("c") => Some(if modifiers.shift_key() {
                CopyPath
            } else {
                CopyImage
            }),
            _ => None,
        };
    }
    match key {
        Key::Named(k) => match k {
            NamedKey::ArrowLeft => Some(Previous),
            NamedKey::ArrowRight => Some(Next),
            NamedKey::Home => Some(First),
            NamedKey::End => Some(Last),
            NamedKey::F11 => Some(Fullscreen),
            NamedKey::Escape => Some(Escape),
            NamedKey::Space => Some(Pause),
            NamedKey::Delete => Some(Delete),
            _ => None,
        },
        Key::Character(s) => match s.to_lowercase().as_str() {
            "+" | "=" => Some(ZoomIn),
            "-" => Some(ZoomOut),
            "0" => Some(Fit),
            "1" => Some(Actual),
            "w" => Some(FitWidth),
            "r" => Some(if modifiers.shift_key() {
                RotateLeft
            } else {
                RotateRight
            }),
            "h" => Some(FlipHorizontal),
            "v" => Some(FlipVertical),
            "i" => Some(Info),
            _ => None,
        },
        _ => None,
    }
}
pub fn command(name: &str) -> Option<Action> {
    use Action::*;
    Some(match name {
        "open" => Open,
        "previous" => Previous,
        "next" => Next,
        "first" => First,
        "last" => Last,
        "in" => ZoomIn,
        "out" => ZoomOut,
        "fit" => Fit,
        "width" => FitWidth,
        "actual" => Actual,
        "fullscreen" => Fullscreen,
        "pause" => Pause,
        "left" => RotateLeft,
        "right" => RotateRight,
        "horizontal" => FlipHorizontal,
        "vertical" => FlipVertical,
        "copy" => CopyImage,
        "path" => CopyPath,
        "delete" => Delete,
        "info" => Info,
        "settings" => Settings,
        "reveal" => Reveal,
        "open-with" => OpenWith,
        "minimize" => Minimize,
        "maximize" => Maximize,
        "close" => Close,
        _ => return None,
    })
}
