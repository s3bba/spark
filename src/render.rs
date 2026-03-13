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
            let coverage = rounded_rect_coverage(clip_rect, radius, x as i32, y as i32);
            if coverage == 255 {
                put_pixel(canvas, width, x, y, color);
            } else if coverage > 0 {
                blend_pixel_with_coverage(canvas, width, x, y, color, coverage);
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
    let x_start = rect.x.max(0) as usize;
    let y_start = rect.y.max(0) as usize;
    let x_end = (rect.x + rect.width).min(width as i32).max(0) as usize;
    let y_end = (rect.y + rect.height).min(height as i32).max(0) as usize;

    for y in y_start..y_end {
        for x in x_start..x_end {
            let coverage = rounded_rect_coverage(rect, radius, x as i32, y as i32);
            if coverage == 255 {
                put_pixel(canvas, width, x, y, color);
            } else if coverage > 0 {
                blend_pixel_with_coverage(canvas, width, x, y, color, coverage);
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
            let outer_coverage = rounded_rect_coverage(rect, radius, x as i32, y as i32) as i32;
            let inner_coverage = if has_inner_rect {
                rounded_rect_coverage(inner_rect, inner_radius, x as i32, y as i32) as i32
            } else {
                0
            };
            let stroke_coverage = (outer_coverage - inner_coverage).max(0) as u8;
            if stroke_coverage == 255 {
                put_pixel(canvas, width, x, y, color);
            } else if stroke_coverage > 0 {
                blend_pixel_with_coverage(canvas, width, x, y, color, stroke_coverage);
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

fn rounded_rect_coverage(rect: Rect, radius: i32, x: i32, y: i32) -> u8 {
    if x < rect.x || y < rect.y || x >= rect.x + rect.width || y >= rect.y + rect.height {
        return 0;
    }

    let radius = radius.max(0);
    if radius == 0 {
        return 255;
    }

    let local_x = x - rect.x;
    let local_y = y - rect.y;
    let corner_x = local_x < radius || local_x >= rect.width - radius;
    let corner_y = local_y < radius || local_y >= rect.height - radius;
    if !corner_x || !corner_y {
        return 255;
    }

    const SUBPIXEL_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let mut covered = 0;
    for sample_y in SUBPIXEL_OFFSETS {
        for sample_x in SUBPIXEL_OFFSETS {
            let pixel_x = x as f64 + sample_x - 0.5;
            let pixel_y = y as f64 + sample_y - 0.5;
            if rounded_rect_contains_at(rect, radius, pixel_x, pixel_y) {
                covered += 1;
            }
        }
    }

    ((covered * 255 + 8) / 16) as u8
}

fn rounded_rect_contains_at(rect: Rect, radius: i32, x: f64, y: f64) -> bool {
    if x < rect.x as f64
        || y < rect.y as f64
        || x >= (rect.x + rect.width) as f64
        || y >= (rect.y + rect.height) as f64
    {
        return false;
    }

    let radius_i32 = radius.max(0);
    let radius = radius_i32 as f64;
    let radius_cubic = radius * radius * radius;
    let local_x = x - rect.x as f64;
    let local_y = y - rect.y as f64;
    let right_edge = (rect.width - radius_i32) as f64;
    let bottom_edge = (rect.height - radius_i32) as f64;

    let dx = if local_x < radius {
        radius - local_x - 1.0
    } else if local_x >= right_edge {
        local_x - right_edge
    } else {
        0.0
    };
    let dy = if local_y < radius {
        radius - local_y - 1.0
    } else if local_y >= bottom_edge {
        local_y - bottom_edge
    } else {
        0.0
    };

    dx * dx * dx + dy * dy * dy <= radius_cubic
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

fn blend_pixel_with_coverage(
    canvas: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    color: u32,
    coverage: u8,
) {
    if coverage == 0 {
        return;
    }
    if coverage == 255 {
        put_pixel(canvas, width, x, y, color);
        return;
    }

    let offset = (y * width + x) * 4;
    let destination = u32::from_le_bytes(
        canvas[offset..offset + 4]
            .try_into()
            .expect("pixel must be four bytes"),
    );

    let source_alpha = ((color >> 24) & 0xFF) as u64;
    let source_red = ((color >> 16) & 0xFF) as u64;
    let source_green = ((color >> 8) & 0xFF) as u64;
    let source_blue = (color & 0xFF) as u64;

    let destination_alpha = ((destination >> 24) & 0xFF) as u64;
    let destination_red = ((destination >> 16) & 0xFF) as u64;
    let destination_green = ((destination >> 8) & 0xFF) as u64;
    let destination_blue = (destination & 0xFF) as u64;

    let coverage = coverage as u64;
    let inverse_coverage = 255 - coverage;

    let source_alpha_weighted = source_alpha * coverage;
    let destination_alpha_weighted = destination_alpha * inverse_coverage;
    let out_alpha_numerator = source_alpha_weighted + destination_alpha_weighted;
    let out_alpha = ((out_alpha_numerator + 127) / 255) as u32;

    if out_alpha_numerator == 0 {
        canvas[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
        return;
    }

    let out_red = ((source_red * source_alpha_weighted
        + destination_red * destination_alpha_weighted
        + out_alpha_numerator / 2)
        / out_alpha_numerator) as u32;
    let out_green = ((source_green * source_alpha_weighted
        + destination_green * destination_alpha_weighted
        + out_alpha_numerator / 2)
        / out_alpha_numerator) as u32;
    let out_blue = ((source_blue * source_alpha_weighted
        + destination_blue * destination_alpha_weighted
        + out_alpha_numerator / 2)
        / out_alpha_numerator) as u32;

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
