//! PWG Raster encoder and decoder.
//!
//! Written from the public specifications:
//!   PWG 5102.4 - PWG Raster Format
//!   CUPS Raster Format (the v2 page header PWG Raster inherits)
//!
//! PWG Raster is the fallback page format every IPP Everywhere / AirPrint
//! printer must accept, so generating it means never depending on a vendor
//! driver. File: magic word `RaS2`, then per page a 1796-byte header
//! followed by that page's run-length compressed bitmap.

pub const MAGIC: &[u8; 4] = b"RaS2";
pub const HEADER_SIZE: usize = 1796;

pub const CSPACE_SGRAY: u32 = 18;
pub const CSPACE_SRGB: u32 = 19;

const OFF_MEDIA_CLASS: usize = 0;
const OFF_MEDIA_TYPE: usize = 128;
const OFF_PAGE_SIZE_NAME: usize = 1732;
const OFF_DUPLEX: usize = 272;
const OFF_HW_RES_X: usize = 276;
const OFF_HW_RES_Y: usize = 280;
const OFF_NUM_COPIES: usize = 340;
const OFF_PAGE_SIZE_X: usize = 352;
const OFF_PAGE_SIZE_Y: usize = 356;
const OFF_TUMBLE: usize = 368;
const OFF_WIDTH: usize = 372;
const OFF_HEIGHT: usize = 376;
const OFF_BITS_PER_COLOR: usize = 384;
const OFF_BITS_PER_PIXEL: usize = 388;
const OFF_BYTES_PER_LINE: usize = 392;
const OFF_COLOR_ORDER: usize = 396;
const OFF_COLOR_SPACE: usize = 400;
const OFF_NUM_COLORS: usize = 420;
const OFF_INTEGER: usize = 452; // cupsInteger[16]
const INT_TOTAL_PAGE_COUNT: usize = 0;
const INT_CROSS_FEED_TRANSFORM: usize = 1;
const INT_FEED_TRANSFORM: usize = 2;
const INT_IMAGE_BOX_RIGHT: usize = 5;
const INT_IMAGE_BOX_BOTTOM: usize = 6;

#[derive(Debug, Clone)]
pub struct PageHeader {
    pub width: u32,
    pub height: u32,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub bits_per_pixel: u32,
    pub bits_per_color: u32,
    pub color_space: u32,
    pub num_colors: u32,
    pub page_size_x: u32, // points
    pub page_size_y: u32,
    pub page_size_name: String,
    pub duplex: bool,
    pub tumble: bool,
    pub num_copies: u32,
    pub total_pages: u32,
}

impl Default for PageHeader {
    fn default() -> Self {
        PageHeader {
            width: 0,
            height: 0,
            dpi_x: 300,
            dpi_y: 300,
            bits_per_pixel: 24,
            bits_per_color: 8,
            color_space: CSPACE_SRGB,
            num_colors: 3,
            page_size_x: 595,
            page_size_y: 842,
            page_size_name: "iso_a4_210x297mm".to_string(),
            duplex: false,
            tumble: false,
            num_copies: 1,
            total_pages: 0,
        }
    }
}

impl PageHeader {
    pub fn bytes_per_line(&self) -> usize {
        (self.width * self.bits_per_pixel).div_ceil(8) as usize
    }
    pub fn bytes_per_pixel(&self) -> usize {
        self.bits_per_pixel.div_ceil(8) as usize
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];
        let put_str = |buf: &mut [u8], off: usize, s: &str, size: usize| {
            let raw = s.as_bytes();
            let n = raw.len().min(size - 1);
            buf[off..off + n].copy_from_slice(&raw[..n]);
        };
        let put_u32 = |buf: &mut [u8], off: usize, v: u32| buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
        let put_i32 = |buf: &mut [u8], off: usize, v: i32| buf[off..off + 4].copy_from_slice(&v.to_be_bytes());

        put_str(&mut buf, OFF_MEDIA_CLASS, "PwgRaster", 64);
        put_str(&mut buf, OFF_MEDIA_TYPE, "", 64);
        put_str(&mut buf, OFF_PAGE_SIZE_NAME, &self.page_size_name, 64);

