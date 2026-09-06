#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    #[default]
    Window,
    Width,
    Actual,
    Free,
}
/// At 100%, one source pixel occupies one physical display pixel, even at 150% DPI.
pub fn logical_image_size(width: u32, height: u32, dpi: f32) -> (f32, f32) {
    let dpi = if dpi.is_finite() && dpi > 0.0 {
        dpi
    } else {
        1.0
    };
    (width as f32 / dpi, height as f32 / dpi)
}
#[derive(Debug, Default)]
pub struct View {
    pub fit: Fit,
    pub zoom: f32,
    pub pan: (f32, f32),
    pub rotation: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}
impl View {
    pub fn reset(&mut self, fit: Fit) {
        *self = Self {
            fit,
            zoom: 1.,
            ..Self::default()
        };
    }
    pub fn scale(&self, image: (f32, f32), viewport: (f32, f32)) -> f32 {
        let (w, h) = if self.rotation.rem_euclid(180) == 90 {
            (image.1, image.0)
        } else {
            image
        };
        if w <= 0. || h <= 0. {
            return 1.;
        }
        match self.fit {
            Fit::Window => (viewport.0 / w).min(viewport.1 / h).min(1.),
            Fit::Width => viewport.0 / w,
            Fit::Actual => 1.,
            Fit::Free => self.zoom,
        }
        .clamp(0.001, 64.)
    }
    pub fn zoom_at(
        &mut self,
        factor: f32,
        anchor: (f32, f32),
        image: (f32, f32),
        viewport: (f32, f32),
    ) {
        let old = self.scale(image, viewport);
        let new = (old * factor).clamp(0.001, 64.);
        let ratio = new / old;
        self.pan.0 = anchor.0 - (anchor.0 - self.pan.0) * ratio;
        self.pan.1 = anchor.1 - (anchor.1 - self.pan.1) * ratio;
        self.zoom = new;
        self.fit = Fit::Free;
    }
    pub fn set_fit(&mut self, fit: Fit) {
        self.fit = fit;
        self.pan = (0., 0.);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_size_respects_display_scale() {
        assert_eq!(logical_image_size(1200, 800, 2.0), (600.0, 400.0));
        assert_eq!(logical_image_size(1200, 800, f32::NAN), (1200.0, 800.0));
    }
    #[test]
    fn cursor_anchor_is_stable() {
        let mut v = View::default();
        v.reset(Fit::Actual);
        v.zoom_at(2., (100., 50.), (400., 400.), (800., 600.));
        assert_eq!(v.pan, (-100., -50.));
        assert_eq!(v.zoom, 2.);
    }
    #[test]
    fn rotated_fit_uses_swapped_dimensions() {
        let mut v = View::default();
        v.reset(Fit::Window);
        v.rotation = 90;
        assert_eq!(v.scale((1000., 500.), (600., 400.)), 0.4);
    }
}
