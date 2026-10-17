//! Writes the escape sequences of the kitty graphics protocol.
//!
//! The protocol sends images to the terminal inside escape sequences, as base64 text split into
//! chunks. The terminal stores each image once and can show it in several places, each called a
//! placement. This module sends and places an image, deletes an image, and deletes its
//! placements. Every command sets `q=2`, which tells the terminal not to reply.

use std::io::{
    Result,
    Write,
};

/// Escape sequence introducer for a graphics command: `ESC _ G`.
const APC_START: &[u8] = b"\x1b_G";
/// Terminator for a graphics command: `ESC \`.
const APC_END: &[u8] = b"\x1b\\";

/// Most base64 bytes the protocol allows in one escape sequence. This counts encoded bytes, so a
/// full chunk carries 3072 bytes of pixels.
const MAX_CHUNK: usize = 4096;

/// The id of the one image this viewer shows. Sending under the same id replaces the previous
/// image, so the terminal does not keep piling up copies.
pub const IMAGE_ID: u32 = 0x707A_6D70;

/// Bytes per pixel in a sent buffer. It is always RGB, because drawing over an opaque background
/// has already removed the alpha channel.
const BYTES_PER_PIXEL: usize = 3;

/// The protocol's `f=` value for 24-bit RGB.
const FORMAT_RGB: u32 = 24;

/// A raw RGB image and its size in pixels.
#[derive(Debug)]
pub struct Frame<'a> {
    /// RGB pixels, row by row from the top.
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
}

impl Frame<'_> {
    /// Returns whether the buffer length matches the width and height.
    ///
    /// This is checked before anything is written. A buffer of the wrong size can leave the
    /// terminal's display stuck instead of failing.
    pub fn is_consistent(&self) -> bool {
        let Ok(width) = usize::try_from(self.width) else {
            return false;
        };
        let Ok(height) = usize::try_from(self.height) else {
            return false;
        };
        width.saturating_mul(height).saturating_mul(BYTES_PER_PIXEL) == self.pixels.len()
    }
}

/// Sends `frame` and shows it at the cursor, one terminal pixel per image pixel.
///
/// It replaces any image already stored under [`IMAGE_ID`]. The caller moves the cursor first.
/// A frame that is empty, or whose buffer does not match its size, is skipped.
pub fn transmit_and_place<W: Write>(out: &mut W, frame: &Frame<'_>) -> Result<()> {
    if !frame.is_consistent() || frame.width == 0 || frame.height == 0 {
        return Ok(());
    }

    let encoded = base64(frame.pixels);
    let mut chunks = encoded.as_bytes().chunks(MAX_CHUNK).peekable();

    // The first chunk carries every key. Later chunks carry only `m`, because repeating the
    // other keys there is an error in the protocol.
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        out.write_all(APC_START)?;
        if first {
            // `q=2` stops the terminal from replying. A reply would arrive on the same input as
            // key presses. `C=1` keeps the cursor where it was instead of moving it past the
            // image.
            write!(
                out,
                "a=T,q=2,f={},s={},v={},i={},C=1,m={}",
                FORMAT_RGB, frame.width, frame.height, IMAGE_ID, more
            )?;
            first = false;
        } else {
            write!(out, "m={more}")?;
        }
        out.write_all(b";")?;
        out.write_all(chunk)?;
        out.write_all(APC_END)?;
    }
    Ok(())
}

/// Removes every placement of this viewer's image, and frees the terminal's copy of its pixels.
pub fn delete_image<W: Write>(out: &mut W) -> Result<()> {
    out.write_all(APC_START)?;
    write!(out, "a=d,q=2,d=I,i={IMAGE_ID}")?;
    out.write_all(b";")?;
    out.write_all(APC_END)?;
    Ok(())
}

/// Removes the image's placements from the screen, but leaves its pixels stored in the terminal.
pub fn delete_placements<W: Write>(out: &mut W) -> Result<()> {
    out.write_all(APC_START)?;
    write!(out, "a=d,q=2,d=i,i={IMAGE_ID}")?;
    out.write_all(b";")?;
    out.write_all(APC_END)?;
    Ok(())
}

