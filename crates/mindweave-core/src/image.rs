//! Pictures placed on the canvas. An image is a shape (library
//! [`crate::shape::TEXT_LIBRARY`], shape [`IMAGE_SHAPE`]) holding a
//! picture file's bytes, so a document carries its images with it. In
//! memory the bytes are shared, so copies of a page (undo steps) do not
//! copy them; files store each distinct picture once.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::shape::Shape;

/// Shape id of an image in [`crate::shape::TEXT_LIBRARY`].
pub const IMAGE_SHAPE: &str = "image";
/// Largest picture file accepted, in bytes.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// Longest side of a newly placed image, in canvas units.
pub const MAX_PLACED_SIDE: f32 = 600.0;

/// A picture file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Tiff,
    Svg,
}

impl ImageFormat {
    /// Extensions the import dialog offers.
    pub const EXTENSIONS: [&str; 9] = [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "svg",
    ];

    /// The format of `bytes`, told from their first bytes; SVG is text
    /// with an `<svg` element near the start.
    pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
        let starts = |m: &[u8]| bytes.starts_with(m);
        if starts(b"\x89PNG\r\n\x1a\n") {
            Some(ImageFormat::Png)
        } else if starts(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageFormat::Jpeg)
        } else if starts(b"GIF87a") || starts(b"GIF89a") {
            Some(ImageFormat::Gif)
        } else if bytes.len() >= 12 && starts(b"RIFF") && &bytes[8..12] == b"WEBP" {
            Some(ImageFormat::Webp)
        } else if starts(b"BM") {
            Some(ImageFormat::Bmp)
        } else if starts(b"II*\0") || starts(b"MM\0*") {
            Some(ImageFormat::Tiff)
        } else {
            let head = &bytes[..bytes.len().min(4096)];
            let text = String::from_utf8_lossy(head);
            text.contains("<svg").then_some(ImageFormat::Svg)
        }
    }
}

/// A picture: its format, an id naming its bytes, and the bytes. In
/// files and clipboard payloads a shape stores only the format and id;
/// the bytes are stored once per picture in an [`ImageStore`] beside the
/// shapes (see [`collect_images`] and [`resolve_images`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    pub format: ImageFormat,
    /// [`image_id`] of the bytes.
    pub id: String,
    /// Empty until resolved from an [`ImageStore`].
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
}

impl ImageData {
    /// The picture in `bytes`, if they are a supported format no larger
    /// than [`MAX_IMAGE_BYTES`].
    pub fn from_file_bytes(bytes: Vec<u8>) -> Option<ImageData> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        Some(ImageData {
            format: ImageFormat::sniff(&bytes)?,
            id: image_id(&bytes),
            bytes: Arc::new(bytes),
        })
    }

    /// A number naming the picture's bytes, for caches.
    pub fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.id.hash(&mut h);
        h.finish()
    }
}

/// A stable name for picture `bytes`: their length and 64-bit FNV-1a
/// hash, in hex. The same bytes always get the same id, so copies of a
/// picture share one stored copy.
pub fn image_id(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:x}-{hash:016x}", bytes.len())
}

/// A stored picture: its format and bytes (base64 in JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredImage {
    pub format: ImageFormat,
    #[serde(serialize_with = "to_base64", deserialize_with = "from_base64")]
    pub bytes: Arc<Vec<u8>>,
}

/// Pictures by id, each stored once.
pub type ImageStore = BTreeMap<String, StoredImage>;

/// One stored copy of each picture the `shapes` show.
pub fn collect_images<'a>(shapes: impl IntoIterator<Item = &'a Shape>) -> ImageStore {
    let mut store = ImageStore::new();
    for data in shapes.into_iter().flat_map(|s| s.pictures()) {
        if !data.bytes.is_empty() {
            store.entry(data.id.clone()).or_insert_with(|| StoredImage {
                format: data.format,
                bytes: data.bytes.clone(),
            });
        }
    }
    store
}

/// Gives each of `shapes` the bytes of its picture from `store`. A shape
/// whose picture is missing, or stored under a different format, loses
/// it and is left an empty box.
pub fn resolve_images<'a>(shapes: impl IntoIterator<Item = &'a mut Shape>, store: &ImageStore) {
    for shape in shapes {
        for slot in shape.pictures_mut() {
            let Some(data) = slot.as_mut() else {
                continue;
            };
            match store.get(&data.id) {
                Some(stored) if stored.format == data.format => data.bytes = stored.bytes.clone(),
                _ => *slot = None,
            }
        }
    }
}

