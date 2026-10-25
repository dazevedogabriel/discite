//! IA local na interface: a janela "Gerar cartões com IA" (sugestões que
//! chegam uma a uma, editáveis, e só viram cartões quando o usuário adiciona),
//! o download do modelo e a seção nas Configurações.

use super::theme::*;
use super::widgets::*;
use super::App;
use crate::ai::engine::{self, Event, GenRequest, Job};
use crate::ai::models::{self, ModelSpec, Phase};
use crate::ai::prompt::{self, Style, Suggestion};
use crate::cards::{self, NoteKind};
use crate::db::Note;
use crate::{tr, tr_n};
use eframe::egui::{self, Align2, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub struct AiState {
    pub download: Option<models::Download>,
    pub review: Option<Review>,
    /// Nome da GPU (procurado uma vez, numa thread, para não travar a tela).
    gpu: Option<Option<String>>,
    gpu_probe: Option<Receiver<Option<String>>>,
}

/// De onde vêm as sugestões.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Page,
    /// Um bloco (menu do bloco).
    Block(i64),
    /// Texto selecionado dentro de um bloco.
    Selection(i64, String),
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    NeedModel,
    Loading,
    Reading(f32),
    Writing(usize),
    Done { secs: f32, made: usize },
    Failed(String),
    Stopped,
}

struct Item {
    s: Suggestion,
    keep: bool,
    /// Parecido com um cartão que já existe (ou com outra sugestão).
    dup: bool,
}

pub struct Review {
    page: i64,
    title: String,
    scope: Scope,
    deck: i64,
    tags: String,
    count: u32,
    style: Style,
    job: Option<Job>,
    status: Status,
    items: Vec<Item>,
    /// Blocos de origem por número `[n]` do último pedido.
    ids: Vec<i64>,
    /// Início de cada bloco, para mostrar de onde veio o cartão.
    snippets: HashMap<i64, String>,
    /// Textos dos cartões que a página já tem.
    existing: Vec<String>,
    truncated: bool,
    /// Pouco conteúdo: quantos cartões foram pedidos de fato.
    capped: Option<u32>,
    first: bool,
}

const COUNTS: [u32; 3] = [5, 10, 15];

/// Abre a janela para a página aberta no caderno.
pub fn open(app: &mut App, scope: Scope) {
    let Some(ed) = app.page.as_mut() else { return };
    ed.flush(&app.db);
    let (page, title, subject) = (ed.meta.id, ed.title.clone(), ed.meta.subject_id);
    let deck = app.default_deck().filter(|d| app.deck(*d).is_some_and(|d| d.subject_id == subject)).or_else(|| app.root_decks(subject).first().map(|d| d.id));
    let Some(deck) = deck else {
        app.toast(tr!("toast.create_deck_first"));
        return;
    };
    let style = app.cfg.ai_style;
    let count = if COUNTS.contains(&app.cfg.ai_count) { app.cfg.ai_count } else { 10 };
    app.ai.review = Some(Review {
        page,
        title,
        scope,
        deck,
        tags: tr!("ai.default_tag"),
        count,
        style,
        job: None,
        status: Status::NeedModel,
        items: vec![],
        ids: vec![],
        snippets: HashMap::new(),
        existing: vec![],
        truncated: false,
        capped: None,
        first: true,
    });
}

fn selected_model(app: &App) -> &'static ModelSpec {
    models::spec(&app.cfg.ai_model).unwrap_or(&models::CATALOG[1])
}

