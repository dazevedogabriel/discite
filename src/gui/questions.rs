//! Questões objetivas na interface: a aba Questões da disciplina (bancos e a
//! lista de questões), o editor de questão, a escolha do simulado e a tela de
//! resolução (com correção a cada questão ou só no final).

use super::modals::Modal;
use super::richtext::{self, Look};
use super::theme::*;
use super::widgets::*;
use super::{App, Screen};
use crate::quiz::{Bank, Counts, OPTIONS, Pick, Question};
use crate::rich;
use crate::{tr, tr_n};
use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, text::CCursor, text::CCursorRange, vec2};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub fn letter(k: usize) -> &'static str {
    ["A", "B", "C", "D"].get(k).copied().unwrap_or("?")
}

// ───────────────────────────── aba Questões ─────────────────────────────

#[derive(Default)]
pub struct State {
    /// Bancos abertos (com a lista de questões à mostra).
    open: HashSet<i64>,
    cache: Option<(u64, i64, Cache)>,
}

#[derive(Default, Clone)]
struct Cache {
    banks: Vec<Bank>,
    counts: HashMap<i64, Counts>,
    questions: HashMap<i64, Vec<Question>>,
    /// Respostas e acertos dos últimos 30 dias.
    month: (u32, u32),
}

fn cache(app: &mut App, sid: i64) -> Cache {
    let version = app.data_version;
    if app.questions.cache.as_ref().is_none_or(|(v, s, _)| *v != version || *s != sid) {
        let banks = app.db.list_banks(sid).unwrap_or_default();
        let counts = app.db.bank_counts(sid, app.clock.today_end()).unwrap_or_default();
        let questions = banks.iter().map(|b| (b.id, app.db.list_questions(b.id).unwrap_or_default())).collect();
        let since = app.clock.day_start(app.clock.today() - 29);
        let month = app.db.quiz_accuracy_since(sid, since).unwrap_or((0, 0));
        app.questions.cache = Some((version, sid, Cache { banks, counts, questions, month }));
    }
    app.questions.cache.as_ref().map(|c| c.2.clone()).unwrap_or_default()
}

/// Botões do cabeçalho da disciplina na aba Questões.
pub fn header_buttons(app: &mut App, bu: &mut Ui, sid: i64) {
    let c = cache(app, sid);
    let total: u32 = c.counts.values().map(|c| c.total).sum();
    let label = tr!("quiz.solve");
    let w = button_width(bu, &label, None, 13.0, 14.0);
    if button_sized(bu, &label, None, Btn::Primary, vec2(w, 34.0), total > 0).clicked() {
        app.modal = Some(Modal::QuizSetup(Box::new(Setup::new(sid, None, Pick::Due))));
    }
    if button(bu, &tr!("quiz.new_question"), Btn::Secondary, 34.0).clicked() {
        new_question(app, sid, None);
    }
}

/// Abre o editor para uma questão nova no banco dado (ou no primeiro).
pub fn new_question(app: &mut App, sid: i64, bank: Option<i64>) {
    let name = app.subject(sid).map(|s| s.name.clone()).unwrap_or_default();
    let bank = match bank {
        Some(b) => b,
        None => match app.db.first_bank(sid, &name) {
            Ok(b) => {
                app.data_version += 1;
                b
            }
            Err(e) => {
                app.error(e);
                return;
            }
        },
    };
    app.question_editor = Some(Editor::new(Question::new(bank), sid, false));
}

pub fn ui(app: &mut App, ui: &mut Ui, sid: i64) {
    let p = pal();
    let c = cache(app, sid);
    let mut sum = Counts::default();
    for v in c.counts.values() {
        sum.add(*v);
    }
    let area = ui.available_rect_before_wrap();
    let content = Rect::from_min_max(area.min + vec2(28.0, 20.0), area.max - vec2(28.0, 24.0));

    // ── Cards de resumo ──
    let gap = 14.0;
    let cw = (content.width() - 2.0 * gap) / 3.0;
    let r1 = Rect::from_min_size(content.min, vec2(cw, 96.0));
    paint_card(ui.painter(), r1);
    caps(ui.painter(), r1.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("quiz.questions"), p.dim);
    let g = text(ui.painter(), r1.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, &thousands(sum.total as i64), serif(30.0, W::Semi), p.text, 160.0);
    text(ui.painter(), pos2(g.right() + 10.0, r1.top() + 52.0), Align2::LEFT_CENTER, &tr_n!("quiz.n_banks", c.banks.len()), serif(12.5, W::Regular), p.muted, cw - g.width() - 40.0);
    text(ui.painter(), r1.min + vec2(18.0, 76.0), Align2::LEFT_CENTER, &tr_n!("quiz.never_answered", sum.new as usize), serif(12.0, W::Italic), p.muted, cw - 36.0);

    let r2 = r1.translate(vec2(cw + gap, 0.0));
    paint_card(ui.painter(), r2);
    caps(ui.painter(), r2.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("quiz.today"), p.dim);
    let today = sum.due + sum.new;
    let g = text(ui.painter(), r2.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, &today.to_string(), serif(30.0, W::Semi), p.text, 120.0);
    // O botão Revisar só cabe no cartão largo (o cabeçalho já tem "Resolver questões").
    let review_btn = cw >= 300.0;
    let breakdown = tr!("quiz.today_breakdown", due = tr_n!("quiz.n_due", sum.due as usize), new = tr_n!("quiz.n_new", sum.new as usize));
    text(ui.painter(), pos2(g.right() + 10.0, r2.top() + 52.0), Align2::LEFT_CENTER, &breakdown, serif(12.5, W::Regular), p.muted, cw - g.width() - if review_btn { 130.0 } else { 40.0 });
    if review_btn {
        let br = Rect::from_min_size(pos2(r2.right() - 18.0 - 96.0, r2.top() + 34.0), vec2(96.0, 30.0));
        let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if button_sized(&mut bu, &tr!("quiz.review"), None, Btn::Secondary, vec2(96.0, 30.0), today > 0).clicked() {
            app.modal = Some(Modal::QuizSetup(Box::new(Setup::new(sid, None, Pick::Due))));
        }
    }
    text(ui.painter(), r2.min + vec2(18.0, 76.0), Align2::LEFT_CENTER, &tr_n!("quiz.wrong_waiting", sum.wrong as usize), serif(12.0, W::Italic), p.muted, cw - 36.0);

    let r3 = r2.translate(vec2(cw + gap, 0.0));
    paint_card(ui.painter(), r3);
    caps(ui.painter(), r3.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("quiz.accuracy_30"), p.dim);
    let (answers, right) = c.month;
    let acc = if answers > 0 { pct(right as f32 / answers as f32, 0) } else { "—".into() };
    text(ui.painter(), r3.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, &acc, serif(30.0, W::Semi), p.text, 160.0);
    let all = sum.accuracy().map(|a| tr!("quiz.accuracy_all", pct = pct(a, 0))).unwrap_or_default();
    text(ui.painter(), r3.min + vec2(18.0, 76.0), Align2::LEFT_CENTER, &format!("{}{all}", tr_n!("quiz.n_answers", answers as usize)), serif(12.0, W::Italic), p.muted, cw - 36.0);

    // ── Tabela de bancos ──
    let table = Rect::from_min_max(pos2(content.left(), r1.bottom() + 18.0), content.max);
    paint_card(ui.painter(), table);
    let hy = table.top() + 20.0;
    // Colunas mais estreitas em janela estreita, para o nome do banco caber.
    let (act, step) = if table.width() < 640.0 { (96.0, 72.0) } else { (150.0, 100.0) };
    let xs = [table.right() - 18.0 - act - 3.0 * step, table.right() - 18.0 - act - 2.0 * step, table.right() - 18.0 - act - step, table.right() - 18.0 - act];
    caps(ui.painter(), pos2(table.left() + 18.0, hy), Align2::LEFT_CENTER, &tr!("quiz.bank"), p.dim);
    for (x, k) in xs.iter().zip(["quiz.col_total", "quiz.col_new", "quiz.col_due", "quiz.col_accuracy"]) {
        caps(ui.painter(), pos2(*x, hy), Align2::RIGHT_CENTER, &tr!(k), p.dim);
    }
    ui.painter().hline(table.x_range(), table.top() + 40.0, Stroke::new(1.0, p.border));
    let footer_h = 54.0;
    let body = Rect::from_min_max(pos2(table.left() + 1.0, table.top() + 41.0), pos2(table.right() - 1.0, table.bottom() - footer_h));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("banks", sid)).auto_shrink([false, false]).show(&mut bu, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for b in &c.banks {
            let counts = c.counts.get(&b.id).copied().unwrap_or_default();
            bank_row(app, ui, b, counts, xs);
            if app.questions.open.contains(&b.id) {
                let list = c.questions.get(&b.id).cloned().unwrap_or_default();
                for (i, q) in list.iter().enumerate() {
                    question_row(app, ui, i, q);
                }
                if list.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                    text(ui.painter(), pos2(r.left() + 56.0, r.center().y), Align2::LEFT_CENTER, &tr!("quiz.bank_empty"), serif(13.0, W::Italic), p.muted, r.width() - 70.0);
                    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_row));
                }
            }
        }
        if c.banks.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(ui.available_width().min(520.0));
                para(ui, &tr!("quiz.none"), serif(14.0, W::Italic), p.muted);
            });
        }
    });
    ui.painter().hline(table.x_range(), table.bottom() - footer_h, Stroke::new(1.0, p.border));
    let foot = Rect::from_min_max(pos2(table.left() + 18.0, table.bottom() - footer_h), pos2(table.right() - 18.0, table.bottom()));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
    fu.spacing_mut().item_spacing.x = 8.0;
    if button(&mut fu, &tr!("quiz.new_bank"), Btn::Secondary, 30.0).clicked() {
        app.modal = Some(Modal::Bank { id: None, subject: sid, name: String::new() });
    }
    let ai = button(&mut fu, &format!("{}  ▾", tr!("quiz.generate_ai")), Btn::Ghost, 30.0);
    ai_pages_menu(app, &ai, sid, None);
    // A dica do rodapé só aparece se couber ao lado dos botões.
    let room = foot.right() - fu.min_rect().right() - 20.0;
    if room >= 200.0 {
        text(fu.painter(), pos2(foot.right(), foot.center().y), Align2::RIGHT_CENTER, &tr!("quiz.footer_tip"), serif(12.0, W::Italic), p.muted, room.min(420.0));
    }
}

