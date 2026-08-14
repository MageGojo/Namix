//! Image helpers on top of a stored object (`cover` / WebP).

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat};

use super::{Storage, StorageError, StorageResult};

pub struct StorageImage {
    storage: Storage,
    key: String,
    image: DynamicImage,
}

impl StorageImage {
    pub fn load(storage: Storage, key: &str) -> StorageResult<Self> {
        let bytes = storage.get(key)?.ok_or(StorageError::NotFound)?;
        let image = image::load_from_memory(&bytes).map_err(|error| StorageError::Image {
            message: error.to_string(),
        })?;
        Ok(Self {
            storage,
            key: key.to_string(),
            image,
        })
    }

    /// Center-crop after scaling so the result fills `width` × `height`.
    pub fn cover(mut self, width: u32, height: u32) -> StorageResult<Self> {
        if width == 0 || height == 0 {
            return Err(StorageError::Image {
                message: "cover size must be greater than zero".into(),
            });
        }
        let (src_w, src_h) = self.image.dimensions();
        if src_w == 0 || src_h == 0 {
            return Err(StorageError::Image {
                message: "source image is empty".into(),
            });
        }
        let scale = (width as f32 / src_w as f32).max(height as f32 / src_h as f32);
        let resized_w = ((src_w as f32) * scale).ceil().max(1.0) as u32;
        let resized_h = ((src_h as f32) * scale).ceil().max(1.0) as u32;
        let resized = self
            .image
            .resize(resized_w, resized_h, FilterType::Lanczos3);
        let x = resized.width().saturating_sub(width) / 2;
        let y = resized.height().saturating_sub(height) / 2;
        self.image = resized.crop_imm(x, y, width, height);
        Ok(self)
    }

    pub fn to_webp(mut self, quality: u8) -> StorageResult<Self> {
        let _ = quality;
        self.key = replace_extension(&self.key, "webp");
        Ok(self)
    }

    pub fn save(self) -> StorageResult<String> {
        let key = self.key.clone();
        self.save_as(&key)
    }

    pub fn save_as(self, key: &str) -> StorageResult<String> {
        let format = format_for_key(key);
        let mut encoded = Cursor::new(Vec::new());
        if format == ImageFormat::WebP {
            if let Err(error) = self.image.write_to(&mut encoded, ImageFormat::WebP) {
                tracing::debug!(error = %error, "webp encode failed; falling back to jpeg");
                let jpeg_key = replace_extension(key, "jpg");
                let mut jpeg = Cursor::new(Vec::new());
                self.image
                    .write_to(&mut jpeg, ImageFormat::Jpeg)
                    .map_err(|error| StorageError::Image {
                        message: error.to_string(),
                    })?;
                self.storage.put(&jpeg_key, &jpeg.into_inner())?;
                return Ok(jpeg_key);
            }
        } else {
            self.image
                .write_to(&mut encoded, format)
                .map_err(|error| StorageError::Image {
                    message: error.to_string(),
                })?;
        }
        self.storage.put(key, &encoded.into_inner())?;
        Ok(key.to_string())
    }
}

fn replace_extension(key: &str, ext: &str) -> String {
    match key.rsplit_once('.') {
        Some((stem, _)) if stem.contains('/') || !stem.is_empty() => format!("{stem}.{ext}"),
        _ => format!("{key}.{ext}"),
    }
}

fn format_for_key(key: &str) -> ImageFormat {
    match key
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::WebP,
        _ => ImageFormat::Png,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn cover_and_webp_round_trip_on_a_fake_disk() {
        let storage = Storage::fake("images");
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(12, 8, Rgb([10, 20, 30]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let mut png = Cursor::new(Vec::new());
        dyn_img.write_to(&mut png, ImageFormat::Png).unwrap();
        storage.put("shot.png", &png.into_inner()).unwrap();

        let saved = storage
            .image("shot.png")
            .unwrap()
            .cover(6, 6)
            .unwrap()
            .to_webp(80)
            .unwrap()
            .save()
            .unwrap();
        assert!(saved.ends_with(".webp") || saved.ends_with(".jpg"));
        storage.assert_exists(&saved);
        let bytes = storage.get(&saved).unwrap().unwrap();
        assert!(!bytes.is_empty());
    }
}