/// Começa (ou continua, com "Gerar mais") a geração.
fn start(app: &mut App, ctx: &egui::Context) {
    let model = selected_model(app);
    let gpu = app.cfg.ai_gpu;
    let Some(r) = app.ai.review.as_mut() else { return };
    if !models::is_installed(model) {
        r.status = Status::NeedModel;
        return;
    }
    let blocks = app.db.load_blocks(r.page).unwrap_or_default();
    let source = prompt::source(&r.title, &blocks);
    r.truncated = source.truncated;
    r.snippets = blocks.iter().map(|b| (b.id, b.plain().chars().take(70).collect())).collect();
    r.existing = app
        .db
        .notes_for_page(r.page)
        .unwrap_or_default()
        .iter()
        .map(|n| cards::sort_field(n.kind, &n.fields))
        .chain(r.items.iter().map(|i| i.s.key()))
        .filter(|s| !s.trim().is_empty())
        .collect();
    let focus = match &r.scope {
        Scope::Page => None,
        Scope::Block(id) => blocks.iter().find(|b| b.id == *id).map(|b| b.plain()),
        Scope::Selection(_, text) => Some(crate::rich::plain(text)),
    };
    // Um bloco ou trecho tem menos fatos que a página: o teto vale para o que foi escolhido.
    let count = match &focus {
        Some(f) => r.count.min((crate::ai::ground::facts(&format!("[1] {f}")) as u32 * 2).max(2)),
        None => prompt::cap(r.count, &source),
    };
    r.capped = (count < r.count).then_some(count);
    let req = prompt::Request { source: &source, existing: &r.existing, count, style: r.style, focus: focus.as_deref() };
    let request = GenRequest {
        model: models::path_of(model),
        gpu,
        prompt: prompt::prompt(&req),
        grammar: prompt::grammar(r.style, count),
        ids: source.ids.clone(),
        notes: source.text.clone(),
        max_tokens: count as usize * 220 + 200,
    };
    r.ids = source.ids;
    let c = ctx.clone();
    r.job = Some(engine::generate(request, move || c.request_repaint()));
    r.status = Status::Loading;
}

/// Lê os eventos da geração em andamento.
fn poll(r: &mut Review) {
    let Some(job) = &r.job else { return };
    let mut finished = false;
    while let Ok(ev) = job.events.try_recv() {
        match ev {
            Event::Loading => r.status = Status::Loading,
            Event::Reading(done, total) => r.status = Status::Reading(done as f32 / total.max(1) as f32),
            Event::Writing => {
                if !matches!(r.status, Status::Writing(_)) {
                    r.status = Status::Writing(0);
                }
            }
            Event::Card(s) => {
                let key = s.key();
                let dup = r.existing.iter().any(|e| prompt::similar(&key, e)) || r.items.iter().any(|i| prompt::similar(&key, &i.s.key()));
                // Repetido ou sem apoio nas notas: vem desmarcado, com o motivo.
                let keep = !dup && s.grounded;
                r.items.push(Item { s, keep, dup });
                let n = r.items.len();
                r.status = Status::Writing(n);
            }
            Event::Finished { secs } => {
                let made = r.items.len();
                r.status = Status::Done { secs, made };
                finished = true;
            }
            Event::Failed(e) => {
                r.status = Status::Failed(e);
                finished = true;
            }
        }
    }
    if finished {
        r.job = None;
    }
}

fn running(r: &Review) -> bool {
    r.job.is_some()
}

/// Grava as sugestões marcadas. Devolve quantas notas foram criadas.
fn add_selected(app: &mut App) -> anyhow::Result<usize> {
    let Some(r) = app.ai.review.as_ref() else { return Ok(0) };
    let tags = crate::db::normalize_tags(&r.tags);
    let (deck, page) = (r.deck, r.page);
    let notes: Vec<Note> = r
        .items
        .iter()
        .filter(|i| i.keep && valid(&i.s))
        .map(|i| {
            let mut n = Note::new(i.s.kind);
            n.fields = i.s.fields();
            n.tags = tags.clone();
            n.source_page = Some(page);
            n.source_block = i.s.block;
            n
        })
        .collect();
    let mut made = 0;
    for mut n in notes {
        app.db.add_note(&mut n, deck)?;
        made += 1;
    }
    if made > 0 {
        app.cfg.last_deck = Some(deck);
        app.save_settings();
        app.changed();
    }
    Ok(made)
}

/// O cartão ainda é válido depois das edições do usuário.
fn valid(s: &Suggestion) -> bool {
    match s.kind {
        NoteKind::Cloze => !cards::cloze_numbers(&s.front).is_empty(),
        _ => !s.front.trim().is_empty() && !s.back.trim().is_empty(),
    }
}