        put_u32(&mut buf, OFF_DUPLEX, self.duplex as u32);
        put_u32(&mut buf, OFF_HW_RES_X, self.dpi_x);
        put_u32(&mut buf, OFF_HW_RES_Y, self.dpi_y);
        put_u32(&mut buf, OFF_NUM_COPIES, self.num_copies);
        put_u32(&mut buf, OFF_PAGE_SIZE_X, self.page_size_x);
        put_u32(&mut buf, OFF_PAGE_SIZE_Y, self.page_size_y);
        put_u32(&mut buf, OFF_TUMBLE, self.tumble as u32);
        put_u32(&mut buf, OFF_WIDTH, self.width);
        put_u32(&mut buf, OFF_HEIGHT, self.height);
        put_u32(&mut buf, OFF_BITS_PER_COLOR, self.bits_per_color);
        put_u32(&mut buf, OFF_BITS_PER_PIXEL, self.bits_per_pixel);
        put_u32(&mut buf, OFF_BYTES_PER_LINE, self.bytes_per_line() as u32);
        put_u32(&mut buf, OFF_COLOR_ORDER, 0); // chunky
        put_u32(&mut buf, OFF_COLOR_SPACE, self.color_space);
        put_u32(&mut buf, OFF_NUM_COLORS, self.num_colors);

        put_i32(&mut buf, OFF_INTEGER + 4 * INT_TOTAL_PAGE_COUNT, self.total_pages as i32);
        put_i32(&mut buf, OFF_INTEGER + 4 * INT_CROSS_FEED_TRANSFORM, 1);
        put_i32(&mut buf, OFF_INTEGER + 4 * INT_FEED_TRANSFORM, 1);
        put_i32(&mut buf, OFF_INTEGER + 4 * INT_IMAGE_BOX_RIGHT, self.width as i32);
        put_i32(&mut buf, OFF_INTEGER + 4 * INT_IMAGE_BOX_BOTTOM, self.height as i32);

        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < HEADER_SIZE {
            return Err(format!("page header is {} bytes, need {}", buf.len(), HEADER_SIZE));
        }
        let u32_at = |off: usize| u32::from_be_bytes(buf[off..off + 4].try_into().unwrap());
        let text_at = |off: usize, size: usize| {
            let raw = &buf[off..off + size];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };

        let media_class = text_at(OFF_MEDIA_CLASS, 64);
        if media_class != "PwgRaster" {
            return Err(format!("not a PWG Raster page header: MediaClass={:?}", media_class));
        }

        Ok(PageHeader {
            width: u32_at(OFF_WIDTH),
            height: u32_at(OFF_HEIGHT),
            dpi_x: u32_at(OFF_HW_RES_X),
            dpi_y: u32_at(OFF_HW_RES_Y),
            bits_per_pixel: u32_at(OFF_BITS_PER_PIXEL),
            bits_per_color: u32_at(OFF_BITS_PER_COLOR),
            color_space: u32_at(OFF_COLOR_SPACE),
            num_colors: u32_at(OFF_NUM_COLORS),
            page_size_x: u32_at(OFF_PAGE_SIZE_X),
            page_size_y: u32_at(OFF_PAGE_SIZE_Y),
            page_size_name: text_at(OFF_PAGE_SIZE_NAME, 64),
            duplex: u32_at(OFF_DUPLEX) != 0,
            tumble: u32_at(OFF_TUMBLE) != 0,
            num_copies: u32_at(OFF_NUM_COPIES),
            total_pages: u32_at(OFF_INTEGER + 4 * INT_TOTAL_PAGE_COUNT),
        })
    }
}

const MAX_REPEAT: usize = 128;
const MAX_LITERAL: usize = 127;

