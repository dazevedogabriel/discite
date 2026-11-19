//! Exportar uma página do caderno com o visual dela: um HTML só, com fontes,
//! imagens, fórmulas e fluxogramas embutidos, sempre no tema claro. É também a
//! base de um EPUB reflowable (que é HTML + CSS). O PDF fica em `pdf.rs`.

use super::theme::{FONTS, ink_bg_on, ink_on, light, soft};
use crate::db::{Block, BlockKind};
use crate::rich::{self, Ink};
use crate::tr;
use eframe::egui::{self, Color32, Galley, Pos2};
use std::path::Path;

/// Cor CSS (o egui guarda as cores pré-multiplicadas).
pub(super) fn css(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0) }
}

pub(super) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A fonte de um texto do egui como atributos SVG.
fn font_attrs(f: &egui::FontId) -> String {
    let (family, weight, italic) = match &f.family {
        egui::FontFamily::Monospace => ("JetBrains Mono", 400, false),
        egui::FontFamily::Name(n) => match n.as_ref() {
            "medium" => ("Literata", 500, false),
            "semi" => ("Literata", 600, false),
            "bold" => ("Literata", 700, false),
            "italic" => ("Literata", 400, true),
            "semi-italic" => ("Literata", 600, true),
            _ => ("Literata", 400, false),
        },
        _ => ("Literata", 400, false),
    };
    format!(r#"font-family="{family}, 'Noto Sans Math'" font-size="{:.2}" font-weight="{weight}"{}"#, f.size, if italic { r#" font-style="italic""# } else { "" })
}

/// Um galley do egui como `<text>` SVG, uma linha por vez, na mesma linha de
/// base: o texto cai no mesmo lugar da tela, com a mesma fonte.
pub(super) fn text_svg(out: &mut String, g: &Galley, origin: Pos2, color: Color32) {
    let Some(section) = g.job.sections.first() else { return };
    let attrs = font_attrs(&section.format.font_id);
    for row in &g.rows {
        let Some(first) = row.row.glyphs.first() else { continue };
        let text: String = row.row.glyphs.iter().map(|gl| gl.chr).collect();
        let (x, y) = (origin.x + row.pos.x + first.pos.x, origin.y + row.pos.y + first.pos.y);
        out.push_str(&format!(r#"<text x="{x:.2}" y="{y:.2}" {attrs} fill="{}" xml:space="preserve">{}</text>"#, css(color), esc(&text)));
    }
}

pub(super) fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        _ => "image/jpeg",
    }
}

/// Onde vão as imagens: dentro do HTML como data URL (a página exportada não
/// depende de pastas) ou como arquivos à parte (o EPUB é um zip).
pub(super) struct Assets<'a> {
    pub media: &'a Path,
    pub files: Option<Vec<(String, Vec<u8>)>>,
}

impl Assets<'_> {
    /// O `src` de uma imagem de `media/`.
    fn image(&mut self, name: &str) -> Option<String> {
        let bytes = std::fs::read(self.media.join(name)).ok()?;
        let Some(files) = &mut self.files else { return Some(format!("data:{};base64,{}", mime(name), crate::ai::agent::base64(&bytes))) };
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let (ext, bytes) = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => (ext, bytes),
            // Fora dos formatos que todo leitor de EPUB abre: vira PNG.
            _ => {
                let mut png = std::io::Cursor::new(vec![]);
                image::load_from_memory(&bytes).ok()?.write_to(&mut png, image::ImageFormat::Png).ok()?;
                ("png".to_owned(), png.into_inner())
            }
        };
        let path = format!("images/{}.{ext}", files.len());
        files.push((path.clone(), bytes));
        Some(path)
    }
}