// ───────────────────────────── janela ─────────────────────────────

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut r) = app.ai.review.take() else { return };
    poll(&mut r);
    let model_ready = models::is_installed(selected_model(app));
    // Primeira abertura (ou modelo acabou de baixar): começa sozinho.
    let auto_start = (r.first || r.status == Status::NeedModel) && model_ready && r.job.is_none() && r.items.is_empty();
    r.first = false;
    app.ai.review = Some(r);
    if auto_start {
        start(app, ctx);
    }
    let Some(mut r) = app.ai.review.take() else { return };

    let p = pal();
    let screen = ctx.content_rect();
    let size = vec2(980.0f32.min(screen.width() - 40.0), 720.0f32.min(screen.height() - 70.0));
    let mut close = false;
    let mut submit = false;
    let mut regenerate = false;
    egui::Modal::new(egui::Id::unique("ai_review"))
        .backdrop_color(p.scrim)
        .frame(egui::Frame::NONE.fill(p.window).corner_radius(16).shadow(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: egui::Color32::from_black_alpha(if p.dark { 140 } else { 46 }) }).stroke(Stroke::new(1.0, p.border_control)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            let rect = Rect::from_min_size(ui.max_rect().min, size);
            body(app, &mut r, ui, rect, &mut close, &mut submit, &mut regenerate);
        });
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close = true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        submit = true;
    }
    app.ai.review = Some(r);
    if regenerate {
        start(app, ctx);
    }
    if submit {
        match add_selected(app) {
            Ok(0) => {}
            Ok(n) => {
                app.toast(tr_n!("ai.added", n));
                close = true;
            }
            Err(e) => app.error(e),
        }
    }
    if close {
        // Soltar o Job interrompe a geração.
        app.ai.review = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn body(app: &mut App, r: &mut Review, ui: &mut Ui, rect: Rect, close: &mut bool, submit: &mut bool, regenerate: &mut bool) {
    let p = pal();
    // ── Cabeçalho ──
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let t = text(ui.painter(), pos2(head.left() + 26.0, head.center().y), Align2::LEFT_CENTER, &tr!("ai.title"), serif(19.0, W::Semi), p.text, 320.0);
    let origin = match &r.scope {
        Scope::Page => r.title.clone(),
        Scope::Block(_) => tr!("ai.scope_block", title = r.title),
        Scope::Selection(..) => tr!("ai.scope_selection", title = r.title),
    };
    let label = format!("{}  {origin}", tr!("note.source"));
    let w = text_width(ui.painter(), &label, serif(12.0, W::Regular)) + 20.0;
    let cr = Rect::from_min_size(pos2(t.right() + 14.0, head.center().y - 12.0), vec2(w.min(420.0), 24.0));
    ui.painter().rect(cr, CornerRadius::same(255), p.chip, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let g = text(ui.painter(), pos2(cr.left() + 10.0, cr.center().y), Align2::LEFT_CENTER, &tr!("note.source"), serif(12.0, W::Regular), p.muted, 60.0);
    text(ui.painter(), pos2(g.right() + 6.0, cr.center().y), Align2::LEFT_CENTER, &origin, serif(12.0, W::Regular), p.text2, cr.right() - g.right() - 14.0);
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

    // ── Rodapé: baralho, tags e adicionar ──
    let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 68.0), rect.max);
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, p.line));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(24.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    fu.spacing_mut().item_spacing.x = 10.0;
    let path = app.deck_path(r.deck);
    let color = app.subject_color_of_deck(r.deck);
    let decks = app.decks.clone();
    let mut pick = None;
    select_field(&mut fu, "ai_deck", &path, Some(color), 240.0, 36.0, |ui| {
        for s in app.subjects.clone() {
            for d in decks.iter().filter(|d| d.subject_id == s.id) {
                if menu_item_ex(ui, &app.deck_path(d.id), Some(subject_color(s.color)), d.id == r.deck, None).clicked() {
                    pick = Some(d.id);
                }
            }
        }
    })
    .on_hover_text(tr!("deck.deck"));
    if let Some(d) = pick {
        r.deck = d;
    }
    text_field(&mut fu, &mut r.tags, &tr!("ai.tags_hint"), 170.0, 36.0).on_hover_text(tr!("ai.tags_tip"));
    let chosen = r.items.iter().filter(|i| i.keep && valid(&i.s)).count();
    fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = if chosen == 0 { tr!("ai.add_none") } else { tr_n!("ai.add_n", chosen) };
        let w = button_width(ui, &label, Some("Ctrl ↵"), 13.0, 14.0);
        if button_sized(ui, &label, Some("Ctrl ↵"), Btn::Primary, vec2(w, 36.0), chosen > 0).clicked() {
            *submit = true;
        }
        if button(ui, &tr!("common.cancel"), Btn::Secondary, 36.0).clicked() {
            *close = true;
        }
    });

    // ── Barra de opções e estado ──
    let bar = Rect::from_min_size(pos2(rect.left(), head.bottom()), vec2(rect.width(), 58.0));
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(24.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bu.spacing_mut().item_spacing.x = 10.0;
    let busy = running(r);
    let need_model = r.status == Status::NeedModel;
    bu.add_enabled_ui(!busy && !need_model, |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let mut ci = COUNTS.iter().position(|c| *c == r.count).unwrap_or(1);
        let labels: Vec<String> = COUNTS.iter().map(|c| tr_n!("ai.n_cards", *c as usize)).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        if segmented(ui, &refs, &mut ci, 26.0, false) {
            r.count = COUNTS[ci];
            app.cfg.ai_count = r.count;
            app.save_settings();
        }
        let mut si = match r.style {
            Style::Auto => 0,
            Style::Basic => 1,
            Style::Cloze => 2,
        };
        let (a, b, c) = (tr!("ai.style_auto"), tr!("ai.style_basic"), tr!("ai.style_cloze"));
        if segmented(ui, &[a.as_str(), b.as_str(), c.as_str()], &mut si, 26.0, false) {
            r.style = [Style::Auto, Style::Basic, Style::Cloze][si];
            app.cfg.ai_style = r.style;
            app.save_settings();
        }
    });
    bu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if busy {
            if button(ui, &tr!("ai.stop"), Btn::Secondary, 32.0).clicked() {
                if let Some(j) = r.job.take() {
                    j.cancel();
                }
                r.status = Status::Stopped;
            }
        } else if !need_model {
            let label = tr!(if r.items.is_empty() { "ai.generate" } else { "ai.generate_more" });
            if button(ui, &label, Btn::Secondary, 32.0).clicked() {
                *regenerate = true;
            }
        }
        status_line(app, r, ui);
    });

    // ── Corpo ──
    let mid = Rect::from_min_max(pos2(rect.left(), bar.bottom()), pos2(rect.right(), foot.top()));
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(mid.shrink2(vec2(24.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    if need_model {
        egui::ScrollArea::vertical().id_salt("ai_setup").auto_shrink([false, false]).show(&mut mu, |ui| setup(app, ui));
        return;
    }
    egui::ScrollArea::vertical().id_salt("ai_items").auto_shrink([false, false]).stick_to_bottom(busy).show(&mut mu, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        if r.truncated {
            para(ui, &tr!("ai.truncated"), serif(12.5, W::Italic), p.muted);
        }
        if let Some(n) = r.capped {
            para(ui, &tr_n!("ai.capped", n as usize), serif(12.5, W::Italic), p.muted);
        }
        let snippets = r.snippets.clone();
        let mut remove = None;
        for (i, item) in r.items.iter_mut().enumerate() {
            if item_card(ui, i, item, &snippets) {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            r.items.remove(i);
        }
        if busy {
            writing_placeholder(ui);
        } else if r.items.is_empty() {
            empty_state(r, ui);
        }
    });
}

fn status_line(app: &App, r: &Review, ui: &mut Ui) {
    let p = pal();
    let (s, color) = match &r.status {
        Status::NeedModel => (tr!("ai.status_need_model"), p.muted),
        Status::Loading => (tr!("ai.status_loading", model = selected_model(app).name), p.muted),
        Status::Reading(f) => (tr!("ai.status_reading", pct = pct(*f, 0)), p.muted),
        Status::Writing(n) => (tr_n!("ai.status_writing", *n), p.accent),
        Status::Done { secs, made } => (tr_n!("ai.status_done", *made, secs = crate::i18n::fixed(*secs as f64, 1)), p.good),
        Status::Failed(e) => (tr!("ai.status_failed", error = e), p.again),
        Status::Stopped => (tr!("ai.status_stopped"), p.muted),
    };
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    text(ui.painter(), rect.right_center(), Align2::RIGHT_CENTER, &s, serif(12.5, W::Italic), color, w);
    if let Status::Failed(e) = &r.status {
        resp.on_hover_text(e);
    }
    if running(r) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    }
}

/// Um cartão sugerido. Devolve `true` se o usuário pediu para descartá-lo.
fn item_card(ui: &mut Ui, i: usize, item: &mut Item, snippets: &HashMap<i64, String>) -> bool {
    let p = pal();
    let mut discard = false;
    let stroke = if item.keep { p.accent } else { p.border };
    egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, stroke)).corner_radius(12).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        // Linha de cima: marcar, tipo, origem e descartar.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            checkbox(ui, &mut item.keep, "");
            let kind = match item.s.kind {
                NoteKind::Cloze => tr!("kind.cloze"),
                _ => tr!("kind.basic"),
            };
            let kw = text_width(ui.painter(), &kind.to_uppercase(), serif(10.0, W::Semi)) + 12.0;
            let (kr, _) = ui.allocate_exact_size(vec2(kw, 20.0), Sense::hover());
            caps_sized(ui.painter(), kr.left_center(), Align2::LEFT_CENTER, &kind, 10.0, p.accent);
            if let Some(snip) = item.s.block.and_then(|b| snippets.get(&b)).filter(|s| !s.trim().is_empty()) {
                let w = ui.available_width() - 40.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w.max(40.0), 20.0), Sense::hover());
                text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &tr!("ai.from_block", text = snip), serif(12.0, W::Italic), p.dim, r.width());
                resp.on_hover_text(snip);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                }
                close_icon(ui.painter(), r.center(), 11.0, if resp.hovered() { p.again } else { p.dim });
                a11y(&resp, egui::accesskit::Role::Button, &tr!("ai.discard"), None);
                if resp.on_hover_text(tr!("ai.discard")).clicked() {
                    discard = true;
                }
            });
        });
        match item.s.kind {
            NoteKind::Cloze => {
                edit_field(ui, ("ai_text", i), &mut item.s.front, &tr!("field.text"));
                // Como a frente vai aparecer na revisão.
                let front = cards::render_cloze(&item.s.front, cards::cloze_numbers(&item.s.front).first().copied().unwrap_or(1), cards::ClozeSide::Front).replace([cards::GAP_OPEN, cards::GAP_CLOSE], "");
                if cards::cloze_numbers(&item.s.front).is_empty() {
                    para(ui, &tr!("ai.cloze_missing"), serif(12.0, W::Italic), p.again);
                } else {
                    para(ui, &tr!("ai.cloze_preview", text = crate::rich::plain(&front)), serif(12.5, W::Regular), p.muted);
                }
            }
            _ => {
                edit_field(ui, ("ai_front", i), &mut item.s.front, &tr!("field.front"));
                edit_field(ui, ("ai_back", i), &mut item.s.back, &tr!("field.back"));
            }
        }
        // A prova: o trecho da nota que o modelo usou.
        if !item.s.quote.is_empty() {
            para(ui, &tr!("ai.quote", text = item.s.quote), serif(12.0, W::Italic), p.dim);
        }
        if !item.s.grounded {
            let msg = if item.s.missing.is_empty() { tr!("ai.not_in_notes") } else { tr!("ai.not_in_notes_words", words = item.s.missing.join(", ")) };
            para(ui, &msg, serif(12.0, W::Semi), p.again);
        }
        if item.dup {
            para(ui, &tr!("ai.duplicate"), serif(12.0, W::Italic), p.hard);
        }
    });
    discard
}