/// Menu com as páginas da disciplina: a escolhida abre a IA em modo questões.
fn ai_pages_menu(app: &mut App, anchor: &egui::Response, sid: i64, bank: Option<i64>) {
    let pages = app.db.list_pages(sid).unwrap_or_default();
    let mut tree = Vec::new();
    page_tree(&pages, None, 0, &mut tree);
    egui::Popup::menu(anchor).frame(popup_frame()).width(300.0).show(|ui| {
        caps_label(ui, &tr!("quiz.ai_from_page"));
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (pg, depth) in tree {
                let label = format!("{}{}", "    ".repeat(depth), pg.title);
                if menu_item(ui, &label, false).clicked() {
                    super::ai::open_questions(app, pg.id, bank);
                }
            }
            if pages.is_empty() {
                label(ui, &tr!("quiz.no_pages"), serif(13.0, W::Italic), pal().muted);
            }
        });
    });
}

/// Páginas na ordem da árvore do caderno, com a profundidade de cada uma.
fn page_tree<'a>(pages: &'a [crate::db::PageMeta], parent: Option<i64>, depth: usize, out: &mut Vec<(&'a crate::db::PageMeta, usize)>) {
    for pg in pages.iter().filter(|p| p.parent_id == parent) {
        out.push((pg, depth));
        if depth < 8 {
            page_tree(pages, Some(pg.id), depth + 1, out);
        }
    }
}

fn bank_row(app: &mut App, ui: &mut Ui, b: &Bank, c: Counts, xs: [f32; 4]) {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line_row));
    let open = app.questions.open.contains(&b.id);
    let x = rect.left() + 17.0;
    text(ui.painter(), pos2(x + 6.0, rect.center().y), Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, serif(10.0, W::Regular), p.dim, 18.0);
    text(ui.painter(), pos2(x + 20.0, rect.center().y), Align2::LEFT_CENTER, &b.name, serif(14.0, W::Semi), p.text, xs[0] - x - 140.0);
    let acc = c.accuracy().map(|a| pct(a, 0)).unwrap_or_else(|| "—".into());
    let cols = [(c.total.to_string(), p.text2, c.total > 0), (c.new.to_string(), p.easy, c.new > 0), (c.due.to_string(), p.good, c.due > 0), (acc, p.text2, c.attempts > 0)];
    for (i, (v, col, on)) in cols.into_iter().enumerate() {
        text(ui.painter(), pos2(xs[i], rect.center().y), Align2::RIGHT_CENTER, &v, serif(14.0, W::Regular), if on { col } else { p.dim }, 80.0);
    }
    let actions = Rect::from_min_max(pos2(rect.right() - if rect.width() < 640.0 { 112.0 } else { 168.0 }, rect.top()), pos2(rect.right() - 17.0, rect.bottom()));
    let mut au = ui.new_child(egui::UiBuilder::new().max_rect(actions).layout(egui::Layout::right_to_left(egui::Align::Center)));
    au.spacing_mut().item_spacing.x = 6.0;
    let more = icon_button(&mut au, "⋯", &tr!("quiz.bank_options"), 28.0);
    egui::Popup::menu(&more).frame(popup_frame()).width(230.0).show(|ui| bank_menu(app, ui, b));
    if hovered && c.total > 0 && button(&mut au, &tr!("quiz.solve_short"), Btn::Primary, 28.0).clicked() {
        app.modal = Some(Modal::QuizSetup(Box::new(Setup::new(b.subject_id, Some(b.id), Pick::Due))));
        return;
    }
    a11y(&resp, egui::accesskit::Role::Row, &b.name, Some(open));
    if resp.clicked() {
        if open {
            app.questions.open.remove(&b.id);
        } else {
            app.questions.open.insert(b.id);
        }
    }
    resp.context_menu(|ui| bank_menu(app, ui, b));
}

