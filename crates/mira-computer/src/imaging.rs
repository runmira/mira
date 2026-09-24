//! Screenshot sizing and encoding.
//!
//! The model never sees the physical framebuffer. Every capture is
//! resized to a *model space* that fits the provider's image limits, and
//! every coordinate the model sends back is in that space. [`Scale`]
//! holds the two sizes and converts between them; getting this wrong on
//! HiDPI displays makes every click miss, so it is the one place that
//! does the math.

use base64::Engine;
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};

use crate::action::Point;
use crate::ComputerError;

/// Upper bounds on what a screenshot sent to the model may be.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImageLimits {
    /// Longest edge, in pixels.
    pub max_long_edge: u32,
    /// Total pixel budget (width × height).
    pub max_pixels: u32,
}

impl Default for ImageLimits {
    /// 1568 px long edge and ~1.15 MP, the sizes Anthropic documents as
    /// the no-downscale ceiling across its vision models. Staying under
    /// them means the provider never resizes behind our back, which
    /// would silently break the coordinate mapping.
    fn default() -> Self {
        Self {
            max_long_edge: 1568,
            max_pixels: 1_150_000,
        }
    }
}

impl ImageLimits {
    /// Largest size with the same aspect ratio as `w × h` that fits both
    /// limits. Never upscales.
    pub fn fit(&self, w: u32, h: u32) -> (u32, u32) {
        if w == 0 || h == 0 {
            return (w, h);
        }
        let mut s = 1.0f64;
        let long = w.max(h) as f64;
        if long > self.max_long_edge as f64 {
            s = s.min(self.max_long_edge as f64 / long);
        }
        let px = w as f64 * h as f64;
        if px > self.max_pixels as f64 {
            s = s.min((self.max_pixels as f64 / px).sqrt());
        }
        if s >= 1.0 {
            return (w, h);
        }
        (
            ((w as f64 * s).floor() as u32).max(1),
            ((h as f64 * s).floor() as u32).max(1),
        )
    }
}

/// Mapping between model space (screenshot pixels) and input space (the
/// coordinates the OS takes for mouse events — logical points on macOS).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Scale {
    pub model: (u32, u32),
    pub input: (u32, u32),
}

impl Scale {
    pub fn new(input: (u32, u32), limits: &ImageLimits) -> Self {
        Self {
            model: limits.fit(input.0, input.1),
            input,
        }
    }

    /// Model-space point → input-space point, clamped on-screen.
    pub fn to_input(&self, p: Point) -> Point {
        let conv = |v: i32, m: u32, i: u32| -> i32 {
            if m == 0 {
                return v;
            }
            let scaled = (v as f64 * i as f64 / m as f64).round() as i32;
            scaled.clamp(0, i.saturating_sub(1) as i32)
        };
        Point {
            x: conv(p.x, self.model.0, self.input.0),
            y: conv(p.y, self.model.1, self.input.1),
        }
    }

    /// Input-space point → model-space point (for `cursor_position`).
    pub fn to_model(&self, p: Point) -> Point {
        let conv = |v: i32, m: u32, i: u32| -> i32 {
            if i == 0 {
                return v;
            }
            (v as f64 * m as f64 / i as f64).round() as i32
        };
        Point {
            x: conv(p.x, self.model.0, self.input.0),
            y: conv(p.y, self.model.1, self.input.1),
        }
    }

    /// Reject model-space points outside the last screenshot. A model
    /// that clicks at (3000, 40) on a 1280-wide image is confused, and
    /// clamping would click somewhere it never looked.
    pub fn check_bounds(&self, p: Point) -> Result<(), ComputerError> {
        if p.x < 0 || p.y < 0 || p.x >= self.model.0 as i32 || p.y >= self.model.1 as i32 {
            return Err(ComputerError::InvalidArgs(format!(
                "coordinate ({}, {}) is outside the {}x{} screenshot",
                p.x, p.y, self.model.0, self.model.1
            )));
        }
        Ok(())
    }
}

/// Decode PNG (or any enabled format) bytes into RGBA.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage, ComputerError> {
    let img = image::load_from_memory(bytes).map_err(|e| ComputerError::Image(e.to_string()))?;
    Ok(img.to_rgba8())
}

/// Resize to exactly `w × h` (no-op when already that size).
pub fn resize(img: RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.width() == w && img.height() == h {
        return img;
    }
    image::imageops::resize(&img, w, h, FilterType::Triangle)
}

/// Crop `[x0, y0, x1, y1]` (inclusive-exclusive, already in `img` space).
pub fn crop(img: &RgbaImage, region: [u32; 4]) -> RgbaImage {
    let [x0, y0, x1, y1] = region;
    let x0 = x0.min(img.width().saturating_sub(1));
    let y0 = y0.min(img.height().saturating_sub(1));
    let w = x1.clamp(x0 + 1, img.width()) - x0;
    let h = y1.clamp(y0 + 1, img.height()) - y0;
    image::imageops::crop_imm(img, x0, y0, w, h).to_image()
}

/// Encode as PNG and base64 it (standard alphabet, no prefix).
pub fn png_base64(img: &RgbaImage) -> Result<String, ComputerError> {
    let mut out = std::io::Cursor::new(Vec::new());
    // RGB is ~25% smaller than RGBA and screenshots have no alpha.
    DynamicImage::ImageRgba8(img.clone())
        .to_rgb8()
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| ComputerError::Image(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(out.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_respects_both_limits_and_never_upscales() {
        let l = ImageLimits::default();
        assert_eq!(l.fit(1024, 768), (1024, 768));
        let (w, h) = l.fit(1920, 1080);
        assert!(w * h <= l.max_pixels && w.max(h) <= l.max_long_edge);
        assert!((w as f64 / h as f64 - 16.0 / 9.0).abs() < 0.01);
        let (w, h) = l.fit(3840, 400);
        assert_eq!(w, 1568);
        assert!(h <= 400);
    }

    #[test]
    fn scale_round_trips_hidpi() {
        // Retina-style: 1512x982 logical points, model space smaller.
        let s = Scale::new((1512, 982), &ImageLimits::default());
        let corner = Point {
            x: s.model.0 as i32 - 1,
            y: s.model.1 as i32 - 1,
        };
        let inp = s.to_input(corner);
        assert!(inp.x <= 1511 && inp.x >= 1505, "{inp:?}");
        let mid = s.to_input(Point {
            x: s.model.0 as i32 / 2,
            y: s.model.1 as i32 / 2,
        });
        assert!(
            (mid.x - 756).abs() <= 1 && (mid.y - 491).abs() <= 1,
            "{mid:?}"
        );
        let back = s.to_model(mid);
        assert!((back.x - s.model.0 as i32 / 2).abs() <= 1);
        assert!(s.check_bounds(Point { x: -1, y: 0 }).is_err());
        assert!(s
            .check_bounds(Point {
                x: s.model.0 as i32,
                y: 0
            })
            .is_err());
    }

    #[test]
    fn crop_clamps_to_image() {
        let img = RgbaImage::new(100, 50);
        let c = crop(&img, [90, 40, 500, 500]);
        assert_eq!((c.width(), c.height()), (10, 10));
    }
}