/// Campo de várias linhas com o visual `.field` e o nome do campo ao lado.
fn edit_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, name: &str) {
    let p = pal();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (lr, _) = ui.allocate_exact_size(vec2(58.0, 30.0), Sense::hover());
        caps(ui.painter(), lr.left_center(), Align2::LEFT_CENTER, name, p.dim);
        let w = ui.available_width();
        egui::Frame::NONE.fill(p.input).stroke(Stroke::new(1.0, p.border_control)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(value)
                    .id(ui.make_persistent_id(id))
                    .frame(egui::Frame::NONE)
                    .font(serif(14.5, W::Regular))
                    .text_color(p.text)
                    .desired_rows(1)
                    .desired_width(w - 22.0),
            );
        });
    });
}

fn writing_placeholder(ui: &mut Ui) {
    let p = pal();
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 64.0), Sense::hover());
    ui.painter().rect(r, CornerRadius::same(12), p.card, Stroke::new(1.0, p.border), StrokeKind::Inside);
    // Três pontos pulsando enquanto o modelo escreve.
    let t = ui.input(|i| i.time) as f32;
    for k in 0..3 {
        let a = ((t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5).clamp(0.15, 1.0);
        ui.painter().circle_filled(pos2(r.left() + 24.0 + k as f32 * 12.0, r.center().y), 3.5, p.accent.gamma_multiply(a));
    }
    text(ui.painter(), pos2(r.left() + 66.0, r.center().y), Align2::LEFT_CENTER, &tr!("ai.writing"), serif(13.0, W::Italic), p.muted, w - 80.0);
    ui.ctx().request_repaint();
}

