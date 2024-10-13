//! ITerm2 protocol implementation.
use base64::{engine::general_purpose, Engine};
use image::{DynamicImage, Rgba};
use ratatui::{buffer::Buffer, layout::Rect};
use std::{cmp::min, format, io::Cursor};

use crate::{FontSize, ImageSource, Resize, Result};

use super::{ProtocolTrait, StatefulProtocolTrait};

// Fixed sixel protocol
#[derive(Clone, Default)]
pub struct Iterm2 {
    pub data: String,
    pub area: Rect,
    pub is_tmux: bool,
}

impl Iterm2 {
    pub fn from_source(
        source: &ImageSource,
        font_size: FontSize,
        resize: Resize,
        background_color: Option<Rgba<u8>>,
        is_tmux: bool,
        area: Rect,
    ) -> Result<Self> {
        let resized = resize.resize(
            source,
            font_size,
            Rect::default(),
            area,
            background_color,
            false,
        );
        let (image, area) = match resized {
            Some((ref image, desired)) => (image, desired),
            None => (&source.image, source.area),
        };

        let data = encode(image, is_tmux)?;
        Ok(Self {
            data,
            area,
            is_tmux,
        })
    }
}

// TODO: change E to sixel_rs::status::Error and map when calling
fn encode(img: &DynamicImage, is_tmux: bool) -> Result<String> {
    let mut png: Vec<u8> = vec![];
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;

    let data = general_purpose::STANDARD.encode(&png);

    let (start, end) = if is_tmux {
        ("\x1bPtmux;\x1b\x1b", "\x1b\\")
    } else {
        ("\x1b", "")
    };
    Ok(format!(
        "{start}]1337;File=inline=1;size={};width={}px;height={}px;doNotMoveCursor=1:{}\x07{end}",
        png.len(),
        img.width(),
        img.height(),
        data,
    ))
}

impl ProtocolTrait for Iterm2 {
    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        render(self.area, &self.data, area, buf, false)
    }
}

fn render(rect: Rect, data: &str, area: Rect, buf: &mut Buffer, overdraw: bool) {
    let render_area = match render_area(rect, area, overdraw) {
        None => {
            // If we render out of area, then the buffer will attempt to write regular text (or
            // possibly other sixels) over the image.
            //
            // Note that [StatefulProtocol] forces to ignore this early return, since it will
            // always resize itself to the area.
            return;
        }
        Some(r) => r,
    };

    buf.cell_mut(render_area).map(|cell| cell.set_symbol(data));
    let mut skip_first = false;

    // Skip entire area
    for y in render_area.top()..render_area.bottom() {
        for x in render_area.left()..render_area.right() {
            if !skip_first {
                skip_first = true;
                continue;
            }
            buf.cell_mut((x, y)).map(|cell| cell.set_skip(true));
        }
    }
}

fn render_area(rect: Rect, area: Rect, overdraw: bool) -> Option<Rect> {
    if overdraw {
        return Some(Rect::new(
            area.x,
            area.y,
            min(rect.width, area.width),
            min(rect.height, area.height),
        ));
    }

    if rect.width > area.width || rect.height > area.height {
        return None;
    }
    Some(Rect::new(area.x, area.y, rect.width, rect.height))
}

#[derive(Clone)]
pub struct StatefulIterm2 {
    source: ImageSource,
    font_size: FontSize,
    current: Iterm2,
    hash: u64,
}

impl StatefulIterm2 {
    pub fn new(source: ImageSource, font_size: FontSize, is_tmux: bool) -> StatefulIterm2 {
        StatefulIterm2 {
            source,
            font_size,
            current: Iterm2 {
                is_tmux,
                ..Iterm2::default()
            },
            hash: u64::default(),
        }
    }
}

impl StatefulProtocolTrait for StatefulIterm2 {
    fn needs_resize(&mut self, resize: &Resize, area: Rect) -> Option<Rect> {
        resize.needs_resize(&self.source, self.font_size, self.current.area, area, false)
    }
    fn resize_encode(&mut self, resize: &Resize, background_color: Option<Rgba<u8>>, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let force = self.source.hash != self.hash;
        if let Some((img, rect)) = resize.resize(
            &self.source,
            self.font_size,
            self.current.area,
            area,
            background_color,
            force,
        ) {
            let is_tmux = self.current.is_tmux;
            match encode(&img, is_tmux) {
                Ok(data) => {
                    self.current = Iterm2 {
                        data,
                        area: rect,
                        is_tmux,
                    };
                    self.hash = self.source.hash;
                }
                Err(_err) => {
                    // TODO: save err in struct and expose in trait?
                }
            }
        }
    }
    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        render(self.current.area, &self.current.data, area, buf, true);
    }
}
