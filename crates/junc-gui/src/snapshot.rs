// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic snapshots (architecture §11.3, feature `scenarios`): an
//! offscreen Slint platform with the software renderer and a font
//! collection that holds only the bundled snapshot font, so a scenario
//! renders the same pixels on every machine. No display is needed.

use slint::fontique_012::fontique;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{PhysicalSize, PlatformError, Rgb8Pixel};
use std::rc::Rc;

/// The font snapshots render with (`fonts/README.md`).
pub const FONT: &[u8] = include_bytes!("../fonts/JetBrainsMonoNerdFont-JuncSubset.ttf");
/// Its family name, for the `Tokens.font-family` token.
pub const FAMILY: &str = "JetBrainsMono Nerd Font";

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

/// An offscreen window of `width` × `height` pixels at scale 1. Call once
/// per process, before any component is created.
pub fn install(width: u32, height: u32) -> Result<Rc<MinimalSoftwareWindow>, PlatformError> {
    let mut fonts = fontique::Collection::new(fontique::CollectionOptions {
        system_fonts: false,
        ..Default::default()
    });
    let ids = fonts.register_fonts(
        fontique::Blob::new(std::sync::Arc::new(FONT.to_vec())),
        None,
    );
    for generic in [
        fontique::GenericFamily::SansSerif,
        fontique::GenericFamily::Monospace,
        fontique::GenericFamily::SystemUi,
    ] {
        fonts.set_generic_families(generic, ids.iter().map(|(family, _)| *family));
    }
    slint::fontique_012::set_shared_collection(fonts)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone())))
        .map_err(|e| PlatformError::Other(e.to_string()))?;
    window.set_size(PhysicalSize::new(width, height));
    Ok(window)
}

/// Renders the window's current content as RGB bytes, row by row.
/// Lets the window settle before a snapshot: a few frames and timer runs,
/// so that callbacks deferred to the event loop (the canvas reporting its
/// width, the mock IPC answering, the hovered link) have run.
pub fn settle(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    for _ in 0..5 {
        let _ = render(window, width, height);
        slint::platform::update_timers_and_animations();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub fn render(window: &MinimalSoftwareWindow, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut pixels = vec![Rgb8Pixel::new(0, 0, 0); w * h];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, w);
    });
    pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

/// Writes RGB bytes as a PNG.
pub fn write_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgb).map_err(|e| e.to_string())
}

/// Reads a PNG written by [`write_png`]: (width, height, RGB bytes).
pub fn read_png(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgb {
        return Err(format!("{}: not an RGB PNG", path.display()));
    }
    Ok((info.width, info.height, buf))
}

/// How two renders differ: the share of pixels whose channels differ by
/// more than `threshold` (anti-aliasing may differ a little between
/// architectures; a changed layout or colour moves far more).
pub fn difference(a: &[u8], b: &[u8], threshold: u8) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let differing = a
        .as_chunks::<3>()
        .0
        .iter()
        .zip(b.as_chunks::<3>().0)
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .any(|(x, y)| x.abs_diff(*y) > threshold)
        })
        .count();
    // Pixel counts are far below 2^52.
    #[allow(clippy::cast_precision_loss)]
    let share = differing as f64 / (a.len() / 3) as f64;
    share
}
