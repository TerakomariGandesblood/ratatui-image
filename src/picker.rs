//! Helper module to build a backend, and swap backends at runtime

use image::{DynamicImage, Rgb};
use ratatui::layout::Rect;
#[cfg(feature = "rustix")]
use rustix::termios::Winsize;
#[cfg(feature = "serde")]
use serde::Deserialize;

#[cfg(feature = "sixel")]
use crate::backend::sixel::{resizeable::SixelState, FixedSixel};

use crate::{
    backend::{
        halfblocks::{resizeable::HalfblocksState, FixedHalfblocks},
        kitty::{FixedKitty, KittyState},
        FixedBackend, ResizeBackend,
    },
    FontSize, ImageSource, Resize, Result,
};

#[derive(Clone, Copy)]
pub struct Picker {
    font_size: FontSize,
    background_color: Option<Rgb<u8>>,
    backend_type: BackendType,
    kitty_counter: u8,
}

#[derive(PartialEq, Clone, Debug, Copy)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum BackendType {
    Halfblocks,
    #[cfg(feature = "sixel")]
    Sixel,
    Kitty,
}

impl BackendType {
    pub fn next(&self) -> BackendType {
        match self {
            #[cfg(not(feature = "sixel"))]
            BackendType::Halfblocks => BackendType::Kitty,
            #[cfg(feature = "sixel")]
            BackendType::Halfblocks => BackendType::Sixel,
            #[cfg(feature = "sixel")]
            BackendType::Sixel => BackendType::Kitty,
            BackendType::Kitty => BackendType::Halfblocks,
        }
    }
}

/// Helper for building widgets
impl Picker {
    /// Create a picker from a given terminal [FontSize].
    ///
    /// # Example
    /// ```rust
    /// use std::io;
    /// use ratatu_image::{
    ///     picker::{BackendType, Picker},
    ///     Resize,
    /// };
    /// use ratatui::{
    ///     backend::{Backend, TestBackend},
    ///     layout::Rect,
    ///     Terminal,
    /// };
    ///
    /// let dyn_img = image::io::Reader::open("./assets/Ada.png").unwrap().decode().unwrap();
    /// let mut picker = Picker::new(
    ///     (7, 14),
    ///     BackendType::Halfblocks,
    ///     None,
    /// ).unwrap();
    ///
    /// // For FixedImage:
    /// let image_static = picker.new_static_fit(
    ///     dyn_img,
    ///     Rect::new(0, 0, 15, 5),
    ///     Resize::Fit,
    /// ).unwrap();
    /// // For ResizeImage:
    /// let image_fit_state = picker.new_state();
    /// ```
    pub fn new(
        font_size: FontSize,
        backend_type: BackendType,
        background_color: Option<Rgb<u8>>,
    ) -> Result<Picker> {
        Ok(Picker {
            font_size,
            background_color,
            backend_type,
            kitty_counter: 0,
        })
    }

    /// Query the terminal window size with I/O for font size.
    #[cfg(feature = "rustix")]
    pub fn from_ioctl(
        backend_type: BackendType,
        background_color: Option<Rgb<u8>>,
    ) -> Result<Picker> {
        let stdout = rustix::stdio::stdout();
        let font_size = font_size(rustix::termios::tcgetwinsize(stdout)?)?;
        Picker::new(font_size, backend_type, background_color)
    }

    pub fn guess(&mut self) -> BackendType {
        self.backend_type = guess_backend();
        self.backend_type
    }

    /// Set a specific backend
    pub fn set(&mut self, r#type: BackendType) {
        self.backend_type = r#type;
    }

    /// Cycle through available backends
    pub fn cycle_backends(&mut self) -> BackendType {
        self.backend_type = self.backend_type.next();
        self.backend_type
    }

