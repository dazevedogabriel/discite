//! Adicionar / editar nota (EditorNota.dc.html): tipo e baralho, campos com
//! marcação inline e imagens, lacunas de cloze, máscaras de oclusão, tags,
//! aviso de duplicata e pré-visualização de cada cartão.

use super::richtext::{self, Look};
use super::theme::*;
use super::widgets::*;
use super::{App, Screen};
use crate::{tr, tr_n};
use crate::cards::{self, Mask, NoteKind};
use crate::db::Note;
use crate::rich;
use eframe::egui::{self, Align2, CornerRadius, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, text::CCursor, text::CCursorRange, vec2};

pub struct NoteEditor {
    pub note: Note,
    pub deck: i64,
    existing: bool,
    keep: bool,
    preview_ord: usize,
    tag_input: String,
    focused: Option<usize>,
    /// Última seleção (campo, faixa em caracteres) para os botões da barra.
    sel: Option<(usize, CCursorRange)>,
    dup: Option<(i64, String, String)>,
    dup_key: String,
    drag: Option<Pos2>,
    mask_sel: Option<usize>,
    origin_title: Option<String>,
    first: bool,
    pending_cursor: Option<(usize, usize, usize)>,
}

impl NoteEditor {
    pub fn new_note(deck: i64, origin: Option<(i64, Option<i64>, String, String)>) -> NoteEditor {
        let mut note = Note::new(NoteKind::Basic);
        let mut origin_title = None;
        if let Some((page, block, text, title)) = origin {
            note.source_page = Some(page);
            note.source_block = block;
            note.fields[0] = text;
            origin_title = Some(title);
        }
        NoteEditor {
            note,
            deck,
            existing: false,
            keep: true,
            preview_ord: 0,
            tag_input: String::new(),
            focused: None,
            sel: None,
            dup: None,
            dup_key: String::new(),
            drag: None,
            mask_sel: None,
            origin_title,
            first: true,
            pending_cursor: None,
        }
    }

    pub fn edit(note: Note, deck: i64) -> NoteEditor {
        let mut e = NoteEditor::new_note(deck, None);
        e.note = note;
        e.existing = true;
        e
    }

    fn field_id(&self, i: usize) -> egui::Id {
        egui::Id::unique(("note_field", self.note.id, i))
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.note_editor.take() else { return };
    if ed.origin_title.is_none() {
        if let Some(pid) = ed.note.source_page {
            ed.origin_title = app.db.page(pid).ok().flatten().map(|p| p.title);
        }
    }
    let p = pal();
    let screen = ctx.content_rect();
    let size = vec2(1000.0f32.min(screen.width() - 40.0), 716.0f32.min(screen.height() - 70.0));
    let mut close = false;
    let mut submit = false;
    egui::Modal::new(egui::Id::unique("note_editor"))
        .backdrop_color(p.scrim)
        .frame(egui::Frame::NONE.fill(p.window).corner_radius(16).shadow(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: egui::Color32::from_black_alpha(if p.dark { 140 } else { 46 }) }).stroke(Stroke::new(1.0, p.border_control)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            let rect = ui.max_rect();
            let rect = Rect::from_min_size(rect.min, size);
            body(app, &mut ed, ui, rect, &mut close, &mut submit);
        });
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && ed.drag.is_none() {
        close = true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        submit = true;
    }
    if submit {
        match save(app, &mut ed) {
            Ok(done) => {
                if done {
                    close = true;
                }
            }
            Err(e) => app.error(e),
        }
    }
    if !close {
        app.note_editor = Some(ed);
    }
}

