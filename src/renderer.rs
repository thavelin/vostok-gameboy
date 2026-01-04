use crate::framebuffer::{FrameBuffer, GB_HEIGHT, GB_WIDTH};
use std::io::{self, Write};

/// Screen rows (144/2 rows for the game using half-block characters)
pub const SCREEN_ROWS: u16 = 72;

/// UTF-8 bytes for ▀ (upper half block)
const HALF_BLOCK_UTF8: &[u8] = "▀".as_bytes();

/// ASCII character palette (ordered by brightness, dark to light)
const ASCII_CHARS: &[u8] = b" .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";

#[derive(Clone, Copy, PartialEq)]
pub enum RenderMode {
    Ascii,
    Block,
}

pub struct RenderConfig {
    pub mode: RenderMode,
}

pub struct Renderer {
    config: RenderConfig,
    output_buffer: Vec<u8>,
    last_fg: u8,  // 256-color index
    last_bg: u8,  // 256-color index
    needs_clear: bool,  // Set when terminal needs full clear (e.g., resize)
}

impl Renderer {
    pub fn new(config: RenderConfig) -> Self {
        // Estimate ~12 bytes per cell worst case with 256-color mode
        let buffer_size = GB_WIDTH * (GB_HEIGHT / 2) * 12;
        Self {
            config,
            output_buffer: Vec::with_capacity(buffer_size),
            last_fg: 255, // Invalid sentinel
            last_bg: 255,
            needs_clear: true, // Clear on first frame
        }
    }
    
    /// Mark that the terminal needs a full clear (e.g., after resize)
    pub fn request_clear(&mut self) {
        self.needs_clear = true;
    }

    pub fn render(&mut self, fb: &FrameBuffer, stdout: &mut io::Stdout) -> io::Result<()> {
        match self.config.mode {
            RenderMode::Block => self.render_block(fb, stdout),
            RenderMode::Ascii => self.render_ascii(fb, stdout),
        }
    }

    /// Convert RGB to 256-color palette index (6x6x6 color cube: indices 16-231)
    #[inline]
    fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
        // Map 0-255 to 0-5 for each channel, then to color cube index
        let r6 = (r as u16 * 6 / 256) as u8;
        let g6 = (g as u16 * 6 / 256) as u8;
        let b6 = (b as u16 * 6 / 256) as u8;
        16 + 36 * r6 + 6 * g6 + b6
    }

    #[inline]
    fn write_u8(&mut self, n: u8) {
        if n >= 100 {
            self.output_buffer.push(b'0' + n / 100);
            self.output_buffer.push(b'0' + (n / 10) % 10);
            self.output_buffer.push(b'0' + n % 10);
        } else if n >= 10 {
            self.output_buffer.push(b'0' + n / 10);
            self.output_buffer.push(b'0' + n % 10);
        } else {
            self.output_buffer.push(b'0' + n);
        }
    }

    /// Set foreground using 256-color mode (much shorter escape sequences)
    #[inline]
    fn set_fg_256(&mut self, color: u8) {
        if color != self.last_fg {
            self.output_buffer.extend_from_slice(b"\x1b[38;5;");
            self.write_u8(color);
            self.output_buffer.push(b'm');
            self.last_fg = color;
        }
    }

    /// Set background using 256-color mode
    #[inline]
    fn set_bg_256(&mut self, color: u8) {
        if color != self.last_bg {
            self.output_buffer.extend_from_slice(b"\x1b[48;5;");
            self.write_u8(color);
            self.output_buffer.push(b'm');
            self.last_bg = color;
        }
    }

    #[inline]
    fn brightness_to_ascii(brightness: u8) -> u8 {
        // Map 0-255 brightness to ASCII char index using integer math
        let index = (brightness as usize * (ASCII_CHARS.len() - 1)) / 255;
        ASCII_CHARS[index]
    }

    fn render_block(&mut self, fb: &FrameBuffer, stdout: &mut io::Stdout) -> io::Result<()> {
        self.output_buffer.clear();
        
        // Use sentinel to force first pixel to set colors
        self.last_fg = 255;
        self.last_bg = 255;

        // Clear entire screen if needed (resize, first frame, etc.)
        if self.needs_clear {
            self.output_buffer.extend_from_slice(b"\x1b[2J");
            self.needs_clear = false;
        }

        // Start at top-left
        self.output_buffer.extend_from_slice(b"\x1b[H");

        for y in (0..GB_HEIGHT).step_by(2) {
            for x in 0..GB_WIDTH {
                let top = fb.get_pixel(x, y);
                let bottom = fb.get_pixel(x, y + 1);

                // Use 256-color palette for much shorter escape sequences
                let fg_color = Self::rgb_to_256(top.r, top.g, top.b);
                let bg_color = Self::rgb_to_256(bottom.r, bottom.g, bottom.b);
                
                self.set_fg_256(fg_color);
                self.set_bg_256(bg_color);
                self.output_buffer.extend_from_slice(HALF_BLOCK_UTF8);
            }
            self.output_buffer.extend_from_slice(b"\r\n");
        }

        // Reset colors at the end
        self.output_buffer.extend_from_slice(b"\x1b[0m");

        stdout.write_all(&self.output_buffer)?;
        stdout.flush()?;
        
        Ok(())
    }

    fn render_ascii(&mut self, fb: &FrameBuffer, stdout: &mut io::Stdout) -> io::Result<()> {
        self.output_buffer.clear();
        
        // Use sentinel to force first pixel to set colors
        self.last_fg = 255;

        // Clear entire screen if needed (resize, first frame, etc.)
        if self.needs_clear {
            self.output_buffer.extend_from_slice(b"\x1b[2J");
            self.needs_clear = false;
        }

        // Start at top-left with black background (256-color index 16 = black)
        self.output_buffer.extend_from_slice(b"\x1b[H\x1b[48;5;16m");

        for y in (0..GB_HEIGHT).step_by(2) {
            for x in 0..GB_WIDTH {
                let top = fb.get_pixel(x, y);
                let bottom = fb.get_pixel(x, y + 1);

                // Average two u8 greys using integer math (avoids overflow)
                let avg_grey = ((top.to_grey() as u16 + bottom.to_grey() as u16) >> 1) as u8;
                let ascii_char = Self::brightness_to_ascii(avg_grey);

                let fg_r = ((top.r as u16 + bottom.r as u16) >> 1) as u8;
                let fg_g = ((top.g as u16 + bottom.g as u16) >> 1) as u8;
                let fg_b = ((top.b as u16 + bottom.b as u16) >> 1) as u8;

                let fg_color = Self::rgb_to_256(fg_r, fg_g, fg_b);
                self.set_fg_256(fg_color);
                self.output_buffer.push(ascii_char);
            }
            self.output_buffer.extend_from_slice(b"\r\n");
        }

        // Reset colors at the end
        self.output_buffer.extend_from_slice(b"\x1b[0m");

        stdout.write_all(&self.output_buffer)?;
        stdout.flush()?;
        Ok(())
    }
}