fn bank_menu(app: &mut App, ui: &mut Ui, b: &Bank) {
    if menu_item(ui, &tr!("quiz.solve_short"), false).clicked() {
        app.modal = Some(Modal::QuizSetup(Box::new(Setup::new(b.subject_id, Some(b.id), Pick::Due))));
    }
    if menu_item(ui, &tr!("quiz.new_question_here"), false).clicked() {
        new_question(app, b.subject_id, Some(b.id));
    }
    menu_sep(ui);
    if menu_item(ui, &tr!("common.rename"), false).clicked() {
        app.modal = Some(Modal::Bank { id: Some(b.id), subject: b.subject_id, name: b.name.clone() });
    }
    if menu_item(ui, &tr!("quiz.delete_bank"), false).clicked() {
        app.modal = Some(Modal::DeleteBank(b.id));
    }
}

fn question_row(app: &mut App, ui: &mut Ui, i: usize, q: &Question) {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line_row));
    // Bolinha: nunca respondida, última certa ou última errada.
    let dot = match q.last_ok {
        None => p.border_strong,
        Some(true) => p.good,
        Some(false) => p.again,
    };
    let x = rect.left() + 44.0;
    ui.painter().circle_filled(pos2(x, rect.center().y), 4.0, dot);
    let num = text(ui.painter(), pos2(x + 14.0, rect.center().y), Align2::LEFT_CENTER, &format!("{}.", i + 1), serif(12.5, W::Regular), p.dim, 40.0);
    let stem = rich::plain(&q.stem).replace('\n', " ");
    let stem = if stem.trim().is_empty() { tr!("quiz.image_only") } else { stem };
    let right = rect.right() - 17.0;
    let stats = if q.attempts > 0 { tr!("quiz.row_stats", right = q.correct, total = q.attempts) } else { tr!("quiz.row_new") };
    let sr = text(ui.painter(), pos2(right, rect.center().y), Align2::RIGHT_CENTER, &stats, serif(12.0, W::Regular), p.muted, 160.0);
    if !q.image.is_empty() {
        text(ui.painter(), pos2(sr.left() - 12.0, rect.center().y), Align2::RIGHT_CENTER, &tr!("quiz.has_image"), serif(11.5, W::Italic), p.dim, 80.0);
    }
    text(ui.painter(), pos2(num.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, &stem, serif(13.5, W::Regular), p.text2, (sr.left() - 90.0 - num.right()).max(40.0));
    a11y(&resp, egui::accesskit::Role::Row, &stem, None);
    let resp = resp.on_hover_text(tr!("quiz.click_to_edit")).on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        let sid = app.questions.cache.as_ref().map(|c| c.1).unwrap_or(0);
        app.question_editor = Some(Editor::new(q.clone(), sid, true));
    }
    resp.context_menu(|ui| {
        if menu_item(ui, &tr!("review.edit"), false).clicked() {
            let sid = app.questions.cache.as_ref().map(|c| c.1).unwrap_or(0);
            app.question_editor = Some(Editor::new(q.clone(), sid, true));
        }
        if menu_item(ui, &tr!("quiz.delete_question"), false).clicked() {
            app.modal = Some(Modal::DeleteQuestions(vec![q.id]));
        }
    });
}

// ───────────────────────────── editor ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Field {
    Stem,
    Opt(usize),
    Expl,
}

pub struct Editor {
    q: Question,
    subject: i64,
    existing: bool,
    keep: bool,
    focused: Option<Field>,
    sel: Option<(Field, CCursorRange)>,
    pending: Option<(Field, usize, usize)>,
    first: bool,
}

impl Editor {
    fn new(q: Question, subject: i64, existing: bool) -> Editor {
        Editor { q, subject, existing, keep: false, focused: None, sel: None, pending: None, first: true }
    }

    fn text(&mut self, f: Field) -> &mut String {
        match f {
            Field::Stem => &mut self.q.stem,
            Field::Opt(k) => &mut self.q.options[k],
            Field::Expl => &mut self.q.explanation,
        }
    }

    fn id(&self, f: Field) -> egui::Id {
        egui::Id::unique(("question_field", self.q.id, f))
    }

    fn sel_bytes(&mut self, f: Field) -> (usize, usize) {
        let sel = self.sel;
        let t = self.text(f);
        match sel {
            Some((sf, r)) if sf == f => {
                let (a, b) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
                (rich::byte_of_char(t, a), rich::byte_of_char(t, b))
            }
            _ => (t.len(), t.len()),
        }
    }

    fn apply(&mut self, f: Field, change: impl FnOnce(&str, usize, usize) -> (String, usize, usize)) {
        let (a, b) = self.sel_bytes(f);
        let t = self.text(f).clone();
        let (s, na, nb) = change(&t, a, b);
        self.pending = Some((f, rich::char_of_byte(&s, na), rich::char_of_byte(&s, nb)));
        *self.text(f) = s;
    }
}

pub fn show_editor(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.question_editor.take() else { return };
    let p = pal();
    let screen = ctx.content_rect();
    let size = vec2(820.0f32.min(screen.width() - 40.0), 740.0f32.min(screen.height() - 60.0));
    let mut close = false;
    let mut submit = false;
    egui::Modal::new(egui::Id::unique("question_editor"))
        .backdrop_color(p.scrim)
        .frame(egui::Frame::NONE.fill(p.window).corner_radius(16).shadow(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: Color32::from_black_alpha(if p.dark { 140 } else { 46 }) }).stroke(Stroke::new(1.0, p.border_control)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            let rect = Rect::from_min_size(ui.max_rect().min, size);
            editor_body(app, &mut ed, ui, rect, &mut close, &mut submit);
        });
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close = true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        submit = true;
    }
    if submit {
        if !ed.q.complete() {
            app.toast(tr!("quiz.incomplete"));
        } else {
            match app.db.save_question(&mut ed.q) {
                Ok(_) => {
                    app.data_version += 1;
                    app.questions.open.insert(ed.q.bank_id);
                    if ed.existing || !ed.keep {
                        app.toast(tr!(if ed.existing { "quiz.saved" } else { "quiz.added" }));
                        close = true;
                    } else {
                        // Continuar adicionando: mesma disciplina e banco, campos limpos.
                        app.toast(tr!("quiz.added"));
                        let mut next = Editor::new(Question::new(ed.q.bank_id), ed.subject, false);
                        next.keep = true;
                        ed = next;
                    }
                }
                Err(e) => app.error(e),
            }
        }
    }
    if !close {
        app.question_editor = Some(ed);
    }
}

