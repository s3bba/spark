use std::convert::TryInto;
#[cfg(not(spark_embedded_font))]
use std::{env, fs};

use freetype as ft;

#[cfg(spark_embedded_font)]
static EMBEDDED_FONT_BYTES: &[u8] = include_bytes!(env!("SPARK_EMBEDDED_FONT_FILE"));

#[derive(Clone, Copy)]
pub(crate) struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

pub(crate) struct LineMetrics {
    pub(crate) ascent: i32,
    pub(crate) height: i32,
}

pub(crate) struct FontRenderer {
    _library: ft::Library,
    face: ft::Face,
}

impl Rect {
    pub(crate) const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl FontRenderer {
    fn new(bytes: Vec<u8>) -> Self {
        let library = ft::Library::init().expect("failed to initialize FreeType");
        let face = library
            .new_memory_face(bytes, 0)
            .expect("failed to load IBM Plex Mono into FreeType");
        Self {
            _library: library,
            face,
        }
    }

    pub(crate) fn line_metrics(&self, font_size: f32) -> LineMetrics {
        self.configure_size(font_size);
        let metrics = self
            .face
            .size_metrics()
            .expect("font size metrics should be available after setting a pixel size");
        LineMetrics {
            ascent: fixed_26_6_to_pixels(metrics.ascender as i64),
            height: fixed_26_6_to_pixels(metrics.height as i64),
        }
    }

    pub(crate) fn measure_text_width(&self, font_size: f32, text: &str) -> i32 {
        self.configure_size(font_size);
        let mut width = 0;

        for character in text.chars() {
            if self
                .face
                .load_char(character as usize, font_load_flags())
                .is_err()
            {
                continue;
            }

            width += fixed_26_6_to_pixels(self.face.glyph().advance().x as i64);
        }

        width
    }

    pub(crate) fn draw_text(
        &self,
        canvas: &mut [u8],
        width: usize,
        height: usize,
        x: i32,
        baseline_y: i32,
        font_size: f32,
        color: u32,
        text: &str,
    ) {
        self.draw_highlighted_text(
            canvas,
            width,
            height,
            x,
            baseline_y,
            font_size,
            color,
            color,
            text,
            &[],
        );
    }

    pub(crate) fn draw_highlighted_text(
        &self,
        canvas: &mut [u8],
        width: usize,
        height: usize,
        x: i32,
        baseline_y: i32,
        font_size: f32,
        color: u32,
        highlight_color: u32,
        text: &str,
        highlighted_positions: &[usize],
    ) {
        self.configure_size(font_size);
        let mut pen_x = x;
        let mut next_highlight = highlighted_positions.iter().copied().peekable();

        for (index, character) in text.chars().enumerate() {
            if self
                .face
                .load_char(character as usize, font_load_flags())
                .is_err()
            {
                continue;
            }

            let glyph = self.face.glyph();
            let glyph_color = if next_highlight.peek().copied() == Some(index) {
                next_highlight.next();
                highlight_color
            } else {
                color
            };

            draw_glyph_bitmap(
                canvas,
                width,
                height,
                &glyph.bitmap(),
                pen_x + glyph.bitmap_left(),
                baseline_y - glyph.bitmap_top(),
                glyph_color,
            );
            pen_x += fixed_26_6_to_pixels(glyph.advance().x as i64);
        }
    }

    fn configure_size(&self, font_size: f32) {
        let pixel_height = font_size.max(1.0).round() as u32;
        self.face
            .set_pixel_sizes(0, pixel_height)
            .expect("failed to configure FreeType pixel size");
    }
}

pub(crate) fn clear(canvas: &mut [u8], color: u32) {
    let pixel = color.to_le_bytes();
    for chunk in canvas.chunks_exact_mut(4) {
        let array: &mut [u8; 4] = chunk.try_into().expect("pixel must be four bytes");
        *array = pixel;
    }
}

pub(crate) fn fill_rect(canvas: &mut [u8], width: usize, height: usize, rect: Rect, color: u32) {
    let x_start = rect.x.max(0) as usize;
    let y_start = rect.y.max(0) as usize;
    let x_end = (rect.x + rect.width).min(width as i32).max(0) as usize;
    let y_end = (rect.y + rect.height).min(height as i32).max(0) as usize;

    for y in y_start..y_end {
        for x in x_start..x_end {
            put_pixel(canvas, width, x, y, color);
        }
    }
}

pub(crate) fn fill_rect_clipped_to_rounded(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    rect: Rect,
    clip_rect: Rect,
    radius: i32,
    color: u32,
) {
    let x_start = rect.x.max(0) as usize;
    let y_start = rect.y.max(0) as usize;
    let x_end = (rect.x + rect.width).min(width as i32).max(0) as usize;
    let y_end = (rect.y + rect.height).min(height as i32).max(0) as usize;

    for y in y_start..y_end {
        for x in x_start..x_end {
            if rounded_rect_contains(clip_rect, radius, x as i32, y as i32) {
                put_pixel(canvas, width, x, y, color);
            }
        }
    }
}

pub(crate) fn fill_rounded_rect(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    rect: Rect,
    radius: i32,
    color: u32,
) {
    let radius = radius.max(0);
    let radius_cubic = cubic(radius);
    let x_start = rect.x.max(0) as usize;
    let y_start = rect.y.max(0) as usize;
    let x_end = (rect.x + rect.width).min(width as i32).max(0) as usize;
    let y_end = (rect.y + rect.height).min(height as i32).max(0) as usize;

    for y in y_start..y_end {
        for x in x_start..x_end {
            let local_x = x as i32 - rect.x;
            let local_y = y as i32 - rect.y;

            let dx = if local_x < radius {
                radius - local_x - 1
            } else if local_x >= rect.width - radius {
                local_x - (rect.width - radius)
            } else {
                0
            };
            let dy = if local_y < radius {
                radius - local_y - 1
            } else if local_y >= rect.height - radius {
                local_y - (rect.height - radius)
            } else {
                0
            };

            if cubic(dx) + cubic(dy) <= radius_cubic {
                put_pixel(canvas, width, x, y, color);
            }
        }
    }
}

pub(crate) fn stroke_rounded_rect(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    rect: Rect,
    radius: i32,
    stroke_width: i32,
    color: u32,
) {
    let stroke_width = stroke_width.max(0);
    if stroke_width == 0 || rect.width <= 0 || rect.height <= 0 {
        return;
    }

    let inner_rect = Rect::new(
        rect.x + stroke_width,
        rect.y + stroke_width,
        rect.width - stroke_width * 2,
        rect.height - stroke_width * 2,
    );
    let inner_radius = (radius - stroke_width).max(0);
    let has_inner_rect = inner_rect.width > 0 && inner_rect.height > 0;
    let x_start = rect.x.max(0) as usize;
    let y_start = rect.y.max(0) as usize;
    let x_end = (rect.x + rect.width).min(width as i32).max(0) as usize;
    let y_end = (rect.y + rect.height).min(height as i32).max(0) as usize;

    for y in y_start..y_end {
        for x in x_start..x_end {
            let inside_outer = rounded_rect_contains(rect, radius, x as i32, y as i32);
            let inside_inner = has_inner_rect
                && rounded_rect_contains(inner_rect, inner_radius, x as i32, y as i32);

            if inside_outer && !inside_inner {
                put_pixel(canvas, width, x, y, color);
            }
        }
    }
}

pub(crate) fn head_text_to_width(
    font: &FontRenderer,
    font_size: f32,
    text: &str,
    max_width: i32,
) -> String {
    if font.measure_text_width(font_size, text) <= max_width {
        return text.to_string();
    }

    let mut fitted = String::new();
    for character in text.chars() {
        let mut candidate = fitted.clone();
        candidate.push(character);
        if font.measure_text_width(font_size, &candidate) > max_width {
            break;
        }
        fitted = candidate;
    }

    fitted
}

pub(crate) fn tail_text_to_width(
    font: &FontRenderer,
    font_size: f32,
    text: &str,
    max_width: i32,
) -> String {
    if font.measure_text_width(font_size, text) <= max_width {
        return text.to_string();
    }

    let characters: Vec<char> = text.chars().collect();
    for start in 0..characters.len() {
        let candidate: String = characters[start..].iter().collect();
        if font.measure_text_width(font_size, &candidate) <= max_width {
            return candidate;
        }
    }

    String::new()
}

pub(crate) fn load_font() -> FontRenderer {
    #[cfg(spark_embedded_font)]
    {
        FontRenderer::new(EMBEDDED_FONT_BYTES.to_vec())
    }

    #[cfg(not(spark_embedded_font))]
    {
        let mut candidates = Vec::new();
        if let Ok(path) = env::var("SPARK_FONT_FILE") {
            candidates.push(path);
        }
        candidates.extend(
            [
                "/usr/share/fonts/opentype/ibm-plex/IBMPlexMono-Regular.otf",
                "/usr/share/fonts/truetype/ibm-plex/IBMPlexMono-Regular.ttf",
                "/usr/share/fonts/OTF/IBMPlexMono-Regular.otf",
            ]
            .into_iter()
            .map(String::from),
        );

        for path in candidates {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };

            return FontRenderer::new(bytes);
        }

        panic!("failed to load IBM Plex Mono; set SPARK_FONT_FILE to the regular font file");
    }
}

