//! Desenho da marcação inline (`rich.rs`) com as fontes e cores do tema.

use super::theme::*;
use crate::rich::{self, Seg};
use eframe::egui::{self, Color32, Rect, FontId, Stroke, Ui, text::LayoutJob, vec2};
use std::ops::Range;
use std::path::Path;

#[derive(Clone, Copy)]
pub struct Look {
    pub size: f32,
    /// Peso base (títulos usam Semi).
    pub semi: bool,
    pub italic: bool,
    pub color: Color32,
    /// Entrelinha em px (None = natural).
    pub line_height: Option<f32>,
}

fn font_for(style: &rich::Style, look: &Look) -> FontId {
    if style.code {
        return mono(look.size * 0.84);
    }
    let bold = style.bold || style.gap || look.semi;
    let italic = style.italic || look.italic;
    let w = match (bold, italic) {
        (true, true) => W::SemiItalic,
        (true, false) => {
            if look.semi && style.bold {
                W::Bold
            } else {
                W::Semi
            }
        }
        (false, true) => W::Italic,
        _ => W::Regular,
    };
    serif(look.size, w)
}

/// Monta o layout. No editor os marcadores aparecem esmaecidos; fora dele somem.
/// Devolve também as faixas (em caracteres do texto exibido) dos `[[links]]`.
pub fn job(text: &str, look: &Look, editor: bool, wrap: f32) -> (LayoutJob, Vec<(Range<usize>, String)>) {
    job_ex(text, look, editor, editor, wrap)
}

/// `editor`: o texto é o do `TextEdit` (marcadores mantidos); `show_markers`:
/// marcadores visíveis (bloco em edição) ou reduzidos a quase nada (fora dela).
pub fn job_ex(text: &str, look: &Look, editor: bool, show_markers: bool, wrap: f32) -> (LayoutJob, Vec<(Range<usize>, String)>) {
    let p = pal();
    let mut job = LayoutJob::default();
    let mut links = Vec::new();
    let mut chars = 0usize;
    for run in rich::parse(text) {
        if run.marker && !editor {
            continue;
        }
        let st = run.style;
        let mut fmt = egui::TextFormat {
            font_id: font_for(&st, look),
            color: if run.marker { p.dim } else if st.link || st.gap { p.accent } else if st.bold && !look.semi { p.text } else { look.color },
            line_height: look.line_height,
            valign: egui::Align::Center,
            ..Default::default()
        };
        if run.marker && !show_markers {
            fmt.font_id = FontId::new(0.5, fmt.font_id.family.clone());
            fmt.color = egui::Color32::TRANSPARENT;
        }
        if st.code && !run.marker {
            fmt.background = p.chip;
        }
        if st.highlight && !run.marker {
            fmt.background = p.mark;
        }
        if st.gap {
            fmt.background = p.accent_soft;
        }
        if st.underline && !run.marker {
            fmt.underline = Stroke::new(1.0, fmt.color);
        }
        if st.link && !run.marker && !show_markers {
            fmt.underline = Stroke::new(1.0, soft(p.accent, 0.5));
        }
        if st.strike && !run.marker {
            fmt.strikethrough = Stroke::new(1.0, fmt.color);
        }
        let n = run.text.chars().count();
        if st.link && !run.marker {
            links.push((chars..chars + n, run.text.clone()));
        }
        let txt = if st.gap && !editor { format!("\u{2009}{}\u{2009}", run.text) } else { run.text.clone() };
        chars += txt.chars().count();
        job.append(&txt, 0.0, fmt);
    }
    if job.sections.is_empty() {
        job.append("", 0.0, egui::TextFormat { font_id: serif(look.size, W::Regular), color: look.color, line_height: look.line_height, ..Default::default() });
    }
    job.wrap.max_width = wrap;
    (job, links)
}

/// Texto rico (sem marcadores) que quebra linha. Devolve o link clicado, se houver.
pub fn label(ui: &mut Ui, text: &str, look: Look) -> (egui::Response, Option<String>) {
    let w = ui.available_width();
    let (job, links) = job(text, &look, false, w);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let sense = if links.is_empty() { egui::Sense::hover() } else { egui::Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w.min(galley.size().x.max(1.0)).max(galley.size().x), galley.size().y), sense);
    ui.painter().galley(rect.min, galley.clone(), look.color);
    let mut clicked = None;
    if !links.is_empty() {
        if let Some(pos) = resp.hover_pos() {
            let c = galley.cursor_from_pos(pos - rect.min).index.0;
            if let Some((_, target)) = links.iter().find(|(r, _)| r.contains(&c)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    clicked = Some(target.clone());
                }
            }
        }
    }
    (resp, clicked)
}