fn editor_body(app: &mut App, ed: &mut Editor, ui: &mut Ui, rect: Rect, close: &mut bool, submit: &mut bool) {
    let p = pal();
    // ── Cabeçalho ──
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let t = text(ui.painter(), pos2(head.left() + 26.0, head.center().y), Align2::LEFT_CENTER, &tr!(if ed.existing { "quiz.edit_question" } else { "quiz.new_question" }), serif(19.0, W::Semi), p.text, 320.0);
    if let Some(title) = ed.q.source_page.and_then(|id| app.db.page(id).ok().flatten()).map(|pg| pg.title) {
        let label = format!("{}  {title}", tr!("note.source"));
        let w = text_width(ui.painter(), &label, serif(12.0, W::Regular)) + 20.0;
        let cr = Rect::from_min_size(pos2(t.right() + 14.0, head.center().y - 12.0), vec2(w.min(360.0), 24.0));
        ui.painter().rect(cr, CornerRadius::same(255), p.chip, Stroke::new(1.0, p.border), StrokeKind::Inside);
        text(ui.painter(), pos2(cr.left() + 10.0, cr.center().y), Align2::LEFT_CENTER, &label, serif(12.0, W::Regular), p.text2, cr.width() - 20.0);
    }
    let close_r = Rect::from_center_size(pos2(head.right() - 36.0, head.center().y), vec2(40.0, 28.0));
    let cresp = ui.interact(close_r, ui.scope_id().with("q_close"), Sense::click());
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
        checkbox(&mut fu, &mut ed.keep, &tr!("quiz.keep_adding"));
    }
    fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button_kbd(ui, &tr!(if ed.existing { "common.save" } else { "common.add" }), Some("Ctrl ↵"), Btn::Primary, 36.0).clicked() {
            *submit = true;
        }
        if button(ui, &tr!("common.cancel"), Btn::Secondary, 36.0).clicked() {
            *close = true;
        }
        if ed.existing && button(ui, &tr!("quiz.delete_question"), Btn::Danger, 36.0).clicked() {
            app.modal = Some(Modal::DeleteQuestions(vec![ed.q.id]));
            *close = true;
        }
    });

    // ── Corpo ──
    let mid = Rect::from_min_max(pos2(rect.left(), head.bottom()), pos2(rect.right(), foot.top()));
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(mid.shrink2(vec2(26.0, 18.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt("q_editor").auto_shrink([false, false]).show(&mut mu, |ui| {
        ui.spacing_mut().item_spacing.y = 14.0;
        let w = ui.available_width() - 6.0;
        // Banco.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            caps_label(ui, &tr!("quiz.bank"));
            let banks = app.db.list_banks(ed.subject).unwrap_or_default();
            let cur = banks.iter().find(|b| b.id == ed.q.bank_id).map(|b| b.name.clone()).unwrap_or_default();
            let color = app.subject(ed.subject).map(|s| subject_color(s.color));
            select_field(ui, "q_bank", &cur, color, 320.0, 36.0, |ui| {
                for b in &banks {
                    if menu_item(ui, &b.name, b.id == ed.q.bank_id).clicked() {
                        ed.q.bank_id = b.id;
                    }
                }
            });
        });
        // Enunciado e imagem.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            caps_label(ui, &tr!("quiz.stem"));
            rich_box(app, ed, ui, Field::Stem, w, 3, true);
            if !ed.q.image.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let uri = crate::media::uri(&app.db.media_dir(), &ed.q.image);
                    richtext::image(ui, &uri, (w - 140.0).min(420.0), 220.0, 8, Sense::hover());
                    ui.vertical(|ui| {
                        if button(ui, &tr!("quiz.change_image"), Btn::Secondary, 30.0).clicked() {
                            pick_image(app, ed);
                        }
                        if button(ui, &tr!("quiz.remove_image"), Btn::Ghost, 30.0).clicked() {
                            ed.q.image.clear();
                        }
                    });
                });
            }
        });
        // Alternativas.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
            caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("quiz.options"), p.dim);
            text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &tr!("quiz.options_tip"), serif(11.5, W::Italic), p.muted, w * 0.6);
            for k in 0..OPTIONS {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let right = ed.q.answer == k;
                    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 40.0), Sense::click());
                    let c = r.center();
                    if right {
                        ui.painter().circle_filled(c, 13.0, p.good);
                    } else {
                        ui.painter().circle_stroke(c, 12.5, Stroke::new(1.2, if resp.hovered() { p.good } else { p.border_strong }));
                    }
                    ui.painter().text(c, Align2::CENTER_CENTER, letter(k), serif(13.0, W::Semi), if right { p.on_accent } else { p.text2 });
                    let tip = tr!("quiz.mark_right", letter = letter(k));
                    a11y(&resp, egui::accesskit::Role::RadioButton, &tip, Some(right));
                    if resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        ed.q.answer = k;
                    }
                    rich_box(app, ed, ui, Field::Opt(k), w - 40.0, 1, false);
                });
            }
        });
        // Explicação.
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            caps_label(ui, &tr!("quiz.explanation"));
            rich_box(app, ed, ui, Field::Expl, w, 2, true);
        });
    });
}

