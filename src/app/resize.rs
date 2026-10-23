use super::*;
use slint::winit_030::winit::{dpi::PhysicalSize, window::CursorIcon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResizeEdge {
    horizontal: i8,
    vertical: i8,
}

impl ResizeEdge {
    fn at(position: (f64, f64), size: (u32, u32), border: f64) -> Option<Self> {
        let (x, y) = position;
        let (width, height) = (f64::from(size.0), f64::from(size.1));
        if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. || x >= width || y >= height {
            return None;
        }
        let horizontal = if x < border {
            -1
        } else if x >= width - border {
            1
        } else {
            0
        };
        let vertical = if y < border {
            -1
        } else if y >= height - border {
            1
        } else {
            0
        };
        (horizontal != 0 || vertical != 0).then_some(Self {
            horizontal,
            vertical,
        })
    }

    fn cursor(self) -> CursorIcon {
        match (self.horizontal, self.vertical) {
            (-1, -1) | (1, 1) => CursorIcon::NwseResize,
            (-1, 1) | (1, -1) => CursorIcon::NeswResize,
            (0, _) => CursorIcon::NsResize,
            _ => CursorIcon::EwResize,
        }
    }
}

pub(super) struct WindowResize {
    edge: ResizeEdge,
    pointer: (f64, f64),
    origin: (i32, i32),
    size: (u32, u32),
    minimum: (u32, u32),
    rebase: bool,
}

impl WindowResize {
    fn bounds(&self, pointer: (f64, f64)) -> ((i32, i32), (u32, u32)) {
        fn axis(origin: i32, size: u32, minimum: u32, edge: i8, delta: f64) -> (i32, u32) {
            if edge == 0 {
                return (origin, size);
            }
            let delta = delta
                .round()
                .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i64;
            let next = (i64::from(size) + i64::from(edge) * delta)
                .clamp(i64::from(minimum), i64::from(i32::MAX));
            let origin = i64::from(origin) + if edge < 0 { i64::from(size) - next } else { 0 };
            (
                origin.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                next as u32,
            )
        }
        let (x, width) = axis(
            self.origin.0,
            self.size.0,
            self.minimum.0,
            self.edge.horizontal,
            pointer.0 - self.pointer.0,
        );
        let (y, height) = axis(
            self.origin.1,
            self.size.1,
            self.minimum.1,
            self.edge.vertical,
            pointer.1 - self.pointer.1,
        );
        ((x, y), (width, height))
    }
}

impl App {
    pub(super) fn resize_edge(&self) -> Option<ResizeEdge> {
        let ui = self.ui.upgrade()?;
        if ui.get_fullscreen() {
            return None;
        }
        let position = self.cursor_physical?;
        ui.window()
            .with_winit_window(|window| {
                if window.is_maximized() {
                    return None;
                }
                let size = window.inner_size();
                ResizeEdge::at(
                    position,
                    (size.width, size.height),
                    6. * window.scale_factor(),
                )
            })
            .flatten()
    }

    pub(super) fn resize_cursor(&self, edge: Option<ResizeEdge>) {
        if let Some(ui) = self.ui.upgrade() {
            ui.window().with_winit_window(|window| {
                window.set_cursor(edge.map_or(CursorIcon::Default, ResizeEdge::cursor))
            });
        }
    }

    pub(super) fn begin_window_resize(&mut self, edge: ResizeEdge) {
        let (Some(ui), Some(pointer)) = (self.ui.upgrade(), self.cursor_physical) else {
            return;
        };
        self.window_resize = ui
            .window()
            .with_winit_window(|window| {
                let inner = window.inner_position().ok()?;
                let outer = window.outer_position().ok()?;
                let size = window.inner_size();
                let dpi = window.scale_factor();
                Some(WindowResize {
                    edge,
                    pointer: (
                        f64::from(inner.x) + pointer.0,
                        f64::from(inner.y) + pointer.1,
                    ),
                    origin: (outer.x, outer.y),
                    size: (size.width, size.height),
                    minimum: ((640. * dpi).ceil() as u32, (420. * dpi).ceil() as u32),
                    rebase: false,
                })
            })
            .flatten();
        self.drag = None;
        self.window_drag = None;
    }

    pub(super) fn resize_scale_changed(&mut self, dpi: f64) {
        if let Some(resize) = self.window_resize.as_mut() {
            resize.minimum = ((640. * dpi).ceil() as u32, (420. * dpi).ceil() as u32);
            // Let winit apply the new monitor's DPI bounds, then continue the
            // drag from those bounds rather than undoing them on the next move.
            resize.rebase = true;
        }
    }

    pub(super) fn resize_window(&mut self, pointer: (f64, f64)) {
        let (Some(ui), Some(resize)) = (self.ui.upgrade(), self.window_resize.as_mut()) else {
            return;
        };
        // Apply bounds without entering Windows' modal resize loop, which would
        // block Slint's video presentation while the user holds a corner.
        ui.window().with_winit_window(|window| {
            let Ok(inner) = window.inner_position() else {
                return;
            };
            let pointer = (
                f64::from(inner.x) + pointer.0,
                f64::from(inner.y) + pointer.1,
            );
            if resize.rebase {
                let Ok(outer) = window.outer_position() else {
                    return;
                };
                let size = window.inner_size();
                resize.origin = (outer.x, outer.y);
                resize.size = (size.width, size.height);
                resize.pointer = pointer;
                resize.rebase = false;
            }
            let (origin, size) = resize.bounds(pointer);
            let _ = window.request_inner_size(PhysicalSize::new(size.0, size.1));
            if window.outer_position().ok() != Some(PhysicalPosition::new(origin.0, origin.1)) {
                window.set_outer_position(PhysicalPosition::new(origin.0, origin.1));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_edges_are_reachable_without_taking_the_canvas() {
        for (point, axes) in [
            ((0., 0.), (-1, -1)),
            ((400., 0.), (0, -1)),
            ((799., 0.), (1, -1)),
            ((0., 300.), (-1, 0)),
            ((799., 300.), (1, 0)),
            ((0., 599.), (-1, 1)),
            ((400., 599.), (0, 1)),
            ((799., 599.), (1, 1)),
        ] {
            let edge = ResizeEdge::at(point, (800, 600), 6.).unwrap();
            assert_eq!((edge.horizontal, edge.vertical), axes);
        }
        for point in [
            (6., 6.),
            (400., 300.),
            (-1., 30.),
            (800., 30.),
            (f64::NAN, 0.),
        ] {
            assert!(ResizeEdge::at(point, (800, 600), 6.).is_none());
        }
        assert!(ResizeEdge::at((8., 20.), (1600, 1200), 12.).is_some());
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_fixed_at_minimum_size() {
        let resize = WindowResize {
            edge: ResizeEdge {
                horizontal: -1,
                vertical: -1,
            },
            pointer: (-1000., 100.),
            origin: (-1000, 100),
            size: (1000, 800),
            minimum: (640, 420),
            rebase: false,
        };
        assert_eq!(resize.bounds((-900., 200.)), ((-900, 200), (900, 700)));
        assert_eq!(resize.bounds((2000., 3000.)), ((-640, 480), (640, 420)));
        let grow = WindowResize {
            edge: ResizeEdge {
                horizontal: 1,
                vertical: 1,
            },
            ..resize
        };
        assert_eq!(grow.bounds((-800., 300.)), ((-1000, 100), (1200, 1000)));
    }
}