pub(crate) fn scale_px(value: i32, scale: f64) -> i32 {
    ((value as f64) * scale).round() as i32
}

fn rounded_rect_contains(rect: Rect, radius: i32, x: i32, y: i32) -> bool {
    if x < rect.x || y < rect.y || x >= rect.x + rect.width || y >= rect.y + rect.height {
        return false;
    }

    let radius = radius.max(0);
    let radius_cubic = cubic(radius);
    let local_x = x - rect.x;
    let local_y = y - rect.y;

    let dx = if local_x < radius {
        radius - local_x - 1
    } else if local_x >= rect.width - radius {
        local_x - (rect.width - radius)
    } else {
        0
    };
    let dy = if local_y < radius {
        radius - local_y - 1
    } else if local_y >= rect.height - radius {
        local_y - (rect.height - radius)
    } else {
        0
    };

    cubic(dx) + cubic(dy) <= radius_cubic
}

fn cubic(value: i32) -> i64 {
    let value = value as i64;
    value * value * value
}

fn put_pixel(canvas: &mut [u8], width: usize, x: usize, y: usize, color: u32) {
    let offset = (y * width + x) * 4;
    let pixel = color.to_le_bytes();
    canvas[offset..offset + 4].copy_from_slice(&pixel);
}

fn blend_pixel(canvas: &mut [u8], width: usize, x: usize, y: usize, color: u32, alpha: u8) {
    let source_alpha = (((color >> 24) & 0xFF) * alpha as u32 + 127) / 255;
    if source_alpha == 0 {
        return;
    }

    let offset = (y * width + x) * 4;
    let destination = u32::from_le_bytes(
        canvas[offset..offset + 4]
            .try_into()
            .expect("pixel must be four bytes"),
    );

    let source_red = (color >> 16) & 0xFF;
    let source_green = (color >> 8) & 0xFF;
    let source_blue = color & 0xFF;

    let destination_alpha = (destination >> 24) & 0xFF;
    let destination_red = (destination >> 16) & 0xFF;
    let destination_green = (destination >> 8) & 0xFF;
    let destination_blue = destination & 0xFF;

    let inverse_alpha = 255 - source_alpha;
    let out_alpha = source_alpha + (destination_alpha * inverse_alpha + 127) / 255;
    let out_red = (source_red * source_alpha + destination_red * inverse_alpha + 127) / 255;
    let out_green = (source_green * source_alpha + destination_green * inverse_alpha + 127) / 255;
    let out_blue = (source_blue * source_alpha + destination_blue * inverse_alpha + 127) / 255;

    let pixel = (out_alpha << 24) | (out_red << 16) | (out_green << 8) | out_blue;
    canvas[offset..offset + 4].copy_from_slice(&pixel.to_le_bytes());
}

