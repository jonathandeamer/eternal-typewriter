use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use noto_sans_mono_bitmap::{
    get_raster, get_raster_width, FontWeight, RasterHeight, RasterizedChar,
};

pub type Rgb = (u8, u8, u8);

pub const PAPER: Rgb = (245, 240, 230); // warm white
pub const INK: Rgb = (40, 38, 34);
pub const DIM: Rgb = (165, 155, 138); // separator lines
pub const ALERT: Rgb = (170, 40, 40); // margin warning glyph

const WEIGHT: FontWeight = FontWeight::Bold;
const HEIGHT: RasterHeight = RasterHeight::Size24;
const MARGIN: usize = 24;

fn raster(ch: char) -> RasterizedChar {
    // Fall back to '-' for glyphs outside the compiled-in ranges; basic
    // latin is always compiled in, so the unwrap cannot fail.
    get_raster(ch, WEIGHT, HEIGHT)
        .unwrap_or_else(|| get_raster('-', WEIGHT, HEIGHT).unwrap())
}

pub struct Renderer {
    buffer: &'static mut [u8],
    info: FrameBufferInfo,
    glyph_width: usize,
    line_height: usize,
    pub columns: usize,
    pub rows: usize,
}

impl Renderer {
    pub fn new(framebuffer: &'static mut FrameBuffer) -> Self {
        let info = framebuffer.info();
        // Spec: glyph metrics come from the font API, the grid from the
        // actual mode the bootloader gave us — nothing is assumed.
        let glyph_width = get_raster_width(WEIGHT, HEIGHT);
        let line_height = HEIGHT.val();
        let columns = (info.width - 2 * MARGIN) / glyph_width;
        let rows = (info.height - 2 * MARGIN) / line_height;
        let mut renderer = Renderer {
            buffer: framebuffer.buffer_mut(),
            info,
            glyph_width,
            line_height,
            columns,
            rows,
        };
        renderer.fill(PAPER);
        renderer
    }

    /// Raw parts for the panic handler (Task 15).
    pub fn raw_parts(&mut self) -> (*mut u8, usize, FrameBufferInfo) {
        (self.buffer.as_mut_ptr(), self.buffer.len(), self.info)
    }

    pub unsafe fn from_raw_parts(ptr: *mut u8, len: usize, info: FrameBufferInfo) -> Self {
        let buffer = core::slice::from_raw_parts_mut(ptr, len);
        let glyph_width = get_raster_width(WEIGHT, HEIGHT);
        let line_height = HEIGHT.val();
        let columns = (info.width - 2 * MARGIN) / glyph_width;
        let rows = (info.height - 2 * MARGIN) / line_height;
        Renderer { buffer, info, glyph_width, line_height, columns, rows }
    }

    fn put_pixel(&mut self, x: usize, y: usize, (r, g, b): Rgb) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let offset = (y * self.info.stride + x) * self.info.bytes_per_pixel;
        let pixel = &mut self.buffer[offset..offset + self.info.bytes_per_pixel];
        match self.info.pixel_format {
            PixelFormat::Rgb => {
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
            }
            PixelFormat::Bgr => {
                pixel[0] = b;
                pixel[1] = g;
                pixel[2] = r;
            }
            _ => {
                let grey = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                pixel[0] = grey;
            }
        }
    }

    pub fn fill(&mut self, color: Rgb) {
        for y in 0..self.info.height {
            for x in 0..self.info.width {
                self.put_pixel(x, y, color);
            }
        }
    }

    fn cell_origin(&self, row: usize, col: usize) -> (usize, usize) {
        (MARGIN + col * self.glyph_width, MARGIN + row * self.line_height)
    }

    fn fill_cell(&mut self, row: usize, col: usize, color: Rgb) {
        let (x0, y0) = self.cell_origin(row, col);
        for dy in 0..self.line_height {
            for dx in 0..self.glyph_width {
                self.put_pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    pub fn draw_char(&mut self, row: usize, col: usize, ch: char, ink: Rgb) {
        if row >= self.rows || col >= self.columns {
            return;
        }
        let glyph = raster(ch);
        let (x0, y0) = self.cell_origin(row, col);
        for (dy, raster_row) in glyph.raster().iter().enumerate() {
            for (dx, &alpha) in raster_row.iter().enumerate() {
                let color = (
                    mix(PAPER.0, ink.0, alpha),
                    mix(PAPER.1, ink.1, alpha),
                    mix(PAPER.2, ink.2, alpha),
                );
                self.put_pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    pub fn draw_cursor(&mut self, row: usize, col: usize, on: bool) {
        if row >= self.rows || col >= self.columns {
            return;
        }
        self.fill_cell(row, col, if on { INK } else { PAPER });
    }

    /// Persistent disk-trouble warning in the top-right margin (Task 14).
    pub fn draw_warning_glyph(&mut self) {
        let glyph = raster('!');
        let x0 = self.info.width - MARGIN + 2;
        for (dy, raster_row) in glyph.raster().iter().enumerate() {
            for (dx, &alpha) in raster_row.iter().enumerate() {
                let color = (
                    mix(PAPER.0, ALERT.0, alpha),
                    mix(PAPER.1, ALERT.1, alpha),
                    mix(PAPER.2, ALERT.2, alpha),
                );
                self.put_pixel(x0 + dx, 4 + dy, color);
            }
        }
    }
}

/// Linear blend of paper and ink by glyph coverage.
fn mix(paper: u8, ink: u8, alpha: u8) -> u8 {
    let p = paper as i32;
    let i = ink as i32;
    (p + (i - p) * alpha as i32 / 255) as u8
}
