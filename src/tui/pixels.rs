//! Kitty graphics: send a PNG and place it over a cell rectangle (the presenter
//! from vostok-camera, without the video parts).

// Shared file: identical in every Vostok app, so not every item is used here.
#![allow(dead_code)]

use base64::Engine;
use image::DynamicImage;
use ratatui::layout::Rect;
use std::io::{self, Write};

const IDS: [u32; 2] = [0x564f4601, 0x564f4602];

#[derive(Default)]
pub struct Presenter {
    displayed: Option<u32>,
    dirty: bool,
}

impl Presenter {
    /// Remove the picture, if one is on screen.
    pub fn cleanup(&mut self, out: &mut impl Write, tmux: bool) -> io::Result<()> {
        if self.dirty {
            for id in IDS {
                command(out, &format!("a=d,d=I,i={id},q=2"), &[], tmux)?;
            }
            out.flush()?;
            self.displayed = None;
            self.dirty = false;
        }
        Ok(())
    }

    /// Show base64 PNG `data` scaled to `area`. The old picture goes after the new
    /// one is placed, so there is no blank frame.
    pub fn present(
        &mut self,
        out: &mut impl Write,
        data: &str,
        area: Rect,
        tmux: bool,
    ) -> io::Result<()> {
        if data.is_empty() || area.is_empty() {
            return Ok(());
        }
        let id = if self.displayed == Some(IDS[0]) {
            IDS[1]
        } else {
            IDS[0]
        };
        self.dirty = true;
        let chunks = data.as_bytes().chunks(4096);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let more = u8::from(index + 1 < count);
            let header = if index == 0 {
                format!("a=t,f=100,t=d,i={id},q=2,m={more};")
            } else {
                format!("m={more};")
            };
            command(out, &header, chunk, tmux)?;
        }
        crossterm::queue!(out, crossterm::cursor::MoveTo(area.x, area.y))?;
        command(
            out,
            &format!("a=p,i={id},c={},r={},C=1,q=2", area.width, area.height),
            &[],
            tmux,
        )?;
        if let Some(old) = self.displayed {
            command(out, &format!("a=d,d=I,i={old},q=2"), &[], tmux)?;
        }
        out.flush()?;
        self.displayed = Some(id);
        Ok(())
    }
}

/// Numbers for gallery pictures start here, clear of the presenter's two.
const GALLERY: u32 = 0x564f_4700;

/// Several small pictures at once, each over its own cell rectangle (the pictures
/// inside a page). The caller says what should be on screen; this sends, moves and
/// removes pictures to match.
#[derive(Default)]
pub struct Gallery {
    shown: Vec<(u32, Rect)>,
}

impl Gallery {
    /// Make the screen show exactly `wanted`: (number, base64 PNG, cells).
    pub fn sync(
        &mut self,
        out: &mut impl Write,
        wanted: &[(u32, &str, Rect)],
        tmux: bool,
    ) -> io::Result<()> {
        let mut wrote = false;
        for (id, _) in &self.shown {
            if !wanted.iter().any(|w| w.0 == *id) {
                // Gone from view: remove it and free the terminal's copy.
                command(out, &format!("a=d,d=I,i={},q=2", GALLERY + id), &[], tmux)?;
                wrote = true;
            }
        }
        self.shown
            .retain(|(id, _)| wanted.iter().any(|w| w.0 == *id));
        for (id, data, area) in wanted {
            if data.is_empty() || area.is_empty() {
                continue;
            }
            let number = GALLERY + id;
            match self.shown.iter_mut().find(|(shown, _)| shown == id) {
                Some((_, at)) if at == area => continue,
                Some((_, at)) => {
                    // Moved: take the old placement away, keep the picture data.
                    command(out, &format!("a=d,d=i,i={number},q=2"), &[], tmux)?;
                    *at = *area;
                }
                None => {
                    let chunks = data.as_bytes().chunks(4096);
                    let count = chunks.len();
                    for (index, chunk) in chunks.enumerate() {
                        let more = u8::from(index + 1 < count);
                        let header = if index == 0 {
                            format!("a=t,f=100,t=d,i={number},q=2,m={more};")
                        } else {
                            format!("m={more};")
                        };
                        command(out, &header, chunk, tmux)?;
                    }
                    self.shown.push((*id, *area));
                }
            }
            crossterm::queue!(out, crossterm::cursor::MoveTo(area.x, area.y))?;
            command(
                out,
                &format!("a=p,i={number},c={},r={},C=1,q=2", area.width, area.height),
                &[],
                tmux,
            )?;
            wrote = true;
        }
        if wrote {
            out.flush()?;
        }
        Ok(())
    }

    /// Remove every picture (leaving, resizing, a dialog on top).
    pub fn clear(&mut self, out: &mut impl Write, tmux: bool) -> io::Result<()> {
        self.sync(out, &[], tmux)
    }
}

/// True for a picture that is mostly runs of one colour (a page, a diagram):
/// on a sample of rows, most pixels are the same as the one to their left.
fn flat(rgb: &image::RgbImage) -> bool {
    let (mut same, mut seen) = (0u64, 0u64);
    for y in (0..rgb.height()).step_by(4) {
        let mut last = None;
        for x in 0..rgb.width() {
            let pixel = rgb.get_pixel(x, y).0;
            if last == Some(pixel) {
                same += 1;
            }
            last = Some(pixel);
            seen += 1;
        }
    }
    seen > 0 && same * 100 >= seen * 35
}

