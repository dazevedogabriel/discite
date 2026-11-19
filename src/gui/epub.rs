//! EPUB 3 reflowable de uma página: o mesmo XHTML da exportação em HTML, com
//! imagens e fontes como arquivos dentro do zip.

use super::export::{Assets, document, esc, style};
use super::theme::FONTS;
use crate::db::Block;
use eframe::egui;
use std::path::Path;

pub fn page_epub(ctx: &egui::Context, title: &str, blocks: &[Block], media: &Path) -> Vec<u8> {
    let mut assets = Assets { media, files: Some(vec![]) };
    let page = document(ctx, title, blocks, &mut assets, r#"<link rel="stylesheet" type="text/css" href="style.css"/>"#);
    let images = assets.files.unwrap_or_default();
    let title = esc(title.trim());
    let lang = crate::i18n::current();
    let id = format!("urn:discite:{}", crate::db::new_id());
    let modified = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    let mut manifest = String::new();
    let svg = if page.contains("<svg") { r#" properties="svg""# } else { "" };
    manifest.push_str(&format!(r#"<item id="page" href="text.xhtml" media-type="application/xhtml+xml"{svg}/>"#));
    manifest.push_str(r#"<item id="nav" href="toc.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="style.css" media-type="text/css"/>"#);
    for (i, (path, _)) in images.iter().enumerate() {
        manifest.push_str(&format!(r#"<item id="img{i}" href="{path}" media-type="{}"/>"#, super::export::mime(path)));
    }
    for f in FONTS {
        manifest.push_str(&format!(r#"<item id="font-{0}" href="fonts/{0}.ttf" media-type="font/ttf"/>"#, f.name));
    }
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id" xml:lang="{lang}"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">{id}</dc:identifier><dc:title>{title}</dc:title><dc:language>{lang}</dc:language><meta property="dcterms:modified">{modified}</meta></metadata><manifest>{manifest}</manifest><spine><itemref idref="page"/></spine></package>
"#
    );
    let nav = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{lang}" xml:lang="{lang}"><head><meta charset="utf-8"/><title>{title}</title></head><body><nav epub:type="toc"><ol><li><a href="text.xhtml">{title}</a></li></ol></nav></body></html>
"#
    );
    let container = r#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>
"#;
    let page = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{page}");
    let css = style(true);

    // O `mimetype` vem primeiro e sem compressão (exigência do EPUB).
    let mut files: Vec<(String, &[u8])> = vec![
        ("mimetype".into(), b"application/epub+zip"),
        ("META-INF/container.xml".into(), container.as_bytes()),
        ("OEBPS/content.opf".into(), opf.as_bytes()),
        ("OEBPS/toc.xhtml".into(), nav.as_bytes()),
        ("OEBPS/text.xhtml".into(), page.as_bytes()),
        ("OEBPS/style.css".into(), css.as_bytes()),
    ];
    for (path, bytes) in &images {
        files.push((format!("OEBPS/{path}"), bytes));
    }
    for f in FONTS {
        files.push((format!("OEBPS/fonts/{}.ttf", f.name), f.data));
    }
    zip_stored(&files)
}

/// Um zip sem compressão (imagens e fontes já vêm comprimidas; o EPUB aceita).
fn zip_stored(files: &[(String, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    for (name, data) in files {
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let size = data.len() as u32;
        // Cabeçalho local: versão 2.0, sem flags, "stored", data 1980-01-01.
        let fields = |out: &mut Vec<u8>| {
            for v in [20u16, 0, 0, 0, 0x21] {
                out.extend(v.to_le_bytes());
            }
            out.extend(crc.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
        };
        out.extend(0x04034b50u32.to_le_bytes());
        fields(&mut out);
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(0x02014b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        fields(&mut central);
        for v in [0u16, 0, 0] {
            central.extend(v.to_le_bytes());
        }
        central.extend(0u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x06054b50u32.to_le_bytes());
    for v in [0u16, 0, files.len() as u16, files.len() as u16] {
        out.extend(v.to_le_bytes());
    }
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::BlockKind;

    #[test]
    fn epub_is_a_zip_with_well_formed_xhtml() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        let blocks = vec![
            Block::new(BlockKind::Paragraph, "**negrito** <tag> & [[Link]]\nlinha 2"),
            Block::new(BlockKind::Paragraph, ""),
            Block::new(BlockKind::Formula, r"\frac{a}{b}"),
        ];
        let epub = page_epub(&ctx, "Título & \"aspas\"", &blocks, Path::new("."));
        assert!(epub.starts_with(b"PK\x03\x04") && epub[30..38] == *b"mimetype");
        // Cada XML do pacote precisa abrir num parser estrito.
        let text = String::from_utf8_lossy(&epub);
        for (start, close) in [("<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang", "</html>"), ("<package", "</package>"), ("<container", "</container>")] {
            let at = text.find(start).expect(start);
            let end = text[at..].find(close).unwrap() + close.len();
            let xml = text[at..at + end].replace("<!DOCTYPE html>", "");
            roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
        }
    }
}
