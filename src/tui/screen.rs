//! The game picture: a PNG for terminals that draw pixels (Kitty graphics),
//! half-blocks for the rest.

use base64::Engine;
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use vostok_gameboy::frame::Frame;

/// `frame` enlarged `scale` times with hard pixel edges, as base64 PNG ready
/// for `pixels::Presenter`. The terminal then fits it to the cells it is
/// given; sending it already enlarged keeps that step from blurring it.
pub fn png(frame: &Frame, scale: usize) -> String {
    let scale = scale.max(1);
    let (w, h) = (frame.width * scale, frame.height * scale);
    let mut row = Vec::with_capacity(w * 3);
    let mut data = Vec::with_capacity(w * h * 3);
    for y in 0..frame.height {
        row.clear();
        for pixel in &frame.pixels[y * frame.width..(y + 1) * frame.width] {
            for _ in 0..scale {
                row.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
            }
        }
        for _ in 0..scale {
            data.extend_from_slice(&row);
        }
    }
    let mut out = Vec::with_capacity(16 * 1024);
    let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    // Sixty of these a second go through the terminal: speed over size.
    encoder.set_compression(png::Compression::Fast);
    let written = encoder.write_header().and_then(|mut writer| {
        writer.write_image_data(&data)?;
        writer.finish()
    });
    if written.is_err() {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD.encode(out)
}

/// Draw `frame` into `area` with ▀: two picture rows per cell, each sampled
/// from the nearest pixel.
pub fn half_blocks(buf: &mut Buffer, area: Rect, frame: &Frame) {
    let area = area.intersection(buf.area);
    if area.is_empty() || frame.width == 0 || frame.height == 0 {
        return;
    }
    let (w, h) = (usize::from(area.width), usize::from(area.height) * 2);
    let sample = |x: usize, y: usize| {
        let pixel = frame.get_pixel(
            (x * 2 + 1) * frame.width / (w * 2),
            (y * 2 + 1) * frame.height / (h * 2),
        );
        Color::Rgb(pixel.r, pixel.g, pixel.b)
    };
    for cy in 0..usize::from(area.height) {
        for cx in 0..w {
            let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
            cell.reset();
            cell.set_symbol("▀")
                .set_fg(sample(cx, cy * 2))
                .set_bg(sample(cx, cy * 2 + 1));
        }
    }
}

/// The largest rectangle inside `area` with the picture's shape, for cells
/// twice as tall as wide (10:9 pixels is 20:9 cells), centred.
pub fn fit(area: Rect) -> Rect {
    let width = area
        .width
        .min((u32::from(area.height) * 20 / 9) as u16)
        .max(1);
    let height = ((u32::from(width) * 9 + 10) / 20).clamp(1, u32::from(area.height.max(1))) as u16;
    let (dx, dy) = (
        area.width.saturating_sub(width) / 2,
        area.height.saturating_sub(height) / 2,
    );
    Rect::new(area.x + dx, area.y + dy, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vostok_gameboy::frame::Rgb;

    #[test]
    fn the_png_is_enlarged_with_hard_edges() {
        let mut frame = Frame::new(2, 1);
        frame.set_pixel(0, Rgb::new(255, 0, 0));
        frame.set_pixel(1, Rgb::new(0, 0, 255));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png(&frame, 3))
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (6, 3));
        assert_eq!(image.get_pixel(2, 2).0, [255, 0, 0]);
        assert_eq!(image.get_pixel(3, 0).0, [0, 0, 255]);
    }

    #[test]
    fn half_blocks_put_two_rows_in_a_cell() {
        let mut frame = Frame::new(2, 2);
        for (index, shade) in [10u8, 20, 30, 40].into_iter().enumerate() {
            frame.set_pixel(index, Rgb::new(shade, shade, shade));
        }
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        half_blocks(&mut buf, area, &frame);
        assert_eq!(buf[(1, 0)].symbol(), "▀");
        assert_eq!(
            (buf[(1, 0)].fg, buf[(1, 0)].bg),
            (Color::Rgb(20, 20, 20), Color::Rgb(40, 40, 40))
        );
    }

    #[test]
    fn fit_keeps_the_picture_shape() {
        assert_eq!(fit(Rect::new(0, 0, 160, 72)), Rect::new(0, 0, 160, 72));
        assert_eq!(fit(Rect::new(0, 0, 100, 10)), Rect::new(39, 0, 22, 10));
        assert_eq!(fit(Rect::new(0, 0, 40, 30)), Rect::new(0, 6, 40, 18));
    }
}