/// Grava. Devolve `true` se o editor deve fechar.
fn save(app: &mut App, ed: &mut NoteEditor) -> anyhow::Result<bool> {
    let n = &mut ed.note;
    let ords = cards::card_ords(n.kind, &n.fields);
    match n.kind {
        NoteKind::Cloze if ords.is_empty() => anyhow::bail!(tr!("note.need_cloze")),
        NoteKind::Occlusion if n.fields[0].is_empty() => anyhow::bail!(tr!("note.need_image")),
        NoteKind::Occlusion if ords.is_empty() => anyhow::bail!(tr!("note.need_mask")),
        NoteKind::Basic | NoteKind::BasicReversed | NoteKind::TypeIn if rich::plain(&n.fields[0]).trim().is_empty() && !n.fields[0].contains("![](") => {
            anyhow::bail!(tr!("note.front_empty"))
        }
        _ => {}
    }
    n.tags = crate::db::normalize_tags(&format!("{} {}", n.tags, ed.tag_input));
    ed.tag_input.clear();
    if ed.existing {
        app.db.update_note(n, ed.deck)?;
        let cards = app.db.note_cards(n.id)?;
        if cards.iter().any(|c| c.deck_id != ed.deck) {
            app.db.move_cards(&cards.iter().map(|c| c.id).collect::<Vec<_>>(), ed.deck)?;
        }
        app.toast(tr!("toast.note_saved"));
        app.changed();
        return Ok(true);
    }
    let made = app.db.add_note(n, ed.deck)?;
    app.cfg.last_deck = Some(ed.deck);
    app.save_settings();
    app.toast(tr_n!("toast.note_added", made));
    app.changed();
    if ed.keep {
        let kind = n.kind;
        let tags = n.tags.clone();
        let (page, block) = (n.source_page, n.source_block);
        ed.note = Note::new(kind);
        ed.note.tags = tags;
        ed.note.source_page = page;
        ed.note.source_block = block;
        ed.preview_ord = 0;
        ed.mask_sel = None;
        ed.first = true;
        Ok(false)
    } else {
        Ok(true)
    }
}

fn body(app: &mut App, ed: &mut NoteEditor, ui: &mut Ui, rect: Rect, close: &mut bool, submit: &mut bool) {
    let p = pal();
    // ── Cabeçalho ──
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let t = text(ui.painter(), pos2(head.left() + 26.0, head.center().y), Align2::LEFT_CENTER, &tr!(if ed.existing { "note.edit_title" } else { "note.add_title" }), serif(19.0, W::Semi), p.text, 300.0);
    if let Some(title) = &ed.origin_title {
        let label = format!("{}  {title}", tr!("note.source"));
        let w = text_width(ui.painter(), &label, serif(12.0, W::Regular)) + 20.0;
        let cr = Rect::from_min_size(pos2(t.right() + 14.0, head.center().y - 12.0), vec2(w.min(380.0), 24.0));
        ui.painter().rect(cr, CornerRadius::same(255), p.chip, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let g = text(ui.painter(), pos2(cr.left() + 10.0, cr.center().y), Align2::LEFT_CENTER, &tr!("note.source"), serif(12.0, W::Regular), p.muted, 60.0);
        text(ui.painter(), pos2(g.right() + 6.0, cr.center().y), Align2::LEFT_CENTER, title, serif(12.0, W::Regular), p.text2, cr.right() - g.right() - 14.0);
    }
    let close_r = Rect::from_center_size(pos2(head.right() - 36.0, head.center().y), vec2(40.0, 28.0));
    let cresp = ui.interact(close_r, ui.scope_id().with("close"), Sense::click());
    if cresp.hovered() {
        ui.painter().rect_filled(close_r, CornerRadius::same(6), p.hover);
    }
    close_icon(ui.painter(), close_r.center(), 13.0, if cresp.hovered() { p.text } else { p.muted });
    a11y(&cresp, egui::accesskit::Role::Button, &tr!("app.close_dialog"), None);
    if cresp.clicked() {
        *close = true;
    }

    // ── Rodapé ──
    let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 64.0), rect.max);
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, p.line));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(24.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if !ed.existing {
        checkbox(&mut fu, &mut ed.keep, &tr!("note.keep"));
    }
    fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button_kbd(ui, &tr!(if ed.existing { "common.save" } else { "common.add" }), Some("Ctrl ↵"), Btn::Primary, 36.0).clicked() {
            *submit = true;
        }
        if button(ui, &tr!("common.cancel"), Btn::Secondary, 36.0).clicked() {
            *close = true;
        }
        if ed.existing && button(ui, &tr!("note.delete"), Btn::Danger, 36.0).clicked() {
            app.modal = Some(super::modals::Modal::DeleteNotes(vec![ed.note.id]));
            *close = true;
        }
    });

    // ── Corpo ──
    let mid = Rect::from_min_max(pos2(rect.left(), head.bottom()), pos2(rect.right(), foot.top()));
    let right_w = 380.0;
    let left = Rect::from_min_max(mid.min, pos2(mid.right() - right_w, mid.bottom()));
    let right = Rect::from_min_max(pos2(mid.right() - right_w, mid.top()), mid.max);
    ui.painter().rect_filled(right, CornerRadius { nw: 0, ne: 0, sw: 0, se: 0 }, p.card);
    ui.painter().vline(left.right(), left.y_range(), Stroke::new(1.0, p.line));

    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(26.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt("ne_left").auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 16.0;
        left_column(app, ed, ui);
    });
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(22.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    preview(app, ed, &mut ru);
}