fn empty_state(r: &Review, ui: &mut Ui) {
    let p = pal();
    let s = match &r.status {
        Status::Failed(_) => tr!("ai.empty_failed"),
        Status::Done { .. } => tr!("ai.empty_done"),
        _ => tr!("ai.empty"),
    };
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(460.0);
        para(ui, &s, serif(14.0, W::Italic), p.muted);
    });
}

/// Sem modelo instalado: explica e oferece o download ali mesmo.
fn setup(app: &mut App, ui: &mut Ui) {
    let p = pal();
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.set_max_width(720.0);
    label(ui, &tr!("ai.setup_title"), serif(17.0, W::Semi), p.text);
    para(ui, &tr!("ai.setup_text"), serif(13.5, W::Regular), p.text2);
    model_list(app, ui);
    para(ui, &tr!("ai.privacy"), serif(12.5, W::Italic), p.muted);
}

// ───────────────────────────── modelos ─────────────────────────────

fn gb(bytes: u64) -> String {
    format!("{} GB", crate::i18n::fixed(bytes as f64 / 1e9, 1))
}

/// A lista de modelos com baixar / usar / apagar.
fn model_list(app: &mut App, ui: &mut Ui) {
    let p = pal();
    // Download terminado: vira o modelo em uso.
    if let Some(d) = &app.ai.download {
        let snap = d.snapshot();
        match snap.phase {
            Phase::Done => {
                app.cfg.ai_model = snap.model.to_owned();
                app.save_settings();
                app.ai.download = None;
                app.toast(tr!("ai.download_done"));
            }
            Phase::Cancelled => app.ai.download = None,
            _ => {}
        }
    }
    for m in &models::CATALOG {
        let installed = models::is_installed(m);
        let in_use = app.cfg.ai_model == m.id;
        let downloading = app.ai.download.as_ref().map(|d| d.snapshot()).filter(|s| s.model == m.id);
        let stroke = if in_use && installed { p.accent } else { p.border };
        egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, stroke)).corner_radius(12).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.set_max_width(ui.available_width() - 220.0);
                    ui.horizontal(|ui| {
                        label(ui, m.name, serif(14.5, W::Semi), p.text);
                        if m.id == models::DEFAULT_MODEL {
                            label(ui, &tr!("ai.recommended"), serif(11.5, W::Italic), p.accent);
                        }
                    });
                    para(ui, &tr!(m.blurb), serif(12.5, W::Regular), p.muted);
                    label(ui, &tr!("ai.model_size", size = gb(m.size), ram = m.ram_gb), serif(12.0, W::Regular), p.dim);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(s) = &downloading {
                        if button(ui, &tr!("common.cancel"), Btn::Secondary, 30.0).clicked() {
                            if let Some(d) = &app.ai.download {
                                d.cancel();
                            }
                        }
                        let (r, _) = ui.allocate_exact_size(vec2(170.0, 30.0), Sense::hover());
                        let frac = s.done as f32 / s.total.max(1) as f32;
                        let label = match &s.phase {
                            Phase::Verifying => tr!("ai.verifying"),
                            Phase::Failed(e) => tr!("ai.download_failed", error = e),
                            _ => tr!("ai.downloading", done = gb(s.done), total = gb(s.total), speed = format!("{} MB/s", crate::i18n::fixed(s.speed / 1e6, 1))),
                        };
                        text(ui.painter(), pos2(r.left(), r.top() + 7.0), Align2::LEFT_CENTER, &label, serif(11.5, W::Regular), p.muted, r.width());
                        progress(ui.painter(), Rect::from_min_size(pos2(r.left(), r.bottom() - 8.0), vec2(r.width(), 5.0)), frac, p.accent);
                        if let Phase::Failed(_) = s.phase {
                            // Falhou: some da lista na próxima vez que o usuário tentar.
                        }
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                    } else if installed {
                        if button(ui, &tr!("ai.delete_model"), Btn::Ghost, 30.0).clicked() {
                            engine::unload();
                            if let Err(e) = models::remove(m) {
                                app.error(e);
                            }
                        }
                        if in_use {
                            label(ui, &tr!("ai.in_use"), serif(12.5, W::Semi), p.good);
                        } else if button(ui, &tr!("ai.use_model"), Btn::Secondary, 30.0).clicked() {
                            app.cfg.ai_model = m.id.to_owned();
                            app.save_settings();
                        }
                    } else {
                        let partial = models::partial_bytes(m);
                        let label = if partial > 0 { tr!("ai.resume_download") } else { tr!("ai.download") };
                        let busy = app.ai.download.as_ref().is_some_and(|d| matches!(d.snapshot().phase, Phase::Downloading | Phase::Verifying));
                        if button_sized(ui, &label, None, Btn::Primary, vec2(button_width(ui, &label, None, 12.5, 10.0), 30.0), !busy).clicked() {
                            let c = ui.ctx().clone();
                            app.ai.download = Some(models::Download::start(m, move || c.request_repaint()));
                        }
                    }
                });
            });
        });
    }
}