/// Texto rico (negrito, cores, realce, código, [[links]]…) em HTML; linhas
/// `![](arquivo)` viram imagens.
fn inline(text: &str, assets: &mut Assets) -> String {
    let p = light();
    let mut out = String::new();
    for seg in rich::segments(text) {
        match seg {
            rich::Seg::Image(name) => {
                if let Some(src) = assets.image(&name) {
                    out.push_str(&format!(r#"<img class="inline" src="{src}" alt=""/>"#));
                }
            }
            rich::Seg::Text(t) => {
                for r in rich::parse(&t) {
                    if r.marker {
                        continue;
                    }
                    let st = r.style;
                    let mut css_rules = vec![];
                    if st.bold {
                        css_rules.push("font-weight:600".to_owned());
                    }
                    if st.italic {
                        css_rules.push("font-style:italic".to_owned());
                    }
                    let mut deco = vec![];
                    if st.underline {
                        deco.push("underline");
                    }
                    if st.strike {
                        deco.push("line-through");
                    }
                    if !deco.is_empty() {
                        css_rules.push(format!("text-decoration:{}", deco.join(" ")));
                    }
                    if let Some(i) = st.ink {
                        css_rules.push(format!("color:{}", css(ink_on(i, false))));
                    }
                    if let Some(i) = st.bg {
                        css_rules.push(format!("background:{}", css(ink_bg_on(i, false))));
                    } else if st.highlight {
                        css_rules.push(format!("background:{}", css(p.mark)));
                    }
                    let body = esc(&r.text).replace('\n', "<br/>").replace(['\u{E000}', '\u{E001}'], "");
                    let class = if st.code {
                        " class=\"code\""
                    } else if st.link {
                        " class=\"link\""
                    } else {
                        ""
                    };
                    if css_rules.is_empty() && class.is_empty() {
                        out.push_str(&body);
                    } else {
                        out.push_str(&format!(r#"<span{class} style="{}">{body}</span>"#, css_rules.join(";")));
                    }
                }
                out.push_str("<br/>");
            }
        }
    }
    out.trim_end_matches("<br/>").to_owned()
}

/// O CSS da página: as medidas e cores do editor no tema claro. Os tamanhos
/// de letra são em rem e as entrelinhas sem unidade (a 16 px dá o mesmo que a
/// tela), para o leitor de EPUB poder aumentar a letra. `epub`: fontes como
/// arquivos `fonts/…` e sem a largura fixa da página.
pub(super) fn style(epub: bool) -> String {
    let p = light();
    let mut fonts = String::new();
    for f in FONTS {
        let src = if epub { format!("fonts/{}.ttf", f.name) } else { format!("data:font/ttf;base64,{}", crate::ai::agent::base64(f.data)) };
        fonts.push_str(&format!(
            "@font-face{{font-family:'{}';font-weight:{};font-style:{};src:url({src}) format('truetype')}}\n",
            f.family,
            f.weight,
            if f.italic { "italic" } else { "normal" },
        ));
    }
    let page = if epub {
        "body{padding:0;max-width:none}"
    } else {
        "@page{size:A4;margin:16mm 14mm}\n@media print{html{background:#fff} body{padding:0;max-width:none}}"
    };
    format!(
        r#"{fonts}
:root{{--text:{text};--text2:{text2};--muted:{muted};--dim:{dim};--accent:{accent};--line:{line};--border:{border};--strong:{strong};--chip:{chip};--input:{input};--card:{card};--window:{window};--tint:{tint};--row:{row}}}
*{{box-sizing:border-box;-webkit-print-color-adjust:exact;print-color-adjust:exact}}
{root_bg}
body{{margin:0 auto;padding:48px 24px 64px;max-width:43rem;font-family:'Literata','Noto Sans Math',Georgia,serif;font-size:1rem;line-height:1.656;color:var(--text2)}}
h1.title{{font-size:2.125rem;line-height:1.294;font-weight:600;color:var(--text);margin:0 0 22px}}
p,.blk{{margin:0 0 10px}}
h1,h2,h3{{font-weight:600;color:var(--text);margin:18px 0 8px}}
h1{{font-size:1.6875rem;line-height:1.333}} h2{{font-size:1.3125rem;line-height:1.429}} h3{{font-size:1.094rem;line-height:1.486}}
.li{{display:flex;gap:0}} .li .mk{{flex:0 0 24px;text-align:right;padding-right:7px;color:var(--text2)}} .li .mk.b{{text-align:left;padding-left:6px}}
.li.done{{color:var(--muted)}}
.box{{display:inline-block;width:15px;height:15px;border:1.3px solid var(--strong);border-radius:4px;vertical-align:-2px;position:relative}}
.box.on{{background:var(--accent);border-color:var(--accent)}}
.box.on:after{{content:'';position:absolute;left:4px;top:1px;width:4px;height:8px;border:solid #fff;border-width:0 2px 2px 0;transform:rotate(45deg)}}
blockquote{{margin:0 0 10px;padding:2px 0 2px 18px;border-left:3px solid var(--strong);font-style:italic;line-height:1.625}}
.callout{{display:flex;gap:0;border-radius:10px;padding:12px 14px;margin:0 0 10px;font-size:.9375rem;line-height:1.6;break-inside:avoid}}
.caps{{font-size:.656rem;font-weight:600;letter-spacing:1.05px;text-transform:uppercase}}
.callout .caps{{flex:0 0 52px;line-height:2.476}}
.frame{{background:var(--input);border:1px solid var(--border);border-radius:8px;padding:10px 14px;margin:0 0 10px;break-inside:avoid}}
.frame .caps{{display:block;margin-bottom:6px;color:var(--dim)}}
pre{{margin:0;font-family:'JetBrains Mono',monospace;font-size:.8125rem;line-height:1.538;color:var(--text);white-space:pre-wrap}}
.formula{{text-align:center}} .formula .caps{{color:var(--accent);text-align:left}}
.code{{font-family:'JetBrains Mono',monospace;font-size:.84em;background:var(--chip);border-radius:3px;padding:0 3px}}
.link{{color:var(--accent);text-decoration:underline;text-decoration-color:{link_line}}}
figure{{margin:0 0 10px;text-align:center;break-inside:avoid}} figure img{{max-width:100%;border-radius:6px}}
figcaption{{font-style:italic;font-size:.8125rem;color:var(--muted);margin-top:4px}}
img.inline{{display:block;max-width:100%;margin:6px 0;border-radius:6px}}
table{{width:100%;border-collapse:separate;border-spacing:0;border:1px solid var(--border);border-radius:8px;margin:0 0 10px;font-size:.875rem;line-height:1.5;break-inside:avoid}}
td,th{{padding:7px 10px;text-align:left;vertical-align:top;border-bottom:1px solid var(--row)}}
tr:last-child td{{border-bottom:none}}
th{{background:var(--chip);font-weight:600;color:var(--text)}} th:first-child{{border-top-left-radius:8px}} th:last-child{{border-top-right-radius:8px}}
td{{color:var(--text2)}}
hr{{border:none;border-top:1px solid var(--strong);margin:16px 0}}
.diagram{{border:1px solid var(--border);border-radius:10px;background:var(--input);padding:8px;margin:0 0 10px;text-align:center;break-inside:avoid}}
.diagram svg{{max-width:100%;height:auto}}
{page}
"#,
        text = css(p.text),
        text2 = css(p.text2),
        muted = css(p.muted),
        dim = css(p.dim),
        accent = css(p.accent),
        line = css(p.line),
        border = css(p.border),
        strong = css(p.border_strong),
        chip = css(p.chip),
        input = css(p.input),
        card = css(p.card),
        window = css(p.window),
        tint = css(p.accent_tint),
        row = css(p.line_row),
        link_line = css(soft(p.accent, 0.5)),
        root_bg = if epub { String::new() } else { "html{background:var(--window)}".to_owned() },
    )
}

/// A página inteira como HTML autossuficiente. `ctx` mede o texto das
/// fórmulas e dos fluxogramas com as mesmas fontes da tela.
pub fn page_html(ctx: &egui::Context, title: &str, blocks: &[Block], media: &Path) -> String {
    let css = format!("<style>{}</style>", style(false));
    document(ctx, title, blocks, &mut Assets { media, files: None }, &css)
}

/// O documento (também XHTML válido, para o EPUB); `head` entra no `<head>`.
pub(super) fn document(ctx: &egui::Context, title: &str, blocks: &[Block], assets: &mut Assets, head: &str) -> String {
    let p = light();
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
    let numbers = super::page::numbering(blocks);
    let mut body = format!(r#"<h1 class="title">{}</h1>"#, esc(title.trim()));
    for (b, n) in blocks.iter().zip(numbers) {
        let pad = |indent: u8| format!("margin-left:{}px", indent as u32 * 24);
        let html = match b.kind {
            BlockKind::Paragraph if b.text.trim().is_empty() => "<p>&#160;</p>".to_owned(),
            BlockKind::Paragraph => format!("<p>{}</p>", inline(&b.text, assets)),
            BlockKind::H1 => format!("<h1>{}</h1>", inline(&b.text, assets)),
            BlockKind::H2 => format!("<h2>{}</h2>", inline(&b.text, assets)),
            BlockKind::H3 => format!("<h3>{}</h3>", inline(&b.text, assets)),
            BlockKind::Bullet => {
                let glyph = ["•", "◦", "▪"][b.indent as usize % 3];
                format!(r#"<div class="li blk" style="{}"><span class="mk b">{glyph}</span><span>{}</span></div>"#, pad(b.indent), inline(&b.text, assets))
            }
            BlockKind::Numbered => format!(r#"<div class="li blk" style="{}"><span class="mk">{n}.</span><span>{}</span></div>"#, pad(b.indent), inline(&b.text, assets)),
            BlockKind::Check => format!(
                r#"<div class="li blk{}" style="{}"><span class="mk b"><span class="box{}"></span></span><span>{}</span></div>"#,
                if b.checked { " done" } else { "" },
                pad(b.indent),
                if b.checked { " on" } else { "" },
                inline(&b.text, assets)
            ),
            BlockKind::Quote => format!("<blockquote>{}</blockquote>", inline(&b.text, assets)),
            BlockKind::Callout => {
                let label = Ink::from_name(&b.ink).map_or(p.accent, |k| ink_on(k, false));
                let (fill, edge) = Ink::from_name(&b.fill).map_or((p.accent_tint, p.border), |k| (soft(ink_on(k, false), 0.14), soft(ink_on(k, false), 0.35)));
                format!(
                    r#"<div class="callout" style="background:{};border:1px solid {}"><span class="caps" style="color:{}">{}</span><div>{}</div></div>"#,
                    css(fill),
                    css(edge),
                    css(label),
                    esc(&tr!("page.tip")),
                    inline(&b.text, assets)
                )
            }
            BlockKind::Code => {
                let label = if b.lang.is_empty() { tr!("page.code") } else { b.lang.clone() };
                format!(r#"<div class="frame"><span class="caps">{}</span><pre>{}</pre></div>"#, esc(&label), esc(&b.text))
            }
            BlockKind::Formula => match super::math::svg(&painter, &b.text, 20.0, p.text) {
                Some(svg) => format!(r#"<div class="frame formula"><span class="caps">{}</span>{svg}</div>"#, esc(&tr!("page.formula_label"))),
                None => String::new(),
            },
            BlockKind::Divider => "<hr>".to_owned(),
            BlockKind::Image => match assets.image(&b.media) {
                Some(src) => format!(
                    r#"<figure><img src="{src}" alt="{}" style="width:{:.0}%"/>{}</figure>"#,
                    esc(&b.caption),
                    b.width.clamp(0.25, 1.0) * 100.0,
                    if b.caption.trim().is_empty() { String::new() } else { format!("<figcaption>{}</figcaption>", esc(&b.caption)) }
                ),
                None => String::new(),
            },
            BlockKind::Table => {
                let mut t = String::from("<table>");
                for (i, row) in b.rows.iter().enumerate() {
                    let cell = if i == 0 { "th" } else { "td" };
                    t.push_str("<tr>");
                    for c in row {
                        t.push_str(&format!("<{cell}>{}</{cell}>", inline(c, assets)));
                    }
                    t.push_str("</tr>");
                }
                t + "</table>"
            }
            BlockKind::Diagram => super::diagram::svg(&painter, &b.diagram).map(|svg| format!(r#"<div class="diagram">{svg}</div>"#)).unwrap_or_default(),
        };
        body.push_str(&html);
        body.push('\n');
    }
    format!(
        "<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"{lang}\" xml:lang=\"{lang}\"><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/><title>{}</title>{head}</head><body>\n{body}</body></html>\n",
        esc(title.trim()),
        lang = crate::i18n::current(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_text_keeps_its_formatting_and_escapes_html() {
        let html = inline("**negrito** e {red|vermelho} <tag> [[Página]]", &mut Assets { media: Path::new("."), files: None });
        assert!(html.contains(r#"<span style="font-weight:600">negrito</span>"#), "{html}");
        assert!(html.contains("color:#"), "{html}");
        assert!(html.contains("&lt;tag&gt;"));
        assert!(html.contains(r#"class="link""#) && html.contains("Página") && !html.contains("[["), "{html}");
    }

    #[test]
    fn colors_are_unmultiplied() {
        assert_eq!(css(Color32::from_rgb(52, 69, 166)), "#3445a6");
        // A pré-multiplicação arredonda a cor em ±1; a opacidade fica.
        let c = css(Color32::from_rgba_unmultiplied(52, 69, 166, 128));
        assert!(c.starts_with("rgba(") && c.ends_with(",0.502)"), "{c}");
    }
}