fn to_base64<S: Serializer>(bytes: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes.as_slice()))
}

fn from_base64<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
    let text = String::deserialize(d)?;
    base64::engine::general_purpose::STANDARD
        .decode(text.as_bytes())
        .map(Arc::new)
        .map_err(serde::de::Error::custom)
}

/// The size a picture `w × h` pixels is placed at: its own size, scaled
/// down so neither side exceeds [`MAX_PLACED_SIDE`] or `fit` (the visible
/// canvas, in canvas units).
pub fn placed_size((w, h): (f32, f32), fit: (f32, f32)) -> (f32, f32) {
    if !(w > 0.0 && h > 0.0) {
        return (MAX_PLACED_SIDE / 2.0, MAX_PLACED_SIDE / 2.0);
    }
    let limit_w = MAX_PLACED_SIDE.min(fit.0.max(1.0));
    let limit_h = MAX_PLACED_SIDE.min(fit.1.max(1.0));
    let scale = (limit_w / w).min(limit_h / h).min(1.0);
    (w * scale, h * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_told_from_their_first_bytes() {
        assert_eq!(
            ImageFormat::sniff(b"\x89PNG\r\n\x1a\nrest"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::sniff(b"GIF89a.."), Some(ImageFormat::Gif));
        assert_eq!(
            ImageFormat::sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
        assert_eq!(ImageFormat::sniff(b"MM\0*...."), Some(ImageFormat::Tiff));
        assert_eq!(
            ImageFormat::sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"/>"),
            Some(ImageFormat::Svg)
        );
        assert_eq!(ImageFormat::sniff(b"hello"), None);
        assert!(ImageData::from_file_bytes(Vec::new()).is_none());
        assert!(ImageData::from_file_bytes(vec![0; MAX_IMAGE_BYTES + 1]).is_none());
    }

    #[test]
    fn copies_of_a_picture_are_stored_once() {
        let pic = ImageData::from_file_bytes(b"GIF89a\x01\x02".to_vec()).unwrap();
        let other = ImageData::from_file_bytes(b"GIF89a\x03".to_vec()).unwrap();
        assert_eq!(pic.id, image_id(b"GIF89a\x01\x02"));
        assert_ne!(pic.id, other.id);
        let frame = crate::geometry::Frame::default();
        let mut shapes = vec![
            Shape::new_image(pic.clone(), frame),
            Shape::new_image(pic.clone(), frame),
            Shape::new_image(other.clone(), frame),
            Shape::new_text_box((0.0, 0.0)),
        ];
        let store = collect_images(&shapes);
        assert_eq!(store.len(), 2);
        assert_eq!(
            serde_json::to_value(&store[&pic.id]).unwrap(),
            serde_json::json!({"format": "gif", "bytes": "R0lGODlhAQI="})
        );
        // A shape stores only the picture's format and id.
        let json = serde_json::to_value(&shapes[0]).unwrap();
        assert_eq!(
            json["image"],
            serde_json::json!({"format": "gif", "id": pic.id})
        );
        let mut read: Vec<Shape> = shapes
            .iter()
            .map(|s| serde_json::from_value(serde_json::to_value(s).unwrap()).unwrap())
            .collect();
        assert!(read[0].image.as_ref().unwrap().bytes.is_empty());
        resolve_images(&mut read, &store);
        assert_eq!(read, shapes);
        // Both copies share the one stored picture.
        let (a, b) = (
            &read[0].image.as_ref().unwrap().bytes,
            &read[1].image.as_ref().unwrap().bytes,
        );
        assert!(Arc::ptr_eq(a, b));
        // A picture missing from the store leaves an empty box.
        resolve_images(&mut shapes[2..3], &ImageStore::new());
        assert!(shapes[2].image.is_none() && shapes[2].is_image());
    }

    #[test]
    fn placed_images_keep_their_shape_and_fit() {
        assert_eq!(
            placed_size((200.0, 100.0), (1000.0, 1000.0)),
            (200.0, 100.0)
        );
        assert_eq!(
            placed_size((1200.0, 600.0), (1000.0, 1000.0)),
            (600.0, 300.0)
        );
        assert_eq!(placed_size((400.0, 800.0), (1000.0, 300.0)), (150.0, 300.0));
    }
}