fn pick_image(app: &mut App, ed: &mut Editor) {
    if let Some(path) = rfd::FileDialog::new().add_filter(tr!("note.images_filter"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() {
        match crate::media::store_file(&app.db.media_dir(), &path) {
            Ok(s) => {
                let _ = app.db.add_media(&s);
                ed.q.image = s.file_name;
            }
            Err(e) => app.error(e),
        }
    }
}

/// Campo de texto rico do editor: barra opcional (B I U, cores e, no
/// enunciado, Imagem) e a marcação desenhada enquanto digita.
fn rich_box(app: &mut App, ed: &mut Editor, ui: &mut Ui, f: Field, w: f32, rows: usize, toolbar: bool) {
    let p = pal();
    let id = ed.id(f);
    let focused = ed.focused == Some(f);
    let look = match f {
        Field::Stem => Look { size: 16.0, semi: false, italic: false, color: p.text, line_height: Some(26.0) },
        _ => Look { size: 14.5, semi: false, italic: false, color: p.text, line_height: Some(22.0) },
    };
    if focused {
        let ctx = ui.ctx().clone();
        for (k, m) in [(Key::B, "**"), (Key::I, "*"), (Key::U, "__")] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, k)) {
                ed.apply(f, |t, a, b| rich::toggle_wrap(t, a, b, m));
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::H)) {
            ed.apply(f, |t, a, b| rich::set_color(t, a, b, true, Some(rich::Ink::Yellow)));
        }
    }
    let stroke = if focused { Stroke::new(1.0, p.accent) } else { Stroke::new(1.0, p.border_control) };
    let resp = egui::Frame::NONE.fill(p.input).stroke(stroke).corner_radius(10).show(ui, |ui| {
        ui.set_width(w);
        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
        if toolbar {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 5)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_width(w - 12.0);
                    for (g, m, key) in [("B", "**", "note.bold"), ("I", "*", "note.italic"), ("U", "__", "note.underline")] {
                        if fmt_button(ui, g, &tr!(key)).clicked() {
                            ed.apply(f, |t, a, b| rich::toggle_wrap(t, a, b, m));
                        }
                    }
                    let cr = color_button(ui, 26.0, &tr!("page.colors"));
                    if let Some((bg, ink)) = color_menu(&cr) {
                        ed.apply(f, |t, a, b| rich::set_color(t, a, b, bg, ink));
                    }
                    if f == Field::Stem && button(ui, &tr!(if ed.q.image.is_empty() { "quiz.add_image" } else { "quiz.change_image" }), Btn::Ghost, 26.0).clicked() {
                        pick_image(app, ed);
                    }
                });
            });
            let (sep, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
            ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, p.line));
        }
        let margin = if toolbar { egui::Margin { left: 14, right: 14, top: 10, bottom: 12 } } else { egui::Margin::symmetric(12, 8) };
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            let mut value = std::mem::take(ed.text(f));
            let mut lay = richtext::layouter(look);
            let out = egui::TextEdit::multiline(&mut value).id(id).frame(egui::Frame::NONE).desired_width(w - 28.0).desired_rows(rows).layouter(&mut lay).show(ui);
            *ed.text(f) = value;
            let r = out.response.response.clone();
            if r.has_focus() {
                ed.focused = Some(f);
                if let Some(c) = out.cursor_range {
                    ed.sel = Some((f, c));
                }
            }
            if ed.first && f == Field::Stem && !ed.existing {
                r.request_focus();
                ed.first = false;
            }
            if let Some((pf, a, b)) = ed.pending {
                if pf == f {
                    ed.pending = None;
                    let mut st = out.state.clone();
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                    st.store(ui.ctx(), id);
                    r.request_focus();
                }
            }
        });
    });
    if focused {
        ui.painter().rect_stroke(resp.response.rect.expand(1.0), CornerRadius::same(12), Stroke::new(3.0, p.accent_soft), StrokeKind::Outside);
    }
    // Imagem colada ou solta na janela vira a imagem do enunciado.
    if f == Field::Stem {
        if let Some(name) = app.pasted_image.take() {
            ed.q.image = name;
        }
        for path in std::mem::take(&mut app.dropped) {
            if crate::media::is_image_path(&path) {
                if let Ok(s) = crate::media::store_file(&app.db.media_dir(), &path) {
                    let _ = app.db.add_media(&s);
                    ed.q.image = s.file_name;
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

// ───────────────────────────── simulado: escolha ─────────────────────────────

pub struct Setup {
    pub subject: i64,
    pub bank: Option<i64>,
    pub pick: Pick,
    /// 0 = todas.
    pub count: u32,
    /// Corrigir a cada questão (senão, só no final).
    pub each: bool,
}

const SETUP_COUNTS: [u32; 4] = [10, 20, 30, 0];
const PICKS: [Pick; 4] = [Pick::Due, Pick::All, Pick::Wrong, Pick::New];

impl Setup {
    pub fn new(subject: i64, bank: Option<i64>, pick: Pick) -> Setup {
        Setup { subject, bank, pick, count: 20, each: true }
    }

    fn banks(&self, app: &App) -> Vec<i64> {
        match self.bank {
            Some(b) => vec![b],
            None => app.db.list_banks(self.subject).unwrap_or_default().iter().map(|b| b.id).collect(),
        }
    }
}

/// Conteúdo do diálogo "Resolver questões".
pub fn setup_ui(app: &mut App, ui: &mut Ui, s: &mut Setup, keep: &mut bool, enter: bool) {
    let p = pal();
    ui.set_width(520.0);
    label(ui, &tr!("quiz.setup_title"), serif(19.0, W::Semi), p.text);
    ui.add_space(4.0);
    caps_label(ui, &tr!("quiz.bank"));
    let banks = app.db.list_banks(s.subject).unwrap_or_default();
    let cur = s.bank.and_then(|b| banks.iter().find(|x| x.id == b)).map(|b| b.name.clone()).unwrap_or_else(|| tr!("quiz.all_banks"));
    select_field(ui, "setup_bank", &cur, app.subject(s.subject).map(|x| subject_color(x.color)), 472.0, 36.0, |ui| {
        if menu_item(ui, &tr!("quiz.all_banks"), s.bank.is_none()).clicked() {
            s.bank = None;
        }
        for b in &banks {
            if menu_item(ui, &b.name, s.bank == Some(b.id)).clicked() {
                s.bank = Some(b.id);
            }
        }
    });
    caps_label(ui, &tr!("quiz.which"));
    let labels = [tr!("quiz.pick_due"), tr!("quiz.pick_all"), tr!("quiz.pick_wrong"), tr!("quiz.pick_new")];
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut pi = PICKS.iter().position(|x| *x == s.pick).unwrap_or(0);
    if segmented(ui, &refs, &mut pi, 30.0, false) {
        s.pick = PICKS[pi];
    }
    let available = app.db.pick_questions(&s.banks(app), s.pick, app.clock.today_end()).unwrap_or_default().iter().filter(|q| q.complete()).count();
    para(ui, &tr_n!("quiz.available", available), serif(12.5, W::Italic), if available > 0 { p.muted } else { p.again });
    caps_label(ui, &tr!("quiz.how_many"));
    let clabels: Vec<String> = SETUP_COUNTS.iter().map(|c| if *c == 0 { tr!("quiz.count_all") } else { c.to_string() }).collect();
    let crefs: Vec<&str> = clabels.iter().map(String::as_str).collect();
    let mut ci = SETUP_COUNTS.iter().position(|c| *c == s.count).unwrap_or(1);
    if segmented(ui, &crefs, &mut ci, 30.0, false) {
        s.count = SETUP_COUNTS[ci];
    }
    caps_label(ui, &tr!("quiz.correction"));
    let (a, b) = (tr!("quiz.correct_each"), tr!("quiz.correct_end"));
    let mut ei = if s.each { 0 } else { 1 };
    if segmented(ui, &[a.as_str(), b.as_str()], &mut ei, 30.0, false) {
        s.each = ei == 0;
    }
    para(ui, &tr!(if s.each { "quiz.correct_each_tip" } else { "quiz.correct_end_tip" }), serif(12.5, W::Italic), p.muted);
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let label = tr!("quiz.start");
        let w = button_width(ui, &label, None, 13.0, 14.0);
        if (button_sized(ui, &label, None, Btn::Primary, vec2(w, 36.0), available > 0).clicked() || (enter && available > 0)) && start(app, s) {
            *keep = false;
        }
        if button(ui, &tr!("common.cancel"), Btn::Secondary, 36.0).clicked() {
            *keep = false;
        }
    });
}

// ───────────────────────────── simulado: resolução ─────────────────────────────

struct Item {
    q: Question,
    /// Posição na tela → índice da alternativa (embaralhadas).
    order: [usize; OPTIONS],
    /// Posição escolhida na tela.
    chosen: Option<usize>,
    recorded: bool,
    ms: i64,
}

impl Item {
    fn ok(&self) -> Option<bool> {
        self.chosen.map(|c| self.order[c] == self.q.answer)
    }

    /// Posição na tela da alternativa certa.
    fn right_pos(&self) -> usize {
        self.order.iter().position(|o| *o == self.q.answer).unwrap_or(0)
    }
}

pub struct Quiz {
    title: String,
    color: Color32,
    subject: i64,
    each: bool,
    items: Vec<Item>,
    idx: usize,
    finished: bool,
    started: Instant,
    shown: Instant,
    back: Screen,
    data_version: u64,
}

fn shuffled_order() -> [usize; OPTIONS] {
    use rand::seq::SliceRandom;
    let mut o = [0, 1, 2, 3];
    o.shuffle(&mut rand::rng());
    o
}

fn begin(app: &mut App, subject: i64, title: String, questions: Vec<Question>, each: bool) {
    let color = app.subject(subject).map(|s| subject_color(s.color)).unwrap_or(pal().accent);
    let items = questions.into_iter().map(|q| Item { q, order: shuffled_order(), chosen: None, recorded: false, ms: 0 }).collect();
    let back = match app.screen {
        Screen::Quiz => Screen::Subject(subject, super::Tab::Questions),
        s => s,
    };
    app.quiz = Some(Quiz { title, color, subject, each, items, idx: 0, finished: false, started: Instant::now(), shown: Instant::now(), back, data_version: app.data_version });
    app.go(Screen::Quiz);
}

/// Monta o simulado. Devolve `false` se não houver questão.
pub fn start(app: &mut App, s: &Setup) -> bool {
    if let Some(p) = app.page.as_mut() {
        p.flush(&app.db);
    }
    let mut qs: Vec<Question> = app.db.pick_questions(&s.banks(app), s.pick, app.clock.today_end()).unwrap_or_default();
    qs.retain(Question::complete);
    if qs.is_empty() {
        app.toast(tr!("quiz.nothing"));
        return false;
    }
    use rand::seq::SliceRandom;
    if s.pick == Pick::Due {
        // Revisão: primeiro as vencidas (as mais atrasadas antes), depois as novas.
        qs.sort_by_key(|q| (q.is_new(), q.due));
    } else {
        qs.shuffle(&mut rand::rng());
    }
    if s.count > 0 {
        qs.truncate(s.count as usize);
    }
    qs.shuffle(&mut rand::rng());
    let subject = app.subject(s.subject).map(|x| x.name.clone()).unwrap_or_default();
    let bank = s.bank.and_then(|b| app.db.bank(b).ok().flatten()).map(|b| b.name).unwrap_or_else(|| tr!("quiz.all_banks"));
    begin(app, s.subject, format!("{subject} › {bank}"), qs, s.each);
    true
}

/// Grava as respostas ainda não gravadas.
fn record(app: &mut App, quiz: &mut Quiz) {
    let mut any = false;
    for it in quiz.items.iter_mut().filter(|i| !i.recorded) {
        let Some(c) = it.chosen else { continue };
        match app.db.answer_question(&it.q, it.order[c], it.ms, &app.clock) {
            Ok(q) => {
                it.q = q;
                it.recorded = true;
                any = true;
            }
            Err(e) => {
                app.error(e);
                break;
            }
        }
    }
    if any {
        app.data_version += 1;
        quiz.data_version = app.data_version;
    }
}

fn exit(app: &mut App) {
    let Some(mut quiz) = app.quiz.take() else { return };
    record(app, &mut quiz);
    let answered = quiz.items.iter().filter(|i| i.chosen.is_some()).count();
    if answered > 0 && !quiz.finished {
        let right = quiz.items.iter().filter(|i| i.ok() == Some(true)).count();
        app.toast(tr_n!("quiz.toast_saved", answered, pct = pct(right as f32 / answered as f32, 0)));
    }
    app.go(quiz.back);
}

fn choose(quiz: &mut Quiz, pos: usize) {
    let each = quiz.each;
    let it = &mut quiz.items[quiz.idx];
    // Na correção a cada questão, a primeira escolha vale.
    if each && it.chosen.is_some() {
        return;
    }
    if it.chosen.is_none() {
        it.ms = quiz.shown.elapsed().as_millis() as i64;
    }
    it.chosen = Some(pos);
}

fn go_to(quiz: &mut Quiz, idx: usize) {
    if idx < quiz.items.len() {
        quiz.idx = idx;
        quiz.shown = Instant::now();
    }
}

fn next(app: &mut App, quiz: &mut Quiz) {
    if quiz.idx + 1 < quiz.items.len() {
        go_to(quiz, quiz.idx + 1);
    } else {
        quiz.finished = true;
        record(app, quiz);
    }
}

pub fn quiz_ui(app: &mut App, ui: &mut Ui) {
    let Some(mut quiz) = app.quiz.take() else {
        app.go(Screen::Dashboard);
        return;
    };
    // A questão foi editada (E)? Recarrega o texto.
    if quiz.data_version != app.data_version {
        quiz.data_version = app.data_version;
        for it in quiz.items.iter_mut() {
            if let Ok(Some(q)) = app.db.question(it.q.id) {
                it.q.stem = q.stem;
                it.q.image = q.image;
                it.q.options = q.options;
                it.q.answer = q.answer;
                it.q.explanation = q.explanation;
            }
        }
    }
    let mut leave = false;
    keyboard(app, ui, &mut quiz, &mut leave);
    if !leave {
        draw(app, ui, &mut quiz, &mut leave);
    }
    app.quiz = Some(quiz);
    if leave {
        exit(app);
    }
}

fn keyboard(app: &mut App, ui: &Ui, quiz: &mut Quiz, leave: &mut bool) {
    if app.question_editor.is_some() || app.modal.is_some() || app.palette.is_some() {
        return;
    }
    let ctx = ui.ctx();
    let key = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if key(Key::Escape) {
        *leave = true;
        return;
    }
    if quiz.finished {
        if key(Key::Enter) {
            *leave = true;
        }
        return;
    }
    let it = &quiz.items[quiz.idx];
    let revealed = quiz.each && it.chosen.is_some();
    for (pos, keys) in [(0, [Key::Num1, Key::A]), (1, [Key::Num2, Key::B]), (2, [Key::Num3, Key::C]), (3, [Key::Num4, Key::D])] {
        if keys.iter().any(|k| key(*k)) && !revealed {
            choose(quiz, pos);
            return;
        }
    }
    if key(Key::E) {
        let q = quiz.items[quiz.idx].q.clone();
        app.question_editor = Some(Editor::new(q, quiz.subject, true));
        return;
    }
    if !quiz.each && key(Key::ArrowLeft) && quiz.idx > 0 {
        go_to(quiz, quiz.idx - 1);
        return;
    }
    let chosen = quiz.items[quiz.idx].chosen.is_some();
    if (key(Key::Enter) || key(Key::Space) || (!quiz.each && key(Key::ArrowRight))) && (chosen || !quiz.each) {
        next(app, quiz);
    }
}

fn draw(app: &mut App, ui: &mut Ui, quiz: &mut Quiz, leave: &mut bool) {
    let p = pal();
    let area = ui.max_rect();
    // ── Barra superior ──
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 60.0));
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(bar.min + vec2(24.0, 0.0), pos2(bar.center().x - 160.0, bar.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if button_kbd(&mut lu, &tr!("review.exit"), Some("Esc"), Btn::Ghost, 34.0).clicked() {
        *leave = true;
        return;
    }
    lu.add_space(6.0);
    let (dr, _) = lu.allocate_exact_size(vec2(lu.available_width(), 20.0), Sense::hover());
    lu.painter().circle_filled(pos2(dr.left() + 4.0, dr.center().y), 4.0, quiz.color);
    text(lu.painter(), pos2(dr.left() + 15.0, dr.center().y), Align2::LEFT_CENTER, &quiz.title, serif(13.0, W::Regular), p.text2, dr.width() - 20.0);
    let n = quiz.items.len();
    if !quiz.finished {
        let c = bar.center();
        text(ui.painter(), pos2(c.x, c.y - 7.0), Align2::CENTER_CENTER, &tr!("quiz.progress", n = quiz.idx + 1, total = n), serif(13.5, W::Semi), p.text, 300.0);
        let pr = Rect::from_center_size(pos2(c.x, c.y + 12.0), vec2(220.0, 4.0));
        let done = quiz.items.iter().filter(|i| i.chosen.is_some()).count();
        progress(ui.painter(), pr, done as f32 / n.max(1) as f32, quiz.color);
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(bar.center().x + 160.0, bar.top()), bar.max - vec2(24.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        ru.spacing_mut().item_spacing.x = 10.0;
        if button_kbd(&mut ru, &tr!("review.edit"), Some("E"), Btn::Ghost, 34.0).clicked() {
            let q = quiz.items[quiz.idx].q.clone();
            app.question_editor = Some(Editor::new(q, quiz.subject, true));
        }
        if quiz.each {
            let right = quiz.items.iter().filter(|i| i.ok() == Some(true)).count();
            let wrong = quiz.items.iter().filter(|i| i.ok() == Some(false)).count();
            let (r, _) = ru.allocate_exact_size(vec2(110.0, 20.0), Sense::hover());
            let a = text(ru.painter(), r.right_center(), Align2::RIGHT_CENTER, &format!("✗ {wrong}"), serif(13.5, W::Semi), if wrong > 0 { p.again } else { p.dim }, 50.0);
            text(ru.painter(), pos2(a.left() - 12.0, r.center().y), Align2::RIGHT_CENTER, &format!("✓ {right}"), serif(13.5, W::Semi), if right > 0 { p.good } else { p.dim }, 50.0);
        }
    }
    let body = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    if quiz.finished {
        results(app, ui, quiz, body, leave);
    } else {
        question_view(app, ui, quiz, body);
    }
}

/// Uma alternativa: a letra num círculo e o texto, com as cores da correção.
fn option_row(ui: &mut Ui, pos: usize, text_s: &str, w: f32, state: OptState, clickable: bool) -> egui::Response {
    let p = pal();
    let look = Look { size: 16.0, semi: false, italic: false, color: p.text, line_height: Some(24.0) };
    let text_w = w - 76.0;
    let (job, _) = richtext::job(text_s, &look, false, text_w);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let h = (galley.size().y + 26.0).max(54.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), if clickable { Sense::click() } else { Sense::hover() });
    let hovered = clickable && resp.hovered();
    let (fill, stroke, circle, glyph_c) = match state {
        OptState::Right => (soft(p.good, 0.14), p.good, p.good, p.on_accent),
        OptState::Wrong => (soft(p.again, 0.12), p.again, p.again, p.on_accent),
        OptState::Chosen => (p.accent_soft, p.accent, p.accent, p.on_accent),
        OptState::Faded => (p.card, p.border, p.chip, p.dim),
        OptState::Idle if hovered => (p.hover, p.border_strong, p.chip, p.text),
        OptState::Idle => (p.card, p.border, p.chip, p.text2),
    };
    ui.painter().rect(rect, CornerRadius::same(12), fill, Stroke::new(if matches!(state, OptState::Idle | OptState::Faded) { 1.0 } else { 1.5 }, stroke), StrokeKind::Inside);
    let c = pos2(rect.left() + 30.0, rect.center().y);
    ui.painter().circle_filled(c, 14.0, circle);
    ui.painter().text(c, Align2::CENTER_CENTER, letter(pos), serif(13.5, W::Semi), glyph_c);
    let color = if state == OptState::Faded { p.muted } else { p.text };
    ui.painter().galley(pos2(rect.left() + 58.0, rect.center().y - galley.size().y / 2.0), galley, color);
    let mark = match state {
        OptState::Right => Some(("✓", p.good)),
        OptState::Wrong => Some(("✗", p.again)),
        _ => None,
    };
    if let Some((m, col)) = mark {
        ui.painter().text(pos2(rect.right() - 20.0, rect.center().y), Align2::CENTER_CENTER, m, serif(16.0, W::Semi), col);
    }
    let label = format!("{} {}", letter(pos), rich::plain(text_s));
    a11y(&resp, egui::accesskit::Role::RadioButton, &label, Some(matches!(state, OptState::Chosen | OptState::Right | OptState::Wrong)));
    if clickable { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptState {
    Idle,
    Chosen,
    Right,
    Wrong,
    Faded,
}

fn opt_state(it: &Item, pos: usize, revealed: bool) -> OptState {
    let chosen = it.chosen == Some(pos);
    if !revealed {
        return if chosen { OptState::Chosen } else { OptState::Idle };
    }
    if pos == it.right_pos() {
        OptState::Right
    } else if chosen {
        OptState::Wrong
    } else {
        OptState::Faded
    }
}

/// Enunciado, imagem e alternativas de uma questão.
fn question_card(app: &mut App, ui: &mut Ui, it: &Item, revealed: bool, w: f32, clickable: bool, verdict: bool) -> Option<usize> {
    let p = pal();
    let media = app.db.media_dir();
    let mut picked = None;
    if !it.q.stem.trim().is_empty() {
        richtext::field(ui, &it.q.stem, Look { size: 20.0, semi: false, italic: false, color: p.text, line_height: Some(30.0) }, &media, 360.0);
    }
    if !it.q.image.is_empty() {
        let r = richtext::image(ui, &crate::media::uri(&media, &it.q.image), w, 340.0, 10, Sense::click());
        if r.on_hover_text(tr!("quiz.zoom")).on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
            app.modal = Some(Modal::Viewer(it.q.image.clone(), 1.0));
        }
    }
    ui.add_space(4.0);
    for pos in 0..OPTIONS {
        let opt = &it.q.options[it.order[pos]];
        if option_row(ui, pos, opt, w, opt_state(it, pos, revealed), clickable).clicked() {
            picked = Some(pos);
        }
    }
    if revealed && verdict {
        let ok = it.ok() == Some(true);
        let msg = if ok { tr!("quiz.right") } else { tr!("quiz.wrong", letter = letter(it.right_pos())) };
        label(ui, &msg, serif(15.0, W::Semi), if ok { p.good } else { p.again });
    }
    if revealed && !it.q.explanation.trim().is_empty() {
        egui::Frame::NONE.fill(p.accent_tint).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(w - 34.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            caps_label(ui, &tr!("quiz.explanation"));
            richtext::field(ui, &it.q.explanation, Look { size: 15.0, semi: false, italic: false, color: p.text2, line_height: Some(23.0) }, &media, 260.0);
        });
    }
    picked
}

fn question_view(app: &mut App, ui: &mut Ui, quiz: &mut Quiz, body: Rect) {
    let p = pal();
    let footer_h = 84.0;
    let main = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - footer_h));
    let card_w = 760.0f32.min(main.width() - 48.0);
    let idx = quiz.idx;
    let revealed = quiz.each && quiz.items[idx].chosen.is_some();
    let mut picked = None;
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(main.shrink2(vec2(24.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    egui::ScrollArea::vertical().id_salt(("quiz_q", quiz.items[idx].q.id)).auto_shrink([false, false]).show(&mut mu, |ui| {
        ui.vertical_centered(|ui| {
            let frame = egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(16).inner_margin(egui::Margin { left: 44, right: 44, top: 32, bottom: 32 });
            ui.allocate_ui(vec2(card_w, 0.0), |ui| {
                frame.show(ui, |ui| {
                    let w = card_w - 88.0;
                    ui.set_width(w);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 12.0;
                        let it = &quiz.items[idx];
                        let status = match (it.q.last_ok, it.q.attempts) {
                            (_, 0) => tr!("quiz.status_new"),
                            (Some(true), _) => tr!("quiz.status_last_right", right = it.q.correct, total = it.q.attempts),
                            _ => tr!("quiz.status_last_wrong", right = it.q.correct, total = it.q.attempts),
                        };
                        caps(ui.painter(), ui.cursor().min + vec2(0.0, 8.0), Align2::LEFT_CENTER, &format!("{} {} · {status}", tr!("quiz.question"), idx + 1), p.dim);
                        ui.add_space(14.0);
                        picked = question_card(app, ui, it, revealed, w, !revealed, true);
                    });
                });
            });
        });
    });
    if let Some(pos) = picked {
        choose(quiz, pos);
    }

    // ── Rodapé: navegação ──
    let foot = Rect::from_min_max(pos2(body.left(), body.bottom() - footer_h), body.max);
    let inner = Rect::from_center_size(foot.center(), vec2(card_w, 44.0));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    fu.spacing_mut().item_spacing.x = 10.0;
    let last = quiz.idx + 1 == quiz.items.len();
    let chosen = quiz.items[quiz.idx].chosen.is_some();
    if !quiz.each && button_sized(&mut fu, &tr!("quiz.previous"), None, Btn::Secondary, vec2(120.0, 40.0), quiz.idx > 0).clicked() {
        go_to(quiz, quiz.idx - 1);
    }
    let tip = if quiz.each { tr!("quiz.keys_each") } else { tr!("quiz.keys_end") };
    let tw = fu.available_width() - 200.0;
    let (tr_, _) = fu.allocate_exact_size(vec2(tw.max(10.0), 20.0), Sense::hover());
    text(fu.painter(), tr_.center(), Align2::CENTER_CENTER, &keys(&tip), serif(12.0, W::Italic), p.muted, tr_.width());
    fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = if last { tr!("quiz.finish") } else { tr!("quiz.next") };
        let enabled = chosen || !quiz.each;
        let w = button_width(ui, &label, Some("↵"), 13.0, 14.0).max(140.0);
        if button_sized(ui, &label, Some("↵"), if enabled { Btn::Primary } else { Btn::Secondary }, vec2(w, 40.0), enabled).clicked() {
            next(app, quiz);
        }
    });
}

