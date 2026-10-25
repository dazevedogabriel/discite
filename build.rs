//! Rasteriza `assets/logo.svg` para o ícone do executável (o que o Explorer,
//! a área de trabalho e os atalhos mostram) e para o ícone da janela. Para
//! trocar o ícone, substitua o SVG e compile de novo.

use resvg::{tiny_skia, usvg};
use std::path::PathBuf;

/// Tamanhos que o Windows escolhe para Explorer, barra de tarefas e HiDPI.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=assets/logo.svg");
    println!("cargo:rerun-if-changed=build.rs");
    let svg = std::fs::read("assets/logo.svg").expect("assets/logo.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("assets/logo.svg inválido");
    let pngs: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, render_png(&tree, s))).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("icon-256.png"), &pngs.last().unwrap().1).unwrap();
    // Ícone do app no macOS; o workflow de build procura `Discite.icns` em OUT_DIR.
    std::fs::write(out.join("Discite.icns"), icns_bytes(&tree)).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = out.join("discite.ico");
    std::fs::write(&ico, ico_bytes(&pngs)).unwrap();
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set("FileDescription", "Discite — caderno e flashcards");
    res.set("ProductName", "Discite");
    if let Err(e) = res.compile() {
        // Sem rc.exe o programa funciona igual, só sem ícone no Explorer.
        println!("cargo:warning=ícone não embutido: {e}");
    }
}

/// Centraliza o logo num quadrado transparente.
fn render_png(tree: &usvg::Tree, side: u32) -> Vec<u8> {
    let mut pix = tiny_skia::Pixmap::new(side, side).unwrap();
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height());
    let dx = (side as f32 - size.width() * scale) / 2.0;
    let dy = (side as f32 - size.height() * scale) / 2.0;
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pix.as_mut());
    pix.encode_png().unwrap()
}

/// Contêiner ICO com imagens PNG (suportado desde o Windows Vista).
fn ico_bytes(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (side, data) in images {
        // 256 é gravado como 0 nos campos de um byte.
        let dim = if *side >= 256 { 0 } else { *side as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}

/// Contêiner ICNS com PNGs, nos tamanhos (e versões @2x) que o macOS procura.
fn icns_bytes(tree: &usvg::Tree) -> Vec<u8> {
    const ENTRIES: [(&[u8; 4], u32); 11] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"icp6", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic13", 256),
        (b"ic14", 512),
    ];
    let mut body = Vec::new();
    for (kind, side) in ENTRIES {
        let png = render_png(tree, side);
        body.extend_from_slice(kind);
        body.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(&body);
    out
}