    /// Returns a new *static* backend for [`crate::FixedImage`] widgets that fits into the given size.
    pub fn new_static_fit(
        &mut self,
        image: DynamicImage,
        size: Rect,
        resize: Resize,
    ) -> Result<Box<dyn FixedBackend>> {
        let source = ImageSource::new(image, self.font_size);
        match self.backend_type {
            BackendType::Halfblocks => Ok(Box::new(FixedHalfblocks::from_source(
                &source,
                resize,
                self.background_color,
                size,
            )?)),
            #[cfg(feature = "sixel")]
            BackendType::Sixel => Ok(Box::new(FixedSixel::from_source(
                &source,
                resize,
                self.background_color,
                size,
            )?)),
            BackendType::Kitty => {
                self.kitty_counter += 1;
                Ok(Box::new(FixedKitty::from_source(
                    &source,
                    resize,
                    self.background_color,
                    size,
                    self.kitty_counter,
                )?))
            }
        }
    }

    /// Returns a new *state* backend for [`crate::ResizeImage`].
    pub fn new_state(&mut self) -> Box<dyn ResizeBackend> {
        match self.backend_type {
            BackendType::Halfblocks => Box::<HalfblocksState>::default(),
            #[cfg(feature = "sixel")]
            BackendType::Sixel => Box::<SixelState>::default(),
            BackendType::Kitty => {
                self.kitty_counter += 1;
                Box::new(KittyState::new(self.kitty_counter))
            }
        }
    }

    pub fn backend_type(&self) -> &BackendType {
        &self.backend_type
    }

    pub fn font_size(&self) -> FontSize {
        self.font_size
    }
}

#[cfg(feature = "rustix")]
pub fn font_size(winsize: Winsize) -> Result<FontSize> {
    let Winsize {
        ws_xpixel: x,
        ws_ypixel: y,
        ws_col: cols,
        ws_row: rows,
    } = winsize;
    if x == 0 || y == 0 || cols == 0 || rows == 0 {
        return Err(String::from("font_size zero value").into());
    }
    Ok((x / cols, y / rows))
}

// Check if Sixel protocol can be used
fn guess_backend() -> BackendType {
    if let Ok(term) = std::env::var("TERM") {
        match term.as_str() {
            "mlterm" | "yaft-256color" | "foot" | "foot-extra" | "alacritty" => {
                return BackendType::Sixel
            }
            "st-256color" | "xterm" | "xterm-256color" => {
                return check_device_attrs().unwrap_or(BackendType::Halfblocks)
            }
            term => {
                if term.contains("kitty") {
                    return BackendType::Kitty;
                }
                if let Ok(term_program) = std::env::var("TERM_PROGRAM") {
                    if term_program == "MacTerm" {
                        return BackendType::Sixel;
                    }
                }
            }
        }
    }
    BackendType::Halfblocks
}

// Check if Sixel is within the terminal's attributes
// see https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Sixel-Graphics
// and https://vt100.net/docs/vt510-rm/DA1.html
fn check_device_attrs() -> Result<BackendType> {
    todo!();
    // let mut term = Term::stdout();
    //
    // write!(&mut term, "\x1b[c")?;
    // term.flush()?;
    //
    // let mut response = String::new();
    //
    // while let Ok(key) = term.read_key() {
    // if let Key::Char(c) = key {
    // response.push(c);
    // if c == 'c' {
    // break;
    // }
    // }
    // }
    //
    // Ok(response.contains(";4;") || response.contains(";4c"))
}

#[cfg(all(test, feature = "rustix", feature = "sixel"))]
mod tests {
    use std::assert_eq;

    use crate::picker::{font_size, BackendType, Picker};
    use rustix::termios::Winsize;

    #[test]
    fn test_font_size() {
        assert!(font_size(Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 10,
            ws_ypixel: 10
        })
        .is_err());
        assert!(font_size(Winsize {
            ws_row: 10,
            ws_col: 10,
            ws_xpixel: 0,
            ws_ypixel: 0
        })
        .is_err());
    }

    #[test]
    fn test_cycle_backends() {
        let mut picker = Picker::new((1, 1), BackendType::Halfblocks, None).unwrap();
        #[cfg(feature = "sixel")]
        assert_eq!(picker.cycle_backends(), BackendType::Sixel);
        assert_eq!(picker.cycle_backends(), BackendType::Kitty);
        assert_eq!(picker.cycle_backends(), BackendType::Halfblocks);
    }
}