/// Encodes `input` as standard base64 with padding. It is short enough to write here instead of
/// adding a dependency.
fn base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for group in input.chunks(3) {
        let (b0, b1, b2) = match *group {
            [b0] => (b0, 0, 0),
            [b0, b1] => (b0, b1, 0),
            [b0, b1, b2] => (b0, b1, b2),
            _ => continue,
        };
        let packed = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(symbol(packed >> 18));
        out.push(symbol(packed >> 12));
        out.push(if group.len() > 1 {
            symbol(packed >> 6)
        } else {
            '='
        });
        out.push(if group.len() > 2 { symbol(packed) } else { '=' });
    }
    out
}

/// Returns the base64 character for the low six bits of `bits`. It is computed rather than
/// looked up in a table, so there is no indexing that could panic.
const fn symbol(bits: u32) -> char {
    let six = (bits & 0x3F) as u8;
    let byte = match six {
        0..=25 => b'A' + six,
        26..=51 => b'a' + (six - 26),
        52..=61 => b'0' + (six - 52),
        62 => b'+',
        _ => b'/',
    };
    byte as char
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_covers_every_alphabet_symbol() {
        // Encoding every byte value uses every base64 character, including `+` and `/`. A
        // URL-safe alphabet would replace those two, and kitty would reject the result.
        let all: Vec<u8> = (0u8..=255).collect();
        let encoded = base64(&all);
        assert_eq!(encoded.len(), 344);
        assert!(encoded.starts_with("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g"));
        assert!(encoded.ends_with("+/w=="));
    }

    /// One RGB pixel is 3 bytes, so it fits in a single chunk marked `m=0`.
    #[test]
    fn a_small_frame_is_one_chunk() {
        let mut out = Vec::new();
        let frame = Frame {
            pixels: &[1, 2, 3],
            width: 1,
            height: 1,
        };
        transmit_and_place(&mut out, &frame).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert_eq!(text.matches("\x1b_G").count(), 1);
        assert!(text.contains("a=T,q=2,f=24,s=1,v=1,"));
        assert!(text.contains("C=1,m=0"));
        assert!(text.ends_with("\x1b\\"));
    }

    /// Every chunk but the last carries `m=1`, and later chunks carry no other keys.
    #[test]
    fn a_large_frame_chunks_correctly() {
        // A full chunk is 4096 base64 bytes holding 3072 raw bytes, which is exactly 1024 RGB
        // pixels. 1500 pixels is 4500 bytes, so it takes one full chunk and one partly full.
        let pixels = vec![0xABu8; 1500 * 3];
        let frame = Frame {
            pixels: &pixels,
            width: 1500,
            height: 1,
        };
        let mut out = Vec::new();
        transmit_and_place(&mut out, &frame).unwrap();
        let text = String::from_utf8(out).unwrap();

        let commands: Vec<&str> = text.split("\x1b_G").skip(1).collect();
        assert_eq!(commands.len(), 2);

        let (first_keys, first_payload) = commands[0].split_once(';').unwrap();
        assert!(first_keys.contains("a=T"));
        assert!(first_keys.ends_with("m=1"));
        assert_eq!(first_payload.trim_end_matches("\x1b\\").len(), MAX_CHUNK);

        let (second_keys, second_payload) = commands[1].split_once(';').unwrap();
        assert_eq!(second_keys, "m=0");
        assert!(second_payload.trim_end_matches("\x1b\\").len() <= MAX_CHUNK);
    }

    /// A buffer whose length does not match its size is skipped rather than half sent.
    #[test]
    fn an_inconsistent_frame_writes_nothing() {
        let frame = Frame {
            pixels: &[1, 2, 3, 4],
            width: 4,
            height: 4,
        };
        assert!(!frame.is_consistent());

        let mut out = Vec::new();
        transmit_and_place(&mut out, &frame).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn delete_sequences_are_well_formed() {
        let mut out = Vec::new();
        delete_image(&mut out).unwrap();
        delete_placements(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("a=d,q=2,d=I,"));
        assert!(text.contains("a=d,q=2,d=i,"));
    }
}
