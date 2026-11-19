//! PDF de uma página do caderno sem navegador: a página vira um documento
//! Typst (diagramação embutida no app) com as fontes, cores e medidas do
//! editor no tema claro, e o Typst grava o PDF. Fórmulas e fluxogramas entram
//! como o mesmo SVG da exportação em HTML.

use super::theme::{FONTS, ink_bg_on, ink_on, light, soft};
use crate::db::{Block, BlockKind};
use crate::rich::{self, Ink};
use crate::tr;
use anyhow::{Result, anyhow};
use eframe::egui::{self, Color32};
use std::path::Path;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt};

/// Largura útil da folha A4 com as margens abaixo, em pt (= os 688 px do editor).
const CONTENT_PT: f32 = 516.0;
/// px da tela → pt do PDF (o CSS usa 96 px por polegada).
const PT: f32 = 0.75;

/// O documento pronto para virar PDF: o fonte Typst e os arquivos que ele usa.
pub struct Doc {
    source: String,
    files: Vec<(String, Vec<u8>)>,
}

fn col(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    format!(r##"rgb("#{r:02x}{g:02x}{b:02x}{a:02x}")"##)
}

/// Texto como string Typst (dentro de código, nada é interpretado como marcação).
fn s(t: &str) -> String {
    let mut out = String::with_capacity(t.len() + 2);
    out.push('"');
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' | '\u{E000}' | '\u{E001}' => {}
            '\t' => out.push_str("    "),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Doc {
    /// Guarda os bytes como arquivo virtual e devolve o caminho para `image()`.
    fn file(&mut self, ext: &str, bytes: Vec<u8>) -> String {
        let path = format!("/f{}.{ext}", self.files.len());
        self.files.push((path.clone(), bytes));
        path
    }

    /// Uma imagem de `media/` com a largura natural (no máximo `max` pt).
    fn image(&mut self, media: &Path, name: &str, max: f32) -> Option<String> {
        let full = media.join(name);
        let bytes = std::fs::read(&full).ok()?;
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let (w, _) = image::image_dimensions(&full).unwrap_or((800, 600));
        let (ext, bytes) = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => (ext, bytes),
            // O Typst não lê BMP e afins: vira PNG.
            _ => {
                let img = image::load_from_memory(&bytes).ok()?;
                let mut png = std::io::Cursor::new(vec![]);
                img.write_to(&mut png, image::ImageFormat::Png).ok()?;
                ("png".to_owned(), png.into_inner())
            }
        };
        let path = self.file(&ext, bytes);
        Some(format!("image({}, width: {:.1}pt)", s(&path), (w as f32 * PT).min(max)))
    }

    /// Um SVG (fórmula, fluxograma) com a largura dele, no máximo `max` pt.
    fn svg(&mut self, svg: String, max: f32) -> String {
        let w = svg.split("width=\"").nth(1).and_then(|r| r.split('"').next()).and_then(|v| v.parse::<f32>().ok()).unwrap_or(300.0);
        let path = self.file("svg", svg.into_bytes());
        format!("image({}, width: {:.1}pt)", s(&path), (w * PT).min(max))
    }

    /// Texto rico como um bloco de código Typst que junta os trechos.
    fn rich(&mut self, text: &str, media: &Path) -> String {
        let p = light();
        let mut parts: Vec<String> = vec![];
        for seg in rich::segments(text) {
            match seg {
                rich::Seg::Image(name) => {
                    if let Some(img) = self.image(media, &name, CONTENT_PT) {
                        parts.push(format!("block(above: 4.5pt, below: 4.5pt, radius: 4.5pt, clip: true, {img})"));
                    }
                }
                rich::Seg::Text(t) => {
                    if !parts.is_empty() {
                        parts.push("linebreak()".into());
                    }
                    for r in rich::parse(&t) {
                        if r.marker || r.text.is_empty() {
                            continue;
                        }
                        let st = r.style;
                        for (i, line) in r.text.split('\n').enumerate() {
                            if i > 0 {
                                parts.push("linebreak()".into());
                            }
                            if line.is_empty() {
                                continue;
                            }
                            let mut args = String::new();
                            if st.bold {
                                args.push_str("weight: 600, ");
                            }
                            if st.italic {
                                args.push_str("style: \"italic\", ");
                            }
                            if let Some(i) = st.ink {
                                args.push_str(&format!("fill: {}, ", col(ink_on(i, false))));
                            } else if st.link {
                                args.push_str(&format!("fill: {}, ", col(p.accent)));
                            }
                            let mut e = if st.code {
                                format!(
                                    "box(fill: {}, inset: (x: 2.25pt), outset: (y: 1.5pt), radius: 2.25pt, text({args}font: \"JetBrains Mono\", size: 0.84em, {}))",
                                    col(p.chip),
                                    s(line)
                                )
                            } else {
                                format!("text({args}{})", s(line))
                            };
                            if st.underline {
                                e = format!("underline({e})");
                            }
                            if st.link {
                                e = format!("underline(stroke: 0.75pt + {}, offset: 2pt, {e})", col(soft(p.accent, 0.5)));
                            }
                            if st.strike {
                                e = format!("strike({e})");
                            }
                            if let Some(i) = st.bg {
                                e = format!("highlight(fill: {}, {e})", col(ink_bg_on(i, false)));
                            } else if st.highlight {
                                e = format!("highlight(fill: {}, {e})", col(p.mark));
                            }
                            parts.push(e);
                        }
                    }
                }
            }
        }
        format!("{{ {} }}", parts.join("; "))
    }
}

/// Os rótulos em versalete das molduras (DICA, CÓDIGO, FÓRMULA).
fn caps(label: &str, color: Color32) -> String {
    format!("text(size: 7.9pt, weight: 600, tracking: 0.79pt, fill: {}, upper({}))", col(color), s(label))
}

/// Monta o documento Typst da página. `ctx` mede o texto das fórmulas e dos
/// fluxogramas com as mesmas fontes da tela.
pub fn page_doc(ctx: &egui::Context, title: &str, blocks: &[Block], media: &Path) -> Doc {
    let p = light();
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
    let numbers = super::page::numbering(blocks);
    let mut d = Doc { source: String::new(), files: vec![] };
    let mut src = format!(
        r#"#set document(title: {title})
#set page(paper: "a4", margin: (x: 14mm, y: 16mm), fill: white)
#set text(font: ("Literata", "Noto Sans Math"), size: 12pt, fill: {text2}, lang: {lang})
#set par(leading: 0.95em, justify: false)
#set block(spacing: 7.5pt)
#set par(spacing: 7.5pt)
#block(below: 16.5pt, text(size: 25.5pt, weight: 600, fill: {text}, {title}))
"#,
        title = s(title.trim()),
        text2 = col(p.text2),
        text = col(p.text),
        lang = s(crate::i18n::current().split('-').next().unwrap_or("pt")),
    );
    let item = |indent: u8, mark: String, body: String| format!("pad(left: {}pt, grid(columns: (18pt, 1fr), {mark}, {body}))", indent as u32 * 18);
    for (b, n) in blocks.iter().zip(numbers) {
        let heading = |d: &mut Doc, size: f32, above: f32| format!("block(above: {above}pt, below: 6pt, text(size: {size}pt, weight: 600, fill: {}, {}))", col(p.text), d.rich(&b.text, media));
        let code = match b.kind {
            BlockKind::Paragraph if b.text.trim().is_empty() => "v(12.4pt)".to_owned(),
            BlockKind::Paragraph => format!("block(width: 100%, {})", d.rich(&b.text, media)),
            BlockKind::H1 => heading(&mut d, 20.25, 13.5),
            BlockKind::H2 => heading(&mut d, 15.75, 13.5),
            BlockKind::H3 => heading(&mut d, 13.1, 13.5),
            BlockKind::Bullet => {
                let glyph = ["•", "◦", "▪"][b.indent as usize % 3];
                item(b.indent, format!("pad(left: 4.5pt, {})", s(glyph)), d.rich(&b.text, media))
            }
            BlockKind::Numbered => item(b.indent, format!("align(right, pad(right: 5.25pt, {}))", s(&format!("{n}."))), d.rich(&b.text, media)),
            BlockKind::Check => {
                let tick = format!("place(dx: 2.4pt, dy: 2.6pt, curve(stroke: 1.5pt + white, curve.move((0pt, 3pt)), curve.line((2.4pt, 5.4pt)), curve.line((6.6pt, 0.6pt))))");
                let (fill, stroke, inner) = if b.checked { (col(p.accent), col(p.accent), tick.as_str()) } else { ("none".into(), col(p.border_strong), "none") };
                let mark = format!("pad(left: 4.5pt, box(width: 11.25pt, height: 11.25pt, baseline: 1.5pt, fill: {fill}, stroke: 1pt + {stroke}, radius: 3pt, {inner}))");
                let body = d.rich(&b.text, media);
                let body = if b.checked { format!("text(fill: {}, {body})", col(p.muted)) } else { body };
                item(b.indent, mark, body)
            }
            BlockKind::Quote => format!(
                "block(inset: (left: 13.5pt, y: 1.5pt), stroke: (left: 2.25pt + {}), text(style: \"italic\", {}))",
                col(p.border_strong),
                d.rich(&b.text, media)
            ),
            BlockKind::Callout => {
                let label = Ink::from_name(&b.ink).map_or(p.accent, |k| ink_on(k, false));
                let (fill, edge) = Ink::from_name(&b.fill).map_or((p.accent_tint, p.border), |k| (soft(ink_on(k, false), 0.14), soft(ink_on(k, false), 0.35)));
                format!(
                    "block(width: 100%, breakable: false, fill: {}, stroke: 0.75pt + {}, radius: 7.5pt, inset: (x: 10.5pt, y: 9pt), grid(columns: (39pt, 1fr), pad(top: 2.5pt, {}), text(size: 11.25pt, {})))",
                    col(fill),
                    col(edge),
                    caps(&tr!("page.tip"), label),
                    d.rich(&b.text, media)
                )
            }
            BlockKind::Code => {
                let label = if b.lang.is_empty() { tr!("page.code") } else { b.lang.clone() };
                let lines: Vec<String> = b.text.split('\n').map(s).collect();
                format!(
                    "block(width: 100%, breakable: false, fill: {}, stroke: 0.75pt + {}, radius: 6pt, inset: (x: 10.5pt, y: 7.5pt), {{ block(below: 4.5pt, {}); set par(leading: 0.55em); text(font: \"JetBrains Mono\", size: 9.75pt, fill: {}, {{ {} }}) }})",
                    col(p.input),
                    col(p.border),
                    caps(&label, p.dim),
                    col(p.text),
                    lines.join("; linebreak(); ")
                )
            }
            BlockKind::Formula => match super::math::svg(&painter, &b.text, 20.0, p.text) {
                Some(svg) => format!(
                    "block(width: 100%, breakable: false, fill: {}, stroke: 0.75pt + {}, radius: 6pt, inset: (x: 10.5pt, y: 7.5pt), {{ block(below: 4.5pt, {}); align(center, {}) }})",
                    col(p.input),
                    col(p.border),
                    caps(&tr!("page.formula_label"), p.accent),
                    d.svg(svg, CONTENT_PT - 21.0)
                ),
                None => continue,
            },
            BlockKind::Divider => format!("block(above: 12pt, below: 12pt, line(length: 100%, stroke: 0.75pt + {}))", col(p.border_strong)),
            BlockKind::Image => {
                let Some(img) = d.image(media, &b.media, f32::INFINITY) else { continue };
                // A largura escolhida no editor vale como fração da coluna.
                let img = img.rsplit_once(", width:").map_or(img.clone(), |(head, _)| format!("{head}, width: 100%)"));
                let cap = if b.caption.trim().is_empty() {
                    String::new()
                } else {
                    format!("; v(3pt); text(style: \"italic\", size: 9.75pt, fill: {}, {})", col(p.muted), s(&b.caption))
                };
                format!(
                    "block(width: 100%, breakable: false, align(center, {{ block(width: {:.0}%, radius: 4.5pt, clip: true, {img}){cap} }}))",
                    b.width.clamp(0.25, 1.0) * 100.0
                )
            }
            BlockKind::Table => {
                let cols = b.rows.iter().map(Vec::len).max().unwrap_or(0);
                if cols == 0 {
                    continue;
                }
                let mut cells = vec![];
                for (i, row) in b.rows.iter().enumerate() {
                    for c in 0..cols {
                        let body = row.get(c).map_or("[]".to_owned(), |t| d.rich(t, media));
                        cells.push(if i == 0 { format!("text(weight: 600, fill: {}, {body})", col(p.text)) } else { body });
                    }
                }
                format!(
                    "block(width: 100%, breakable: false, stroke: 0.75pt + {}, radius: 6pt, clip: true, text(size: 10.5pt, table(columns: (1fr,) * {cols}, inset: (x: 7.5pt, y: 5.25pt), stroke: (x, y) => if y > 0 {{ (top: 0.75pt + {}) }}, fill: (x, y) => if y == 0 {{ {} }}, {})))",
                    col(p.border),
                    col(p.line_row),
                    col(p.chip),
                    cells.join(", ")
                )
            }
            BlockKind::Diagram => match super::diagram::svg(&painter, &b.diagram) {
                Some(svg) => format!(
                    "block(width: 100%, breakable: false, fill: {}, stroke: 0.75pt + {}, radius: 7.5pt, inset: 6pt, align(center, {}))",
                    col(p.input),
                    col(p.border),
                    d.svg(svg, CONTENT_PT - 12.0)
                ),
                None => continue,
            },
        };
        src.push('#');
        src.push_str(&code);
        src.push('\n');
    }
    d.source = src;
    d
}

/// O "mundo" do Typst: o fonte, as imagens da página e as fontes do app.
struct World {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
    files: Vec<(FileId, Bytes)>,
}

fn file_id(path: &str) -> FileId {
    RootedPath::new(VirtualRoot::Project, VirtualPath::new(path).expect("caminho virtual válido")).intern()
}

impl typst::World for World {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.iter().find(|(f, _)| *f == id).map(|(_, b)| b.clone()).ok_or(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Diagrama o documento e devolve os bytes do PDF.
pub fn render(doc: Doc) -> Result<Vec<u8>> {
    typst_pdf::pdf(&layout(doc)?, &typst_pdf::PdfOptions::default()).map_err(|e| fail(&e))
}

fn fail(errs: &[typst::diag::SourceDiagnostic]) -> anyhow::Error {
    anyhow!(errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))
}

fn layout(doc: Doc) -> Result<typst_layout::PagedDocument> {
    let fonts: Vec<Font> = FONTS.iter().flat_map(|f| Font::iter(Bytes::new(f.data))).collect();
    let world = World {
        library: LazyHash::new(Library::default()),
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
        main: Source::new(file_id("/main.typ"), doc.source),
        files: doc.files.into_iter().map(|(p, b)| (file_id(&p), Bytes::new(b))).collect(),
    };
    typst::compile(&world).output.map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_cannot_break_out_into_typst_code() {
        assert_eq!(s(r#"a "b" \ #c"#), r#""a \"b\" \\ #c""#);
        assert_eq!(s("x\ny"), r#""x\ny""#);
    }

    #[test]
    fn a_page_with_every_kind_of_block_becomes_a_pdf() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        let mut blocks = vec![];
        for (kind, text) in [
            (BlockKind::H1, "Título"),
            (BlockKind::Paragraph, "**negrito** _itálico_ {red|vermelho} {=yellow|realce} `código` [[Link]] ~~riscado~~ ⟶ \"aspas\" #hash"),
            (BlockKind::Bullet, "item"),
            (BlockKind::Numbered, "um"),
            (BlockKind::Check, "feito"),
            (BlockKind::Quote, "citação"),
            (BlockKind::Callout, "dica"),
            (BlockKind::Code, "fn main() {\n    println!(\"oi\");\n}"),
            (BlockKind::Formula, r"\frac{a}{b} + x^2"),
            (BlockKind::Divider, ""),
        ] {
            let mut b = Block::new(kind, text);
            b.checked = kind == BlockKind::Check;
            blocks.push(b);
        }
        let mut t = Block::new(BlockKind::Table, "");
        t.rows = vec![vec!["A".into(), "B".into()], vec!["1".into()]];
        blocks.push(t);
        let pdf = render(page_doc(&ctx, "Teste \"PDF\"", &blocks, Path::new("."))).expect("o Typst gera o PDF");
        assert!(pdf.starts_with(b"%PDF"));
    }
}