fn left_column(app: &mut App, ed: &mut NoteEditor, ui: &mut Ui) {
    let p = pal();
    let w = ui.available_width();
    // Tipo e baralho.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            caps_label_w(ui, &tr!("note.note_type"), 180.0);
            let cur = ed.note.kind;
            select_field(ui, "kind", &cur.label(), None, 180.0, 36.0, |ui| {
                for k in NoteKind::ALL {
                    if menu_item(ui, &k.label(), k == cur).clicked() && k != cur {
                        if ed.existing && cards::card_ords(k, &NoteKind::convert_fields(cur, k, &ed.note.fields)).len() < cards::card_ords(cur, &ed.note.fields).len() {
                            // Trocar de tipo pode apagar cartões (e o histórico deles).
                        }
                        ed.note.fields = NoteKind::convert_fields(cur, k, &ed.note.fields);
                        ed.note.kind = k;
                        ed.preview_ord = 0;
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let dw = ui.available_width();
            caps_label_w(ui, &tr!("deck.deck"), dw);
            let path = app.deck_path(ed.deck);
            let color = app.subject_color_of_deck(ed.deck);
            let decks = app.decks.clone();
            select_field(ui, "deck", &path, Some(color), dw, 36.0, |ui| {
                for s in app.subjects.clone() {
                    for d in decks.iter().filter(|d| d.subject_id == s.id) {
                        let label = app.deck_path(d.id);
                        if menu_item_ex(ui, &label, Some(subject_color(s.color)), d.id == ed.deck, None).clicked() {
                            ed.deck = d.id;
                        }
                    }
                }
            });
        });
    });

    let names = ed.note.kind.field_names();
    let kind = ed.note.kind;
    for i in 0..names.len() {
        if kind == NoteKind::Occlusion && i == 1 {
            continue;
        }
        if kind == NoteKind::Occlusion && i == 0 {
            occlusion_editor(app, ed, ui);
            continue;
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let hint = if kind == NoteKind::Cloze && i == 0 {
                let n = cards::cloze_numbers(&ed.note.fields[0]).len();
                Some(tr_n!("note.gaps", n))
            } else {
                None
            };
            let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
            caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!(names[i]), p.dim);
            if let Some(h) = hint {
                text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &h, serif(11.5, W::Regular), p.muted, 240.0);
            }
            field_box(app, ed, ui, i, w);
        });
    }

    // Tags.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        caps_label_w(ui, &tr!("note.tags"), w);
        let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
        ui.painter().rect(r, CornerRadius::same(8), p.input, Stroke::new(1.0, p.border_control), StrokeKind::Inside);
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        cu.spacing_mut().item_spacing.x = 6.0;
        let tags = ed.note.tag_list();
        for t in &tags {
            if chip(&mut cu, &format!("{t} ×"), false, 22.0).on_hover_text(tr!("note.remove_tag")).clicked() {
                ed.note.tags = tags.iter().filter(|x| *x != t).cloned().collect::<Vec<_>>().join(" ");
            }
        }
        let resp = cu.add(
            egui::TextEdit::singleline(&mut ed.tag_input)
                .frame(egui::Frame::NONE)
                .font(serif(13.0, W::Regular))
                .hint_text(egui::RichText::new(tr!("note.add_tag")).italics().color(p.dim))
                .desired_width(cu.available_width()),
        );
        if ed.tag_input.ends_with(' ') || ed.tag_input.ends_with(',') || (resp.lost_focus() && !ed.tag_input.trim().is_empty()) {
            ed.note.tags = crate::db::normalize_tags(&format!("{} {}", ed.note.tags, ed.tag_input));
            ed.tag_input.clear();
            if !resp.lost_focus() {
                resp.request_focus();
            }
        }
    });

    // Duplicata.
    let key = cards::sort_field(ed.note.kind, &ed.note.fields);
    if key != ed.dup_key {
        ed.dup_key = key;
        ed.dup = app.db.find_duplicate(ed.note.kind, &ed.note.fields, ed.note.id).ok().flatten();
    }
    if let Some((id, front, kind_label)) = ed.dup.clone() {
        egui::Frame::NONE.fill(soft(p.hard, 0.12)).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(84.0, 18.0), Sense::hover());
                caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("note.duplicate"), p.hard);
                ui.vertical(|ui| {
                    para(ui, &tr!("note.similar", front = front, kind = kind_label), serif(13.0, W::Regular), p.text2);
                    if ui.link(egui::RichText::new(tr!("note.view_note")).color(p.accent).size(13.0)).clicked() {
                        app.browser.query = format!("nid:{id}");
                        app.browser.select_note = Some(id);
                        app.browser.dirty = true;
                        app.go(Screen::Browser);
                    }
                });
            });
        });
    }
}