/// Run-length encodes one raster line's pixels (PackBits-like, per PWG 5102.4 /
/// CUPS raster: a repeat-run byte is `count - 1` (0..127), a literal-run byte
/// is `257 - count` (so 129..255) followed by that many literal pixels).
pub fn compress_line(line: &[u8], bpp: usize) -> Vec<u8> {
    assert!(bpp > 0 && line.len().is_multiple_of(bpp), "line length must be a multiple of bpp");
    let npix = line.len() / bpp;
    let px = |i: usize| &line[i * bpp..(i + 1) * bpp];
    let mut out = Vec::new();
    let mut i = 0;

    while i < npix {
        let mut run = 1;
        while run < MAX_REPEAT && i + run < npix && px(i + run) == px(i) {
            run += 1;
        }
        if run > 1 {
            out.push((run - 1) as u8);
            out.extend_from_slice(px(i));
            i += run;
            continue;
        }

        let start = i;
        let mut count = 1;
        i += 1;
        while count < MAX_LITERAL && i < npix {
            let cur = px(i);
            let next_is_same = i + 1 < npix && px(i + 1) == cur;
            if next_is_same {
                break;
            }
            count += 1;
            i += 1;
        }
        if count == 1 {
            out.push(0);
            out.extend_from_slice(px(start));
        } else {
            out.push((257 - count) as u8);
            out.extend_from_slice(&line[start * bpp..(start + count) * bpp]);
        }
    }
    out
}

pub fn decompress_line(data: &[u8], offset: usize, bytes_per_line: usize, bpp: usize) -> Result<(Vec<u8>, usize), String> {
    let mut out = Vec::with_capacity(bytes_per_line);
    let mut pos = offset;
    while out.len() < bytes_per_line {
        if pos >= data.len() {
            return Err("raster data ended mid-line".into());
        }
        let count = data[pos];
        pos += 1;
        if count & 0x80 != 0 {
            let n = ((257 - count as u32) as usize * bpp).min(bytes_per_line - out.len());
            if pos + n > data.len() {
                return Err("literal run runs past end of raster data".into());
            }
            out.extend_from_slice(&data[pos..pos + n]);
            pos += n;
        } else {
            if pos + bpp > data.len() {
                return Err("repeat run runs past end of raster data".into());
            }
            let pixel = &data[pos..pos + bpp];
            pos += bpp;
            let reps = count as usize + 1;
            for _ in 0..reps {
                if out.len() >= bytes_per_line {
                    break;
                }
                out.extend_from_slice(pixel);
            }
        }
    }
    out.truncate(bytes_per_line);
    Ok((out, pos))
}

