//! Pictures: drawn by the image model, kept as JPEG on tmpfile, whose links the pages show.
use crate::{
    ai,
    error::{AppError, Result},
    state::AppState,
};
use chrono::{DateTime, Utc};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

/// Drawn pictures are kept at their size, below this.
const DRAWN_SIDE: u32 = 2048;
/// What decoding one picture may allocate.
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
/// Smaller than this, a picture is a broken one.
const MIN_SIDE: u32 = 320;

/// A picture kept on tmpfile for a lesson.
#[derive(Debug)]
pub(crate) struct Picture {
    pub(crate) id: Uuid,
    pub(crate) url: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) expires_at: DateTime<Utc>,
}

/// Every picture is a JPEG.
const CONTENT_TYPE: &str = "image/jpeg";

/// A picture drawn to `prompt` and kept. A drawing slot is held from the request to the upload.
pub(crate) async fn drawn(state: &AppState, prompt: &str) -> Result<Picture> {
    let _slot = state
        .drawing_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("服务正在关闭".into()))?;
    let png = ai::draw(state, prompt).await?;
    let (jpeg, width, height) = tokio::task::spawn_blocking(move || jpeg(&png, DRAWN_SIDE))
        .await
        .map_err(|_| AppError::Internal("图片转换失败".into()))?
        .ok_or_else(|| AppError::Internal("图像模型返回的图片读不出来".into()))?;
    let stored = upload(state, jpeg).await?;
    Ok(Picture {
        id: Uuid::new_v4(),
        url: stored.direct_link,
        // Both sides are DRAWN_SIDE at most.
        width: width as i32,
        height: height as i32,
        expires_at: stored.expires_at,
    })
}

/// The picture as a JPEG at most `max_side` pixels on its longer side, transparency laid on
/// white, with its width and height. The whole file is decoded under size and memory limits, so
/// a picture that can't be read, or is too small to be one, gives `None`.
pub(crate) fn jpeg(bytes: &[u8], max_side: u32) -> Option<(Vec<u8>, u32, u32)> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width < MIN_SIDE || height < MIN_SIDE || decoder.total_bytes() > MAX_DECODE_BYTES {
        return None;
    }
    let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
    if decoded.width() > max_side || decoded.height() > max_side {
        decoded = decoded.thumbnail(max_side, max_side);
    }
    let decoded = if decoded.color().has_alpha() {
        let rgba = decoded.into_rgba8();
        image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let alpha = u16::from(pixel[3]);
            image::Rgb(std::array::from_fn(|channel| {
                ((u16::from(pixel[channel]) * alpha + 255 * (255 - alpha)) / 255) as u8
            }))
        })
    } else {
        decoded.into_rgb8()
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88)
        .encode_image(&decoded)
        .ok()?;
    Some((jpeg, decoded.width(), decoded.height()))
}

#[derive(Debug, Deserialize)]
struct Stored {
    direct_link: String,
    expires_at: DateTime<Utc>,
}

/// Keeps a JPEG on tmpfile. Its link must lead back to tmpfile itself: a proxy forwarding plain
/// HTTP inside may answer with an `http` link, which is read as the configured `https`.
async fn upload(state: &AppState, jpeg: Vec<u8>) -> Result<Stored> {
    let base = &state.config.tmpfile_base_url;
    let part = Part::bytes(jpeg)
        .file_name(format!("learn-{}.jpg", Uuid::new_v4()))
        .mime_str(CONTENT_TYPE)?;
    let stored: Stored = state
        .http
        .post(format!("{}/upload", base.as_str().trim_end_matches('/')))
        .bearer_auth(&state.config.tmpfile_api_key)
        .multipart(Form::new().part("file", part))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut link = Url::parse(&stored.direct_link)
        .map_err(|_| AppError::Internal("tmpfile 返回的地址无效".into()))?;
    if link.scheme() == "http"
        && base.scheme() == "https"
        && link.host_str() == base.host_str()
        && link.port() == base.port()
    {
        let _ = link.set_scheme("https");
    }
    if !matches!(link.scheme(), "http" | "https")
        || !link.username().is_empty()
        || link.password().is_some()
        || link.origin() != base.origin()
        || stored.expires_at <= Utc::now()
    {
        return Err(AppError::Internal("tmpfile 返回的地址不可用".into()));
    }
    Ok(Stored {
        direct_link: link.to_string(),
        expires_at: stored.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, alpha]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn a_drawing_becomes_a_jpeg_within_its_size() {
        let (converted, width, height) = jpeg(&png(1536, 1024, 255), 768).unwrap();
        assert_eq!(
            image::guess_format(&converted).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert_eq!((width, height), (768, 512));
        let decoded = image::load_from_memory(&converted).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (768, 512));
        // Transparency is laid on white, and a sliver or garbage is no picture.
        assert!(jpeg(&png(400, 400, 0), 2048).is_some());
        assert!(jpeg(&png(100, 1000, 255), 2048).is_none());
        assert!(jpeg(b"not a picture", 2048).is_none());
    }
}