fn caps_label_w(ui: &mut Ui, s: &str, w: f32) {
    let p = pal();
    let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
    caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, s, p.dim);
}

/// Caixa de campo: barra (B I U, Imagem, Nova lacuna) + texto.
fn field_box(app: &mut App, ed: &mut NoteEditor, ui: &mut Ui, i: usize, w: f32) {
    let p = pal();
    let main = i == 0;
    let cloze = ed.note.kind == NoteKind::Cloze && i == 0;
    let id = ed.field_id(i);
    let focused = ed.focused == Some(i);
    let look = if main { Look { size: 16.0, semi: false, italic: false, color: p.text, line_height: Some(28.0) } } else { Look { size: 14.0, semi: false, italic: false, color: p.text2, line_height: Some(22.0) } };

    // Atalhos de formatação no campo focado.
    if focused {
        let ctx = ui.ctx().clone();
        for (k, m) in [(Key::B, "**"), (Key::I, "*"), (Key::U, "__")] {
            if ctx.input_mut(|inp| inp.consume_key(Modifiers::COMMAND, k)) {
                wrap_sel(ed, i, m);
            }
        }
        if cloze && ctx.input_mut(|inp| inp.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::C)) {
            new_cloze(ed);
        }
    }

    let frame_stroke = if focused { Stroke::new(1.0, p.accent) } else { Stroke::new(1.0, p.border_control) };
    let resp = egui::Frame::NONE.fill(p.input).stroke(frame_stroke).corner_radius(10).show(ui, |ui| {
        ui.set_width(w);
        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
        // Barra.
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 5)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_width(w - 12.0);
                for (g, m, key) in [("B", "**", "note.bold"), ("I", "*", "note.italic"), ("U", "__", "note.underline")] {
                    let label = &tr!(key);
                    let r = fmt_button(ui, g, label);
                    if r.clicked() {
                        wrap_sel(ed, i, m);
                    }
                }
                if button(ui, &tr!("note.image"), Btn::Ghost, 26.0).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter(tr!("note.images_filter"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() {
                        match crate::media::store_file(&app.db.media_dir(), &path) {
                            Ok(s) => {
                                let _ = app.db.add_media(&s);
                                insert_image(ed, i, &s.file_name);
                            }
                            Err(e) => app.error(e),
                        }
                    }
                }
                if cloze {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button_kbd(ui, &tr!("note.new_gap"), Some("Ctrl ⇧ C"), Btn::Secondary, 26.0).clicked() {
                            new_cloze(ed);
                        }
                    });
                }
            });
        });
        let (sep, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, p.line));
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 14 }).show(ui, |ui| {
            let mut field = std::mem::take(&mut ed.note.fields[i]);
            let te = egui::TextEdit::multiline(&mut field).id(id).frame(egui::Frame::NONE).desired_width(w - 32.0).desired_rows(if main { 3 } else { 2 });
            let out = if cloze {
                let mut lay = richtext::cloze_layouter(look);
                te.layouter(&mut lay).show(ui)
            } else {
                let mut lay = richtext::layouter(look);
                te.layouter(&mut lay).show(ui)
            };
            ed.note.fields[i] = field;
            if out.response.response.has_focus() {
                ed.focused = Some(i);
                if let Some(r) = out.cursor_range {
                    ed.sel = Some((i, r));
                }
            }
            if ed.first && i == 0 && !ed.existing && ed.note.kind != NoteKind::Occlusion {
                out.response.response.request_focus();
                ed.first = false;
            }
            if let Some((fi, a, b)) = ed.pending_cursor.take() {
                if fi == i {
                    let mut st = out.state.clone();
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                    st.store(ui.ctx(), id);
                    out.response.response.request_focus();
                } else {
                    ed.pending_cursor = Some((fi, a, b));
                }
            }
        });
    });
    if focused {
        ui.painter().rect_stroke(resp.response.rect.expand(1.0), CornerRadius::same(12), Stroke::new(3.0, p.accent_soft), StrokeKind::Outside);
    }
    // Imagem colada vai para o campo focado.
    if focused {
        if let Some(name) = app.pasted_image.take() {
            insert_image(ed, i, &name);
        }
        for path in std::mem::take(&mut app.dropped) {
            if crate::media::is_image_path(&path) {
                if let Ok(s) = crate::media::store_file(&app.db.media_dir(), &path) {
                    let _ = app.db.add_media(&s);
                    insert_image(ed, i, &s.file_name);
                }
            }
        }
    }
}