/// Encodes one page: its header plus its compressed bitmap. `lines` must
/// yield exactly `header.height` rows, each `header.bytes_per_line()` long.
/// Identical consecutive rows collapse into one line-repeat run.
pub fn encode_page<'a>(header: &PageHeader, lines: impl Iterator<Item = &'a [u8]>) -> Result<Vec<u8>, String> {
    let bpp = header.bytes_per_pixel();
    let expected = header.bytes_per_line();
    let mut out = header.encode();

    let mut pending: Option<(&'a [u8], Vec<u8>)> = None;
    let mut repeats: usize = 0;
    let mut rows = 0usize;

    let flush = |out: &mut Vec<u8>, pending: &Option<(&'a [u8], Vec<u8>)>, repeats: usize| {
        if let Some((_, compressed)) = pending {
            let mut remaining = repeats;
            while remaining > 0 {
                let chunk = remaining.min(256);
                out.push((chunk - 1) as u8);
                out.extend_from_slice(compressed);
                remaining -= chunk;
            }
        }
    };

    for row in lines {
        if row.len() != expected {
            return Err(format!("raster row is {} bytes, header says {}", row.len(), expected));
        }
        rows += 1;
        if let Some((prev, _)) = &pending
            && *prev == row {
                repeats += 1;
                continue;
            }
        flush(&mut out, &pending, repeats);
        pending = Some((row, compress_line(row, bpp)));
        repeats = 1;
    }
    flush(&mut out, &pending, repeats);

    if rows != header.height as usize {
        return Err(format!("got {} raster rows, header says {}", rows, header.height));
    }
    Ok(out)
}

pub fn encode_document(pages: &[(PageHeader, Vec<Vec<u8>>)]) -> Result<Vec<u8>, String> {
    let mut out = MAGIC.to_vec();
    for (header, lines) in pages {
        out.extend(encode_page(header, lines.iter().map(|v| v.as_slice()))?);
    }
    Ok(out)
}

pub fn decode_document(blob: &[u8]) -> Result<Vec<(PageHeader, Vec<Vec<u8>>)>, String> {
    if !blob.starts_with(MAGIC) {
        return Err("missing RaS2 magic word".into());
    }
    let mut pos = MAGIC.len();
    let mut pages = Vec::new();
    while pos < blob.len() {
        let header = PageHeader::decode(&blob[pos..pos + HEADER_SIZE.min(blob.len() - pos)])?;
        pos += HEADER_SIZE;
        let bpp = header.bytes_per_pixel();
        let bytes_per_line = header.bytes_per_line();
        let mut rows = Vec::with_capacity(header.height as usize);
        while rows.len() < header.height as usize {
            if pos >= blob.len() {
                return Err("raster data ended before the page was complete".into());
            }
            let repeat = blob[pos] as usize + 1;
            pos += 1;
            let (line, new_pos) = decompress_line(blob, pos, bytes_per_line, bpp)?;
            pos = new_pos;
            for _ in 0..repeat.min(header.height as usize - rows.len()) {
                rows.push(line.clone());
            }
        }
        pages.push((header, rows));
    }
    Ok(pages)
}

/// Fits an RGB8 or L8 `image` crate buffer into a page matching the header's
/// colour space, row by row.
pub fn image_to_rows(img: &image::DynamicImage, header: &PageHeader) -> Result<Vec<Vec<u8>>, String> {
    let (want_w, want_h) = (header.width, header.height);
    if img.width() != want_w || img.height() != want_h {
        return Err(format!("image is {}x{}, header says {}x{}", img.width(), img.height(), want_w, want_h));
    }
    let stride = header.bytes_per_line();
    let mut rows = Vec::with_capacity(header.height as usize);
    if header.color_space == CSPACE_SGRAY {
        let gray = img.to_luma8();
        for y in 0..header.height {
            let row = gray.rows().nth(y as usize).unwrap();
            let bytes: Vec<u8> = row.map(|p| p.0[0]).collect();
            debug_assert_eq!(bytes.len(), stride);
            rows.push(bytes);
        }
    } else {
        let rgb = img.to_rgb8();
        for y in 0..header.height {
            let row = rgb.rows().nth(y as usize).unwrap();
            let mut bytes = Vec::with_capacity(stride);
            for p in row {
                bytes.extend_from_slice(&p.0);
            }
            debug_assert_eq!(bytes.len(), stride);
            rows.push(bytes);
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_size_and_magic() {
        let h = PageHeader { width: 2480, height: 3508, ..Default::default() };
        let blob = h.encode();
        assert_eq!(blob.len(), HEADER_SIZE);
        assert_eq!(MAGIC, b"RaS2");
        assert_eq!(&blob[0..9], b"PwgRaster");
        assert_eq!(blob[9], 0);
    }

    #[test]
    fn documented_field_offsets() {
        let h = PageHeader { width: 2480, height: 3508, dpi_x: 300, dpi_y: 300, ..Default::default() };
        let blob = h.encode();
        assert_eq!(u32::from_be_bytes(blob[276..280].try_into().unwrap()), 300); // HWResolutionX
        assert_eq!(u32::from_be_bytes(blob[280..284].try_into().unwrap()), 300); // HWResolutionY
        assert_eq!(u32::from_be_bytes(blob[372..376].try_into().unwrap()), 2480); // cupsWidth
        assert_eq!(u32::from_be_bytes(blob[376..380].try_into().unwrap()), 3508); // cupsHeight
        assert_eq!(u32::from_be_bytes(blob[392..396].try_into().unwrap()), 2480 * 3); // cupsBytesPerLine
        assert_eq!(u32::from_be_bytes(blob[400..404].try_into().unwrap()), CSPACE_SRGB);
    }

    #[test]
    fn header_round_trip() {
        let h = PageHeader { width: 2480, height: 3508, page_size_name: "iso_a4_210x297mm".into(), ..Default::default() };
        let back = PageHeader::decode(&h.encode()).unwrap();
        assert_eq!(back.width, 2480);
        assert_eq!(back.height, 3508);
        assert_eq!(back.page_size_name, "iso_a4_210x297mm");
        assert_eq!(back.bytes_per_line(), 2480 * 3);
    }

    #[test]
    fn rejects_header_without_marker() {
        assert!(PageHeader::decode(&[0u8; HEADER_SIZE]).is_err());
    }

    #[test]
    fn rle_round_trips_every_pattern() {
        let cases: Vec<(&str, Vec<u8>, usize)> = vec![
            ("flat colour", vec![0xffu8; 1500], 3),
            ("single pixel", vec![0x12, 0x34, 0x56], 3),
            ("two identical", [0xaau8, 0xbb, 0xcc].repeat(2), 3),
            ("alternating", { let mut v = vec![]; for _ in 0..100 { v.extend([0,0,0,255,255,255]); } v }, 3),
            ("long repeat past 128", [5u8, 5, 5].repeat(400), 3),
            ("greyscale gradient", (0..=255u8).collect(), 1),
            ("greyscale flat", vec![0x80u8; 1000], 1),
        ];
        for (label, line, bpp) in cases {
            let compressed = compress_line(&line, bpp);
            let (out, pos) = decompress_line(&compressed, 0, line.len(), bpp).unwrap();
            assert_eq!(out, line, "mismatch: {label}");
            assert_eq!(pos, compressed.len(), "didn't consume exactly its bytes: {label}");
        }
    }

    #[test]
    fn compression_actually_compresses() {
        let flat = compress_line(&vec![0x77u8; 3000], 3);
        assert!(flat.len() < 300, "a uniform line should compress by >10x");
    }

    #[test]
    fn count_byte_semantics() {
        let rep = compress_line(&[1u8, 1, 1].repeat(10), 3);
        assert!(rep[0] <= 127);
        assert_eq!(rep[0], 9); // 10x repeat -> count-1 = 9
        let lit = compress_line(&[1u8, 2, 3, 4, 5, 6, 7, 8, 9], 3);
        assert!(lit[0] >= 128);
        assert_eq!(lit[0], (257 - 3) as u8); // 3-pixel literal
    }

    #[test]
    fn line_repeat_collapsing_and_splitting() {
        let h = PageHeader { width: 4, height: 10, bits_per_pixel: 24, ..Default::default() };
        let row = vec![0xabu8, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef];
        let page = encode_page(&h, std::iter::repeat(row.as_slice()).take(10)).unwrap();
        assert_eq!(page[HEADER_SIZE], 9); // 10 identical rows -> one repeat run, count-1=9

        let tall = PageHeader { width: 2, height: 300, bits_per_pixel: 24, ..Default::default() };
        let trow = vec![0u8; 6];
        let tpage = encode_page(&tall, std::iter::repeat(trow.as_slice()).take(300)).unwrap();
        let decoded = decode_document(&[MAGIC.as_slice(), &tpage].concat()).unwrap();
        assert_eq!(decoded[0].1.len(), 300, "300 identical rows splits across repeat runs correctly");
    }

    #[test]
    fn whole_document_round_trip_bit_exact() {
        fn make_rows(w: usize, h: usize, seed: u64) -> Vec<Vec<u8>> {
            let mut state = seed;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 256) as u8
            };
            (0..h)
                .map(|y| {
                    if y % 3 == 0 {
                        vec![0xffu8; w * 3]
                    } else {
                        (0..w * 3).map(|_| next()).collect()
                    }
                })
                .collect()
        }
        let h1 = PageHeader { width: 16, height: 12, total_pages: 2, ..Default::default() };
        let h2 = PageHeader { width: 16, height: 9, total_pages: 2, ..Default::default() };
        let rows1 = make_rows(16, 12, 1);
        let rows2 = make_rows(16, 9, 2);
        let doc = encode_document(&[(h1, rows1.clone()), (h2, rows2.clone())]).unwrap();
        assert!(doc.starts_with(MAGIC));
        let decoded = decode_document(&doc).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0.height, 12);
        assert_eq!(decoded[0].1, rows1, "page 1 pixels bit-exact");
        assert_eq!(decoded[1].1, rows2, "page 2 pixels bit-exact");
        assert_eq!(decoded[0].0.total_pages, 2);
    }

    #[test]
    fn malformed_input_rejected() {
        assert!(encode_page(&PageHeader { width: 4, height: 1, ..Default::default() }, std::iter::once([0u8; 5].as_slice())).is_err());
        assert!(decode_document(b"XXXX").is_err());
    }
}