fn command(out: &mut impl Write, header: &str, payload: &[u8], tmux: bool) -> io::Result<()> {
    out.write_all(if tmux {
        b"\x1bPtmux;\x1b\x1b_G"
    } else {
        b"\x1b_G"
    })?;
    out.write_all(header.as_bytes())?;
    out.write_all(payload)?;
    out.write_all(if tmux { b"\x1b\x1b\\\x1b\\" } else { b"\x1b\\" })
}

/// PNG, base64. Large pictures are reduced first: the terminal scales the placement.
pub fn encode(image: &DynamicImage) -> anyhow::Result<String> {
    use image::{
        ExtendedColorType, ImageEncoder,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };
    let rgb = if image.width() > 1600 || image.height() > 1200 {
        image.thumbnail(1600, 1200).into_rgb8()
    } else {
        image.to_rgb8()
    };
    let mut png = Vec::with_capacity(rgb.as_raw().len() / 2);
    // Drawn pages (flat colour, text) shrink many times over with full compression
    // for a few milliseconds more, and every byte has to go through the terminal.
    // Photographs barely shrink and take twenty times as long: those go fast.
    let level = if flat(&rgb) {
        CompressionType::Default
    } else {
        CompressionType::Fast
    };
    PngEncoder::new_with_quality(&mut png, level, FilterType::Adaptive).write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        ExtendedColorType::Rgb8,
    )?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_sent_placed_and_removed() {
        let image =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 2, image::Rgb([1, 2, 3])));
        let data = encode(&image).unwrap();
        assert!(data.starts_with("iVBOR"));
        let mut out = Vec::new();
        let mut presenter = Presenter::default();
        presenter
            .present(&mut out, &data, Rect::new(3, 4, 10, 5), false)
            .unwrap();
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("\x1b_Ga=t,f=100,t=d,i=1448035841,q=2,m=0;iVBOR"));
        assert!(text.contains("a=p,i=1448035841,c=10,r=5,C=1,q=2"));
        out.clear();
        presenter
            .present(&mut out, &data, Rect::new(3, 4, 10, 5), true)
            .unwrap();
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("\x1bPtmux;\x1b\x1b_Ga=t"));
        assert!(text.contains("a=d,d=I,i=1448035841"));
        out.clear();
        presenter.cleanup(&mut out, false).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("a=d,d=I,i=1448035842"));
        out.clear();
        presenter.cleanup(&mut out, false).unwrap();
        assert!(out.is_empty());
        presenter
            .present(&mut out, "", Rect::new(0, 0, 1, 1), false)
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_page_is_packed_small_and_a_photograph_fast() {
        use image::{Rgb, RgbImage};
        // A page: white with dark lines of "text".
        let page = RgbImage::from_fn(400, 200, |x, y| {
            if y % 20 < 8 && x % 9 < 2 {
                Rgb([20, 20, 30])
            } else {
                Rgb([250, 250, 248])
            }
        });
        // A photograph: no two neighbours alike.
        let mut seed = 7u32;
        let photo = RgbImage::from_fn(400, 200, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
        });
        assert!(flat(&page) && !flat(&photo));
        let packed = encode(&DynamicImage::ImageRgb8(page)).unwrap();
        // 240 000 bytes of pixels come out as a few hundred bytes of text.
        assert!(packed.len() < 4_000, "{}", packed.len());
        assert!(encode(&DynamicImage::ImageRgb8(photo)).is_ok());
    }

    #[test]
    fn a_gallery_sends_once_moves_and_removes() {
        let text = |out: &[u8]| String::from_utf8_lossy(out).into_owned();
        let mut gallery = Gallery::default();
        let mut out = Vec::new();
        let (a, b) = (Rect::new(2, 3, 20, 6), Rect::new(2, 12, 10, 4));
        gallery
            .sync(&mut out, &[(1, "AAAA", a), (2, "BBBB", b)], false)
            .unwrap();
        let first = text(&out);
        assert!(first.contains("a=t,f=100,t=d,i=1448036097,q=2,m=0;AAAA"));
        assert!(first.contains("a=p,i=1448036097,c=20,r=6,C=1,q=2"));
        assert!(first.contains("a=p,i=1448036098,c=10,r=4,C=1,q=2"));
        // Nothing changed: nothing is written.
        out.clear();
        gallery
            .sync(&mut out, &[(1, "AAAA", a), (2, "BBBB", b)], false)
            .unwrap();
        assert!(out.is_empty());
        // One scrolls up a row and the other leaves the screen.
        let moved = Rect::new(2, 2, 20, 6);
        gallery
            .sync(&mut out, &[(1, "AAAA", moved)], false)
            .unwrap();
        let second = text(&out);
        assert!(second.contains("a=d,d=I,i=1448036098"));
        assert!(second.contains("a=d,d=i,i=1448036097"));
        assert!(!second.contains("a=t"), "the data is not sent again");
        assert!(second.contains("a=p,i=1448036097,c=20,r=6"));
        out.clear();
        gallery.clear(&mut out, true).unwrap();
        assert!(text(&out).contains("\x1bPtmux;\x1b\x1b_Ga=d,d=I,i=1448036097"));
        out.clear();
        gallery.clear(&mut out, false).unwrap();
        assert!(out.is_empty());
    }
}