fn fmt_button(ui: &mut Ui, glyph: &str, label: &str) -> egui::Response {
    let p = pal();
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
    }
    let font = match glyph {
        "B" => serif(14.0, W::Bold),
        "I" => serif(14.0, W::Italic),
        _ => serif(14.0, W::Regular),
    };
    let tr = text(ui.painter(), r.center(), Align2::CENTER_CENTER, glyph, font, p.text2, 26.0);
    if glyph == "U" {
        ui.painter().hline(tr.x_range(), tr.bottom() - 1.0, Stroke::new(1.0, p.text2));
    }
    a11y(&resp, egui::accesskit::Role::Button, label, None);
    resp.on_hover_text(label)
}

fn selection_bytes(ed: &NoteEditor, i: usize) -> (usize, usize) {
    let t = &ed.note.fields[i];
    match ed.sel {
        Some((fi, r)) if fi == i => {
            let (a, b) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
            (rich::byte_of_char(t, a), rich::byte_of_char(t, b))
        }
        _ => (t.len(), t.len()),
    }
}

fn wrap_sel(ed: &mut NoteEditor, i: usize, marker: &str) {
    let (a, b) = selection_bytes(ed, i);
    let t = ed.note.fields[i].clone();
    let (s, na, nb) = rich::toggle_wrap(&t, a, b, marker);
    ed.pending_cursor = Some((i, rich::char_of_byte(&s, na), rich::char_of_byte(&s, nb)));
    ed.note.fields[i] = s;
}

fn new_cloze(ed: &mut NoteEditor) {
    let t = ed.note.fields[0].clone();
    let n = cards::next_cloze_number(&t);
    let (a, b) = selection_bytes(ed, 0);
    let sel = &t[a..b];
    let open = format!("{{{{c{n}::");
    let s = format!("{}{open}{sel}}}}}{}", &t[..a], &t[b..]);
    let start = rich::char_of_byte(&s, a + open.len());
    ed.pending_cursor = Some((0, start, start + sel.chars().count()));
    ed.note.fields[0] = s;
    ed.preview_ord = (n - 1) as usize;
}

fn insert_image(ed: &mut NoteEditor, i: usize, name: &str) {
    let (a, _) = selection_bytes(ed, i);
    let t = ed.note.fields[i].clone();
    let before = &t[..a];
    let after = &t[a..];
    let pre = if before.is_empty() || before.ends_with('\n') { "" } else { "\n" };
    let post = if after.starts_with('\n') { "" } else { "\n" };
    let ins = format!("{pre}{}{post}", rich::image_markup(name));
    let s = format!("{before}{ins}{after}");
    let c = rich::char_of_byte(&s, a + ins.len());
    ed.pending_cursor = Some((i, c, c));
    ed.note.fields[i] = s;
}

