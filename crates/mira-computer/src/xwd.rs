//! Minimal XWD (X Window Dump) decoder.
//!
//! `xwd -root` ships with the stock X11 utilities, so it is the one
//! screenshot command that is almost always present on an X11 box. This
//! decoder covers the only layout a modern root window produces:
//! ZPixmap, TrueColor/DirectColor, 24 or 32 bits per pixel.

use image::RgbaImage;

use crate::ComputerError;

const HEADER_LEN: usize = 100;
const ZPIXMAP: u32 = 2;
const COLOR_ENTRY_LEN: usize = 12;

fn bad(msg: impl Into<String>) -> ComputerError {
    ComputerError::Image(format!("xwd: {}", msg.into()))
}

pub fn decode(buf: &[u8]) -> Result<RgbaImage, ComputerError> {
    if buf.len() < HEADER_LEN {
        return Err(bad("truncated header"));
    }
    let field = |i: usize| u32::from_be_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
    let header_size = field(0) as usize;
    let pixmap_format = field(2);
    let width = field(4);
    let height = field(5);
    let byte_order = field(7); // 0 = LSBFirst, 1 = MSBFirst
    let bits_per_pixel = field(11);
    let bytes_per_line = field(12) as usize;
    let (red_mask, green_mask, blue_mask) = (field(14), field(15), field(16));
    let ncolors = field(19) as usize;

    if pixmap_format != ZPIXMAP {
        return Err(bad(format!("unsupported pixmap format {pixmap_format}")));
    }
    if bits_per_pixel != 24 && bits_per_pixel != 32 {
        return Err(bad(format!("unsupported depth {bits_per_pixel} bpp")));
    }
    let bpp = (bits_per_pixel / 8) as usize;
    let data_start = header_size + ncolors * COLOR_ENTRY_LEN;
    let needed = data_start + bytes_per_line * height as usize;
    if header_size < HEADER_LEN || buf.len() < needed || bytes_per_line < width as usize * bpp {
        return Err(bad("truncated pixel data"));
    }

    let channel = |pixel: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        let v = (pixel & mask) >> shift;
        if max == 255 {
            v as u8
        } else {
            ((v as u64 * 255) / max as u64) as u8
        }
    };

    let mut img = RgbaImage::new(width, height);
    for y in 0..height as usize {
        let row = &buf[data_start + y * bytes_per_line..];
        for x in 0..width as usize {
            let px = &row[x * bpp..x * bpp + bpp];
            let mut v: u32 = 0;
            if byte_order == 0 {
                for (i, b) in px.iter().enumerate() {
                    v |= (*b as u32) << (8 * i);
                }
            } else {
                for b in px {
                    v = (v << 8) | *b as u32;
                }
            }
            img.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([
                    channel(v, red_mask),
                    channel(v, green_mask),
                    channel(v, blue_mask),
                    255,
                ]),
            );
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny 2x1 LSB-first 32bpp dump by hand.
    #[test]
    fn decodes_lsb_32bpp() {
        let mut fields = [0u32; 25];
        fields[0] = (HEADER_LEN + 4) as u32; // header + "root" name
        fields[1] = 7;
        fields[2] = ZPIXMAP;
        fields[3] = 24;
        fields[4] = 2;
        fields[5] = 1;
        fields[7] = 0;
        fields[11] = 32;
        fields[12] = 8;
        fields[14] = 0x00ff_0000;
        fields[15] = 0x0000_ff00;
        fields[16] = 0x0000_00ff;
        let mut buf: Vec<u8> = fields.iter().flat_map(|f| f.to_be_bytes()).collect();
        buf.extend_from_slice(b"root");
        // pixel 0: pure red (BGRA little-endian), pixel 1: (1,2,3)
        buf.extend_from_slice(&[0, 0, 255, 0, 3, 2, 1, 0]);
        let img = decode(&buf).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(&[0u8; 10]).is_err());
    }
}
