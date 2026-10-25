//! Imagens: guardadas em `media/`, nomeadas pelo hash do conteúdo (a mesma
//! imagem colada duas vezes vira um arquivo só).

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct Stored {
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    pub size: u64,
    pub mime: &'static str,
}

fn ext_mime(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some(("png", "image/png")),
        image::ImageFormat::Jpeg => Some(("jpg", "image/jpeg")),
        image::ImageFormat::WebP => Some(("webp", "image/webp")),
        image::ImageFormat::Gif => Some(("gif", "image/gif")),
        image::ImageFormat::Bmp => Some(("bmp", "image/bmp")),
        _ => None,
    }
}

/// Salva bytes de imagem (png/jpg/webp/gif/bmp) em `dir`.
pub fn store_bytes(dir: &Path, bytes: &[u8]) -> Result<Stored> {
    let (mut ext, mut mime) = ext_mime(bytes).with_context(|| crate::tr!("media.unsupported"))?;
    let img = image::load_from_memory(bytes).with_context(|| crate::tr!("media.invalid"))?;
    let (width, height) = (img.width(), img.height());
    // BMP vira PNG (menor e lido em qualquer lugar).
    let owned;
    let bytes = if ext == "bmp" {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
        ext = "png";
        mime = "image/png";
        owned = out;
        &owned[..]
    } else {
        bytes
    };
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().take(12).map(|b| format!("{b:02x}")).collect();
    let file_name = format!("{hex}.{ext}");
    std::fs::create_dir_all(dir)?;
    let path = dir.join(&file_name);
    if !path.exists() {
        std::fs::write(&path, bytes).with_context(|| crate::tr!("media.write_failed", path = path.display()))?;
    }
    Ok(Stored { file_name, width, height, size: bytes.len() as u64, mime })
}

pub fn store_file(dir: &Path, src: &Path) -> Result<Stored> {
    let bytes = std::fs::read(src).with_context(|| crate::tr!("media.read_failed", path = src.display()))?;
    store_bytes(dir, &bytes)
}

/// Imagem colada (Ctrl+V), em RGBA.
pub fn store_rgba(dir: &Path, width: u32, height: u32, rgba: Vec<u8>) -> Result<Stored> {
    let buf = image::RgbaImage::from_raw(width, height, rgba).with_context(|| crate::tr!("media.paste_invalid"))?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(buf).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    store_bytes(dir, &out)
}

pub fn is_image_path(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
    )
}

/// URI que o carregador de imagens do egui entende. No Windows precisa de três
/// barras (`file:///C:/...`): com duas, o egui lê `C:` como servidor de rede.
pub fn uri(dir: &Path, file_name: &str) -> String {
    let p: PathBuf = dir.join(file_name);
    let s = p.display().to_string().replace('\\', "/");
    if s.starts_with('/') { format!("file://{s}") } else { format!("file:///{s}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_paths_get_three_slashes() {
        let u = super::uri(std::path::Path::new(r"C:\Users\x\media"), "a.png");
        assert_eq!(u, "file:///C:/Users/x/media/a.png");
    }
}