/// Imagem + máscaras: arraste para desenhar, clique para selecionar, Delete apaga.
fn occlusion_editor(app: &mut App, ed: &mut NoteEditor, ui: &mut Ui) {
    let p = pal();
    let w = ui.available_width();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let masks = cards::parse_masks(&ed.note.fields[1]);
        let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
        caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("note.image"), p.dim);
        text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &tr_n!("note.masks", masks.len()), serif(11.5, W::Regular), p.muted, 260.0);
        ui.horizontal(|ui| {
            if button(ui, &tr!(if ed.note.fields[0].is_empty() { "note.choose_image" } else { "note.change_image" }), Btn::Secondary, 30.0).clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter(tr!("note.images_filter"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() {
                    match crate::media::store_file(&app.db.media_dir(), &path) {
                        Ok(s) => {
                            let _ = app.db.add_media(&s);
                            ed.note.fields[0] = s.file_name;
                        }
                        Err(e) => app.error(e),
                    }
                }
            }
            label(ui, &tr!("note.paste_or_drag"), serif(12.5, W::Italic), p.muted);
            if ed.mask_sel.is_some() && button(ui, &tr!("note.delete_mask"), Btn::Danger, 30.0).clicked() {
                remove_mask(ed);
            }
        });
        if let Some(name) = app.pasted_image.take() {
            ed.note.fields[0] = name;
        }
        for path in std::mem::take(&mut app.dropped) {
            if crate::media::is_image_path(&path) {
                if let Ok(s) = crate::media::store_file(&app.db.media_dir(), &path) {
                    let _ = app.db.add_media(&s);
                    ed.note.fields[0] = s.file_name;
                }
            }
        }
        if ed.note.fields[0].is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(w, 160.0), Sense::hover());
            ui.painter().rect(r, CornerRadius::same(10), p.card, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
            text(ui.painter(), r.center(), Align2::CENTER_CENTER, &tr!("note.no_image"), serif(13.0, W::Italic), p.muted, w);
            return;
        }
        let resp = richtext::image(ui, &crate::media::uri(&app.db.media_dir(), &ed.note.fields[0]), w, 340.0, 0, Sense::click_and_drag());
        let r = resp.rect;
        let to_norm = |pt: Pos2| pos2(((pt.x - r.left()) / r.width()).clamp(0.0, 1.0), ((pt.y - r.top()) / r.height()).clamp(0.0, 1.0));
        let mut masks = masks;
        if resp.drag_started() {
            ed.drag = resp.interact_pointer_pos().map(to_norm);
        }
        let mut preview_rect = None;
        if let (Some(start), Some(cur)) = (ed.drag, resp.interact_pointer_pos()) {
            let c = to_norm(cur);
            preview_rect = Some(Rect::from_two_pos(start, c));
            if resp.drag_stopped() {
                let nr = Rect::from_two_pos(start, c);
                if nr.width() > 0.01 && nr.height() > 0.01 {
                    masks.push(Mask { x: nr.left(), y: nr.top(), w: nr.width(), h: nr.height() });
                    ed.mask_sel = Some(masks.len() - 1);
                    ed.preview_ord = masks.len() - 1;
                    ed.note.fields[1] = cards::masks_json(&masks);
                }
                ed.drag = None;
            }
        } else if resp.drag_stopped() {
            ed.drag = None;
        }
        if resp.clicked() {
            if let Some(pt) = resp.interact_pointer_pos().map(to_norm) {
                ed.mask_sel = masks.iter().rposition(|m| Rect::from_min_size(pos2(m.x, m.y), vec2(m.w, m.h)).contains(pt));
                if let Some(s) = ed.mask_sel {
                    ed.preview_ord = s;
                }
            }
        }
        for (i, m) in masks.iter().enumerate() {
            let mr = Rect::from_min_size(pos2(r.left() + m.x * r.width(), r.top() + m.y * r.height()), vec2(m.w * r.width(), m.h * r.height()));
            let sel = ed.mask_sel == Some(i);
            ui.painter().rect(mr, CornerRadius::same(4), soft(p.accent, if sel { 0.75 } else { 0.5 }), Stroke::new(if sel { 2.0 } else { 1.0 }, p.accent), StrokeKind::Inside);
            text(ui.painter(), mr.center(), Align2::CENTER_CENTER, &(i + 1).to_string(), serif(13.0, W::Semi), p.on_accent, mr.width());
        }
        if let Some(pr) = preview_rect {
            let mr = Rect::from_min_max(pos2(r.left() + pr.left() * r.width(), r.top() + pr.top() * r.height()), pos2(r.left() + pr.right() * r.width(), r.top() + pr.bottom() * r.height()));
            ui.painter().rect_stroke(mr, CornerRadius::same(4), Stroke::new(1.5, p.accent), StrokeKind::Inside);
        }
        if ed.mask_sel.is_some() && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            remove_mask(ed);
        }
    });
}