fn results(app: &mut App, ui: &mut Ui, quiz: &mut Quiz, body: Rect, leave: &mut bool) {
    let p = pal();
    let n = quiz.items.len();
    let right = quiz.items.iter().filter(|i| i.ok() == Some(true)).count();
    let wrong = quiz.items.iter().filter(|i| i.ok() == Some(false)).count();
    let blank = n - right - wrong;
    let card_w = 760.0f32.min(body.width() - 48.0);
    let mut redo: Option<Vec<Question>> = None;
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(24.0, 20.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    egui::ScrollArea::vertical().id_salt("quiz_results").auto_shrink([false, false]).show(&mut mu, |ui| {
        ui.vertical_centered(|ui| {
            ui.allocate_ui(vec2(card_w, 0.0), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 14.0;
                    egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(16).inner_margin(egui::Margin::symmetric(32, 26)).show(ui, |ui| {
                        ui.set_width(card_w - 64.0);
                        ui.spacing_mut().item_spacing.y = 14.0;
                        caps_label(ui, &tr!("quiz.result"));
                        let score = pct(right as f32 / n.max(1) as f32, 0);
                        label(ui, &tr!("quiz.score", right = right, total = n, pct = score), serif(24.0, W::Semi), p.text);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 28.0;
                            let mins = (quiz.started.elapsed().as_secs_f64() / 60.0).ceil();
                            for (v, l, c) in [(right.to_string(), tr!("quiz.stat_right"), p.good), (wrong.to_string(), tr!("quiz.stat_wrong"), p.again), (blank.to_string(), tr!("quiz.stat_blank"), p.muted), (tr!("time.minutes", n = mins), tr!("review.stat_studying"), p.text)] {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    label(ui, &v, serif(24.0, W::Semi), c);
                                    label(ui, &l, serif(12.0, W::Regular), p.muted);
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            if button_kbd(ui, &tr!("review.back"), Some("↵"), Btn::Primary, 36.0).clicked() {
                                *leave = true;
                            }
                            if wrong + blank > 0 && button(ui, &tr!("quiz.redo_wrong"), Btn::Secondary, 36.0).clicked() {
                                redo = Some(quiz.items.iter().filter(|i| i.ok() != Some(true)).map(|i| i.q.clone()).collect());
                            }
                        });
                        para(ui, &tr!("quiz.schedule_note"), serif(12.5, W::Italic), p.muted);
                    });
                    // Correção de cada questão.
                    for (i, it) in quiz.items.iter().enumerate() {
                        egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(14).inner_margin(egui::Margin::symmetric(28, 20)).show(ui, |ui| {
                            let w = card_w - 56.0;
                            ui.set_width(w);
                            ui.spacing_mut().item_spacing.y = 10.0;
                            let (msg, col) = match it.ok() {
                                Some(true) => (tr!("quiz.right"), p.good),
                                Some(false) => (tr!("quiz.wrong", letter = letter(it.right_pos())), p.again),
                                None => (tr!("quiz.blank", letter = letter(it.right_pos())), p.muted),
                            };
                            ui.horizontal(|ui| {
                                caps(ui.painter(), ui.cursor().min + vec2(0.0, 9.0), Align2::LEFT_CENTER, &format!("{} {}", tr!("quiz.question"), i + 1), p.dim);
                                ui.add_space(90.0);
                                label(ui, &msg, serif(13.5, W::Semi), col);
                            });
                            question_card(app, ui, it, true, w, false, false);
                        });
                    }
                });
            });
        });
    });
    if let Some(qs) = redo {
        let (subject, title, each) = (quiz.subject, quiz.title.clone(), quiz.each);
        let back = quiz.back;
        begin(app, subject, title, qs, each);
        if let Some(q) = app.quiz.take() {
            *quiz = Quiz { back, ..q };
        }
    }
}
