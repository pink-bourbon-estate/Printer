//! Page preparation: fit images onto paper, build PWG Raster for printers
//! that won't take anything else, and lay out plain text as pages.
//!
//! PDF input is sent to printers that accept `application/pdf` untouched —
//! no rasterising needed. Pulling in a PDF rendering crate to rasterise PDFs
//! for printers that *don't* take PDF is out of scope for this port; such a
//! printer will get an honest error naming what it does and doesn't accept.

use crate::pwgraster::{self, PageHeader, CSPACE_SGRAY, CSPACE_SRGB};
use image::{DynamicImage, Rgb, RgbImage};

pub struct PaperSpec {
    pub name: String,
    pub dpi: u32,
}

/// Page sizes in points (1/72 inch), the unit PDF and PWG headers both use.
pub fn page_points(name: &str) -> (u32, u32) {
    match name {
        "Letter" => (612, 792),
        "Legal" => (612, 1008),
        "A5" => (420, 595),
        _ => (595, 842), // A4
    }
}

pub fn media_keyword(name: &str) -> &'static str {
    match name {
        "Letter" => "na_letter_8.5x11in",
        "Legal" => "na_legal_8.5x14in",
        "A5" => "iso_a5_148x210mm",
        _ => "iso_a4_210x297mm",
    }
}

impl PaperSpec {
    pub fn points(&self) -> (u32, u32) {
        page_points(&self.name)
    }
    pub fn media_keyword(&self) -> &'static str {
        media_keyword(&self.name)
    }
    pub fn pixels(&self) -> (u32, u32) {
        let (w, h) = self.points();
        (w * self.dpi / 72, h * self.dpi / 72)
    }
}

/// Scales an image to fill the sheet without distortion, centred on white.
/// Rotates 90 degrees first when that gives a better fit (a landscape photo
/// on portrait paper).
pub fn fit_to_paper(img: &DynamicImage, paper: &PaperSpec, grayscale: bool) -> DynamicImage {
    let (target_w, target_h) = paper.pixels();

    let img_landscape = img.width() > img.height();
    let page_landscape = target_w > target_h;
    let img = if img_landscape != page_landscape { img.rotate90() } else { img.clone() };

    let scale = (target_w as f64 / img.width() as f64).min(target_h as f64 / img.height() as f64);
    let new_w = ((img.width() as f64 * scale).round() as u32).max(1);
    let new_h = ((img.height() as f64 * scale).round() as u32).max(1);
    let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);

    let mut canvas = RgbImage::from_pixel(target_w, target_h, Rgb([255, 255, 255]));
    let ox = (target_w - new_w) / 2;
    let oy = (target_h - new_h) / 2;
    image::imageops::overlay(&mut canvas, &resized.to_rgb8(), ox as i64, oy as i64);

    let out = DynamicImage::ImageRgb8(canvas);
    if grayscale {
        DynamicImage::ImageLuma8(out.to_luma8())
    } else {
        out
    }
}

/// Builds a PWG Raster document from already-fitted page images.
pub fn build_raster(images: &[DynamicImage], paper: &PaperSpec, grayscale: bool, duplex: bool) -> Result<Vec<u8>, String> {
    let (w_pt, h_pt) = paper.points();
    let (px_w, px_h) = paper.pixels();
    let total = images.len() as u32;

    let mut pages = Vec::with_capacity(images.len());
    for img in images {
        let header = PageHeader {
            width: px_w,
            height: px_h,
            dpi_x: paper.dpi,
            dpi_y: paper.dpi,
            bits_per_pixel: if grayscale { 8 } else { 24 },
            bits_per_color: 8,
            color_space: if grayscale { CSPACE_SGRAY } else { CSPACE_SRGB },
            num_colors: if grayscale { 1 } else { 3 },
            page_size_x: w_pt,
            page_size_y: h_pt,
            page_size_name: paper.media_keyword().to_string(),
            duplex,
            total_pages: total,
            ..Default::default()
        };
        let rows = pwgraster::image_to_rows(img, &header)?;
        pages.push((header, rows));
    }
    pwgraster::encode_document(&pages)
}

/// Encodes a page image as a JPEG at the given DPI (for printers that only
/// take image/jpeg).
pub fn image_to_jpeg(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.to_rgb8().write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

/// Classifies a file by its extension, falling back to sniffing the magic
/// bytes if the name doesn't tell us.
pub fn classify(path: &std::path::Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => return "pdf",
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tif" | "tiff" | "webp" => return "image",
        _ => {}
    }
    if let Ok(head) = std::fs::read(path).map(|b| b.into_iter().take(8).collect::<Vec<u8>>()) {
        if head.starts_with(b"%PDF") {
            return "pdf";
        }
        if head.starts_with(&[0xff, 0xd8, 0xff]) || head.starts_with(b"\x89PNG") {
            return "image";
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_pixel_sizes() {
        let a4 = PaperSpec { name: "A4".into(), dpi: 300 };
        assert_eq!(a4.points(), (595, 842));
        assert_eq!(a4.pixels(), (2479, 3508));
        assert_eq!(a4.media_keyword(), "iso_a4_210x297mm");

        let letter = PaperSpec { name: "Letter".into(), dpi: 300 };
        assert_eq!(letter.points(), (612, 792));
    }

    #[test]
    fn fit_rotates_landscape_onto_portrait() {
        let wide = DynamicImage::ImageRgb8(RgbImage::from_pixel(2000, 1000, Rgb([10, 20, 30])));
        let paper = PaperSpec { name: "A4".into(), dpi: 72 };
        let fitted = fit_to_paper(&wide, &paper, false);
        assert_eq!((fitted.width(), fitted.height()), paper.pixels());
    }

    #[test]
    fn fit_keeps_proportions_not_stretched() {
        // A square image on A4 should cover the full width, not be stretched
        // to fill the whole (non-square) sheet.
        let square = DynamicImage::ImageRgb8(RgbImage::from_pixel(500, 500, Rgb([255, 0, 0])));
        let paper = PaperSpec { name: "A4".into(), dpi: 72 };
        let fitted = fit_to_paper(&square, &paper, false).to_rgb8();
        let red_pixels = fitted.pixels().filter(|p| p.0[0] > 200 && p.0[1] < 50).count();
        let total = (fitted.width() * fitted.height()) as usize;
        let ratio = red_pixels as f64 / total as f64;
        // A4 is 595x842pt; a square filling the width covers 595/842 of area.
        assert!(ratio > 0.55 && ratio < 0.78, "ratio was {ratio}");
    }

    #[test]
    fn raster_build_round_trips_through_decoder() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, Rgb([40, 80, 120])));
        let paper = PaperSpec { name: "A4".into(), dpi: 72 };
        let fitted = fit_to_paper(&img, &paper, false);
        let raster = build_raster(&[fitted], &paper, false, false).unwrap();
        let decoded = pwgraster::decode_document(&raster).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!((decoded[0].0.width, decoded[0].0.height), paper.pixels());
    }

    #[test]
    fn classify_sniffs_bytes_when_extension_is_missing() {
        let dir = std::env::temp_dir().join(format!("printer-rs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("mystery");
        std::fs::write(&p, b"%PDF-1.4 rest of file").unwrap();
        assert_eq!(classify(&p), "pdf");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