fn remove_mask(ed: &mut NoteEditor) {
    let mut masks = cards::parse_masks(&ed.note.fields[1]);
    if let Some(s) = ed.mask_sel.take() {
        if s < masks.len() {
            masks.remove(s);
        }
    }
    ed.note.fields[1] = cards::masks_json(&masks);
    ed.preview_ord = 0;
}

fn preview(app: &App, ed: &mut NoteEditor, ui: &mut Ui) {
    let p = pal();
    let w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 12.0;
    let ords = cards::card_ords(ed.note.kind, &ed.note.fields);
    let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
    caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("note.preview"), p.dim);
    if ords.len() > 1 {
        let labels: Vec<String> = ords
            .iter()
            .map(|o| match ed.note.kind {
                NoteKind::Cloze => format!("c{}", o + 1),
                NoteKind::Occlusion => format!("{}", o + 1),
                _ => tr!("note.card_n", n = o + 1),
            })
            .take(8)
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut sel = ords.iter().position(|o| *o as usize == ed.preview_ord).unwrap_or(0).min(refs.len() - 1);
        let total: f32 = refs.iter().map(|l| text_width(ui.painter(), l, serif(12.0, W::Regular)) + 24.0).sum::<f32>() + 4.0;
        let mut su = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.right() - total, r.top()), vec2(total, 30.0))));
        if segmented(&mut su, &refs, &mut sel, 24.0, false) {
            ed.preview_ord = ords[sel] as usize;
        }
    }
    let ord = if ords.contains(&(ed.preview_ord as u32)) { ed.preview_ord as u32 } else { ords.first().copied().unwrap_or(0) };
    let face = cards::render(ed.note.kind, &ed.note.fields, ord);
    let media = app.db.media_dir();
    egui::ScrollArea::vertical().id_salt("ne_prev").auto_shrink([false, false]).max_height(ui.available_height() - 50.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        label(ui, &tr!("note.front"), serif(12.0, W::Regular), p.muted);
        let box_frame = egui::Frame::NONE.fill(p.window).stroke(Stroke::new(1.0, p.border)).corner_radius(12).inner_margin(20);
        let look = Look { size: 17.0, semi: false, italic: false, color: p.text, line_height: Some(25.0) };
        box_frame.show(ui, |ui| {
            ui.set_width(w - 40.0);
            if let Some((img, masks, t)) = &face.occlusion {
                if !img.is_empty() {
                    super::review::occlusion_view(ui, &media, img, masks, *t, false);
                }
            } else if face.question.trim().is_empty() {
                label(ui, "…", serif(17.0, W::Italic), p.dim);
            } else {
                richtext::field(ui, &face.question, look, &media, 160.0);
            }
            if face.type_answer.is_some() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                ui.painter().rect(r, CornerRadius::same(8), p.input, Stroke::new(1.0, p.border_control), StrokeKind::Inside);
                text(ui.painter(), r.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, &tr!("note.type_placeholder"), serif(13.0, W::Italic), p.dim, r.width());
            }
        });
        label(ui, &tr!("note.back"), serif(12.0, W::Regular), p.muted);
        box_frame.show(ui, |ui| {
            ui.set_width(w - 40.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            if let Some((img, masks, t)) = &face.occlusion {
                if !img.is_empty() {
                    super::review::occlusion_view(ui, &media, img, masks, *t, true);
                }
            } else if let Some(back) = &face.cloze_back {
                richtext::field(ui, back, look, &media, 160.0);
            } else {
                richtext::field(ui, &face.answer, look, &media, 160.0);
            }
            if !face.extra.trim().is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
                richtext::field(ui, &face.extra, Look { size: 13.5, semi: false, italic: true, color: p.muted, line_height: Some(20.0) }, &media, 120.0);
            }
        });
    });
    let tip = &tr!(match ed.note.kind {
        NoteKind::Cloze => "note.tip_cloze",
        NoteKind::BasicReversed => "note.tip_reversed",
        NoteKind::TypeIn => "note.tip_type_in",
        NoteKind::Occlusion => "note.tip_occlusion",
        NoteKind::Basic => "note.tip_basic",
    });
    let rest = ui.available_height() - 40.0;
    if rest > 0.0 {
        ui.add_space(rest);
    }
    para(ui, tip, serif(12.0, W::Italic), p.muted);
}