/// A seção "IA local" nas Configurações.
pub fn settings_section(app: &mut App, ui: &mut Ui) {
    let p = pal();
    super::settings::section(ui, &tr!("ai.settings_title"), |ui| {
        para(ui, &tr!("ai.settings_text"), serif(13.0, W::Regular), p.text2);
        model_list(app, ui);
        let gpu = gpu_name(app, ui.ctx());
        super::settings::row(ui, &tr!("ai.use_gpu"), |ui| {
            let mut on = app.cfg.ai_gpu;
            if toggle(ui, &mut on, &tr!("ai.use_gpu")).clicked() {
                app.cfg.ai_gpu = on;
                app.save_settings();
                engine::unload();
            }
        });
        let detail = match gpu {
            Some(Some(name)) => tr!("ai.gpu_found", name = name),
            Some(None) => tr!("ai.gpu_none"),
            None => tr!("ai.gpu_looking"),
        };
        para(ui, &detail, serif(12.0, W::Italic), p.muted);
        super::settings::row(ui, &tr!("ai.models_folder"), |ui| {
            if button(ui, &tr!("settings.open_folder"), Btn::Secondary, 32.0).clicked() {
                let dir = models::models_dir();
                let _ = std::fs::create_dir_all(&dir);
                super::settings::open_folder(&dir);
            }
        });
        para(ui, &tr!("ai.privacy"), serif(12.0, W::Italic), p.muted);
    });
}

/// `None` enquanto procura; depois `Some(nome)` ou `Some(None)`.
fn gpu_name(app: &mut App, ctx: &egui::Context) -> Option<Option<String>> {
    if app.ai.gpu.is_none() {
        match &app.ai.gpu_probe {
            None => {
                let (tx, rx) = std::sync::mpsc::channel();
                let c = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(engine::gpu_name());
                    c.request_repaint();
                });
                app.ai.gpu_probe = Some(rx);
            }
            Some(rx) => {
                if let Ok(v) = rx.try_recv() {
                    app.ai.gpu = Some(v);
                    app.ai.gpu_probe = None;
                }
            }
        }
    }
    app.ai.gpu.clone()
}