fn draw_glyph_bitmap(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    bitmap: &ft::Bitmap,
    origin_x: i32,
    origin_y: i32,
    color: u32,
) {
    let pixel_mode = match bitmap.pixel_mode() {
        Ok(mode) => mode,
        Err(_) => return,
    };

    let pitch = bitmap.pitch().unsigned_abs() as usize;
    let rows = bitmap.rows().max(0) as usize;
    let columns = bitmap.width().max(0) as usize;
    let buffer = bitmap.buffer();

    match pixel_mode {
        ft::bitmap::PixelMode::Gray => {
            for row in 0..rows {
                for column in 0..columns {
                    let pixel_x = origin_x + column as i32;
                    let pixel_y = origin_y + row as i32;
                    if pixel_x < 0
                        || pixel_y < 0
                        || pixel_x as usize >= width
                        || pixel_y as usize >= height
                    {
                        continue;
                    }

                    let alpha = buffer[row * pitch + column];
                    blend_pixel(
                        canvas,
                        width,
                        pixel_x as usize,
                        pixel_y as usize,
                        color,
                        alpha,
                    );
                }
            }
        }
        ft::bitmap::PixelMode::Mono => {
            for row in 0..rows {
                for column in 0..columns {
                    let byte = buffer[row * pitch + column / 8];
                    let mask = 0x80 >> (column % 8);
                    if byte & mask == 0 {
                        continue;
                    }

                    let pixel_x = origin_x + column as i32;
                    let pixel_y = origin_y + row as i32;
                    if pixel_x < 0
                        || pixel_y < 0
                        || pixel_x as usize >= width
                        || pixel_y as usize >= height
                    {
                        continue;
                    }

                    blend_pixel(
                        canvas,
                        width,
                        pixel_x as usize,
                        pixel_y as usize,
                        color,
                        255,
                    );
                }
            }
        }
        _ => {}
    }
}

fn fixed_26_6_to_pixels(value: i64) -> i32 {
    ((value + 32) >> 6) as i32
}

fn font_load_flags() -> ft::face::LoadFlag {
    ft::face::LoadFlag::RENDER | ft::face::LoadFlag::TARGET_LIGHT
}