/// Campo de cartão: texto rico + imagens (`![](arquivo)`).
pub fn field(ui: &mut Ui, text: &str, look: Look, media: &Path, max_img_h: f32) {
    for seg in rich::segments(text) {
        match seg {
            Seg::Text(t) => {
                label(ui, &t, look);
            }
            Seg::Image(name) => {
                let w = ui.available_width();
                image(ui, &crate::media::uri(media, &name), w, max_img_h, 8, egui::Sense::hover());
            }
        }
    }
}

/// Imagem do tamanho natural, reduzida para caber em `max_w` × `max_h`.
///
/// O tamanho sai das dimensões da imagem, não do espaço "disponível" do
/// layout: dentro de linhas horizontais esse espaço tem altura quase zero e o
/// `egui::Image` encolheria a imagem até sumir. Enquanto carrega, reserva um
/// espaço com um indicador de progresso.
pub fn image(ui: &mut Ui, uri: &str, max_w: f32, max_h: f32, radius: u8, sense: egui::Sense) -> egui::Response {
    let img = egui::Image::new(uri.to_owned()).corner_radius(radius).sense(sense);
    match img.load_for_size(ui.ctx(), vec2(max_w, max_h)) {
        Ok(poll) => match poll.size() {
            Some(size) if size.x > 0.0 && size.y > 0.0 => {
                let scale = (max_w / size.x).min(max_h / size.y).min(1.0);
                ui.add(img.fit_to_exact_size(size * scale))
            }
            _ => {
                let (r, resp) = ui.allocate_exact_size(vec2(max_w.min(320.0), 120.0), egui::Sense::hover());
                egui::Spinner::new().paint_at(ui, Rect::from_center_size(r.center(), vec2(20.0, 20.0)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                resp
            }
        },
        Err(_) => {
            let p = pal();
            let (r, resp) = ui.allocate_exact_size(vec2(max_w.min(320.0), 60.0), egui::Sense::hover());
            ui.painter().rect_stroke(r, egui::CornerRadius::same(8), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "imagem indisponível", serif(12.0, W::Italic), p.muted);
            resp
        }
    }
}

/// Layouter para `TextEdit` com marcação viva.
pub fn layouter(look: Look) -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let (job, _) = job(buf.as_str(), &look, true, wrap);
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// Layouter de bloco fora de edição: marcação aplicada, marcadores escondidos.
pub fn layouter_quiet(look: Look) -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let (job, _) = job_ex(buf.as_str(), &look, true, false, wrap);
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// Layouter mono simples (código, fórmulas).
pub fn mono_layouter(size: f32, color: Color32) -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = LayoutJob::single_section(buf.as_str().to_owned(), egui::TextFormat { font_id: mono(size), color, line_height: Some(size * 1.55), ..Default::default() });
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// Layouter dos campos de cloze: as lacunas `{{c1::…}}` aparecem como
/// "fichas" mono em destaque (como no design); o resto segue a marcação inline.
pub fn cloze_layouter(look: Look) -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let p = pal();
        let text = buf.as_str();
        let mut out = LayoutJob::default();
        let mut last = 0;
        let push_rich = |out: &mut LayoutJob, s: &str| {
            if s.is_empty() {
                return;
            }
            let (j, _) = job(s, &look, true, wrap);
            for sec in j.sections {
                out.append(&j.text[sec.byte_range.start.0..sec.byte_range.end.0], 0.0, sec.format);
            }
        };
        for c in crate::cards::parse_clozes(text) {
            push_rich(&mut out, &text[last..c.start]);
            out.append(
                &text[c.start..c.end],
                0.0,
                egui::TextFormat { font_id: mono(look.size * 0.8), color: p.accent, background: p.accent_soft, line_height: look.line_height, valign: egui::Align::Center, ..Default::default() },
            );
            last = c.end;
        }
        push_rich(&mut out, &text[last..]);
        if out.sections.is_empty() {
            out.append("", 0.0, egui::TextFormat { font_id: serif(look.size, W::Regular), color: look.color, line_height: look.line_height, ..Default::default() });
        }
        out.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(out))
    }
}
