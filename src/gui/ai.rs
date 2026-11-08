//! IA na interface: a janela "Gerar com IA" (cartões ou questões objetivas
//! que chegam um a um, editáveis, e só são criados quando o usuário adiciona),
//! o download do modelo e as seções "IA local" e "IA avançada" (um .gguf
//! qualquer ou uma IA online por API) nas Configurações.

use super::theme::*;
use super::widgets::*;
use super::App;
use crate::ai::engine::{self, Event, GenRequest, Job};
use crate::ai::models::{self, ModelSpec, Phase};
use crate::ai::online::{self, Api};
use std::path::PathBuf;
use crate::ai::prompt::{self, Level, QSuggestion, Style, Suggestion};
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
    /// Últimas instruções digitadas (valem até fechar o app).
    guide: String,
    /// IA avançada: link de download digitado e a busca de modelos do serviço online.
    link: String,
    models_probe: Option<Receiver<anyhow::Result<Vec<String>>>>,
    online_models: Vec<String>,
}

/// A IA escolhida nas Configurações.
enum Engine {
    Listed(&'static ModelSpec),
    File(PathBuf),
    Online(Api),
}

impl Engine {
    fn of(app: &App) -> Engine {
        if app.cfg.ai_model == "online" {
            Engine::Online(app.cfg.ai_api.clone())
        } else if let Some(p) = app.cfg.ai_model.strip_prefix("file:") {
            Engine::File(PathBuf::from(p))
        } else {
            Engine::Listed(models::spec(&app.cfg.ai_model).unwrap_or(&models::CATALOG[1]))
        }
    }

    fn ready(&self) -> bool {
        match self {
            Engine::Listed(m) => models::is_installed(m),
            Engine::File(p) => p.is_file(),
            Engine::Online(a) => a.ready(),
        }
    }

    fn name(&self) -> String {
        match self {
            Engine::Listed(m) => m.name.to_owned(),
            Engine::File(p) => p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Engine::Online(a) => a.model.trim().to_owned(),
        }
    }
}

/// De onde vêm as sugestões.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Page,
    /// Blocos escolhidos: o do menu do bloco, ou vários (Shift/Ctrl+clique no ⠿).
    Blocks(Vec<i64>),
    /// Trecho selecionado: dentro de um bloco (`Some(id)`) ou no modo Markdown (`None`).
    Selection(Option<i64>, String),
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    NeedModel,
    /// Modelo pronto, esperando o usuário clicar em Gerar.
    Ready,
    Loading,
    Reading(f32),
    Writing(usize),
    Done { secs: f32, made: usize },
    Failed(String),
    Stopped,
}

/// O que a janela gera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Make {
    Cards,
    Questions,
}

enum Sugg {
    Card(Suggestion),
    Question(QSuggestion),
}

impl Sugg {
    fn key(&self) -> String {
        match self {
            Sugg::Card(s) => s.key(),
            Sugg::Question(q) => q.key(),
        }
    }

    fn grounded(&self) -> bool {
        match self {
            Sugg::Card(s) => s.grounded,
            Sugg::Question(q) => q.grounded,
        }
    }
}

struct Item {
    s: Sugg,
    keep: bool,
    /// Parecido com um cartão que já existe (ou com outra sugestão).
    dup: bool,
}

pub struct Review {
    page: i64,
    title: String,
    subject: i64,
    scope: Scope,
    make: Make,
    deck: i64,
    /// Banco das questões (`None` = criar um com o nome da disciplina).
    bank: Option<i64>,
    tags: String,
    count: u32,
    style: Style,
    level: Level,
    guide: String,
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
}

const COUNTS: [u32; 3] = [5, 10, 15];

/// Abre a janela para a página aberta no caderno.
pub fn open(app: &mut App, scope: Scope) {
    let Some(ed) = app.page.as_mut() else { return };
    ed.flush(&app.db);
    let (page, title, subject) = (ed.meta.id, ed.title.clone(), ed.meta.subject_id);
    open_page(app, page, title, subject, scope, Make::Cards);
}

/// Abre a janela de questões para uma página (da aba Questões).
pub fn open_questions(app: &mut App, page: i64, bank: Option<i64>) {
    if let Some(ed) = app.page.as_mut().filter(|e| e.meta.id == page) {
        ed.flush(&app.db);
    }
    let Ok(Some(meta)) = app.db.page(page) else { return };
    open_page(app, page, meta.title, meta.subject_id, Scope::Page, Make::Questions);
    if let (Some(r), Some(b)) = (app.ai.review.as_mut(), bank) {
        r.bank = Some(b);
    }
}

fn open_page(app: &mut App, page: i64, title: String, subject: i64, scope: Scope, make: Make) {
    let deck = app.default_deck().filter(|d| app.deck(*d).is_some_and(|d| d.subject_id == subject)).or_else(|| app.root_decks(subject).first().map(|d| d.id));
    let Some(deck) = deck else {
        app.toast(tr!("toast.create_deck_first"));
        return;
    };
    let bank = app.db.list_banks(subject).ok().and_then(|b| b.first().map(|b| b.id));
    let style = app.cfg.ai_style;
    let count = if COUNTS.contains(&app.cfg.ai_count) { app.cfg.ai_count } else { 10 };
    app.ai.review = Some(Review {
        page,
        title,
        subject,
        scope,
        make,
        deck,
        bank,
        tags: tr!("ai.default_tag"),
        count,
        style,
        level: app.cfg.ai_level,
        guide: app.ai.guide.clone(),
        job: None,
        status: Status::NeedModel,
        items: vec![],
        ids: vec![],
        snippets: HashMap::new(),
        existing: vec![],
        truncated: false,
    });
}

/// Começa (ou continua, com "Gerar mais") a geração.
fn start(app: &mut App, ctx: &egui::Context) {
    let engine = Engine::of(app);
    let gpu = app.cfg.ai_gpu;
    let Some(r) = app.ai.review.as_mut() else { return };
    if !engine.ready() {
        r.status = Status::NeedModel;
        return;
    }
    let all = app.db.load_blocks(r.page).unwrap_or_default();
    // Só o que foi escolhido vai para a IA: numa página grande, o trecho
    // escolhido poderia ficar de fora do limite de texto da página inteira.
    let (blocks, focus) = match &r.scope {
        Scope::Page => (all.clone(), None),
        Scope::Blocks(ids) => (all.iter().filter(|b| ids.contains(&b.id)).cloned().collect(), None),
        Scope::Selection(Some(id), text) => (all.iter().filter(|b| b.id == *id).cloned().collect(), Some(crate::rich::plain(text))),
        Scope::Selection(None, md) => (super::page::source_blocks(md, &all), None),
    };
    let source = prompt::source(&r.title, &blocks);
    r.truncated = source.truncated;
    r.snippets = all.iter().map(|b| (b.id, b.plain().chars().take(70).collect())).collect();
    let questions = r.make == Make::Questions;
    let before: Vec<String> = if questions {
        app.db.question_stems_for_page(r.page).unwrap_or_default()
    } else {
        app.db.notes_for_page(r.page).unwrap_or_default().iter().map(|n| cards::sort_field(n.kind, &n.fields)).collect()
    };
    r.existing = before.into_iter().chain(r.items.iter().map(|i| i.s.key())).filter(|s| !s.trim().is_empty()).collect();
    let count = r.count;
    let req = prompt::Request { source: &source, existing: &r.existing, count, style: r.style, focus: focus.as_deref(), level: r.level, guide: &r.guide };
    let (model, own_template, online) = match engine {
        Engine::Listed(m) => (models::path_of(m), false, None),
        Engine::File(p) => (p, true, None),
        Engine::Online(a) => (PathBuf::new(), false, Some(a)),
    };
    let request = GenRequest {
        model,
        gpu,
        own_template,
        online,
        shape: prompt::json_shape(questions, r.style),
        count,
        chat: if questions { prompt::prompt_questions(&req) } else { prompt::prompt(&req) },
        grammar: if questions { prompt::grammar_questions(count) } else { prompt::grammar(r.style, count) },
        ids: source.ids.clone(),
        notes: source.text.clone(),
        // Uma questão (enunciado, quatro alternativas e explicação) é bem maior que um cartão.
        max_tokens: count as usize * if questions { 360 } else { 220 } + 200,
        questions,
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
            Event::Card(_) | Event::Question(_) => {
                let s = match ev {
                    Event::Card(c) => Sugg::Card(c),
                    Event::Question(q) => Sugg::Question(q),
                    _ => unreachable!(),
                };
                let key = s.key();
                let dup = r.existing.iter().any(|e| prompt::similar(&key, e)) || r.items.iter().any(|i| prompt::similar(&key, &i.s.key()));
                // Repetido ou sem apoio nas notas: vem desmarcado, com o motivo.
                let keep = !dup && s.grounded();
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

/// Grava as sugestões marcadas. Devolve quantas notas (ou questões) foram criadas.
fn add_selected(app: &mut App) -> anyhow::Result<usize> {
    let Some(r) = app.ai.review.as_ref() else { return Ok(0) };
    if r.make == Make::Questions {
        return add_questions(app);
    }
    let tags = crate::db::normalize_tags(&r.tags);
    let (deck, page) = (r.deck, r.page);
    let notes: Vec<Note> = r
        .items
        .iter()
        .filter(|i| i.keep && valid(&i.s))
        .filter_map(|i| match &i.s {
            Sugg::Card(s) => Some(s),
            Sugg::Question(_) => None,
        })
        .map(|s| {
            let mut n = Note::new(s.kind);
            n.fields = s.fields();
            n.tags = tags.clone();
            n.source_page = Some(page);
            n.source_block = s.block;
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

/// Grava as questões marcadas no banco escolhido (criando um, se preciso).
fn add_questions(app: &mut App) -> anyhow::Result<usize> {
    let Some(r) = app.ai.review.as_ref() else { return Ok(0) };
    let name = app.subject(r.subject).map(|s| s.name.clone()).unwrap_or_default();
    let bank = match r.bank {
        Some(b) => b,
        None => app.db.first_bank(r.subject, &name)?,
    };
    let page = r.page;
    let mut made = 0;
    for i in r.items.iter().filter(|i| i.keep && valid(&i.s)) {
        let Sugg::Question(s) = &i.s else { continue };
        let mut q = crate::quiz::Question::new(bank);
        q.stem = s.stem.clone();
        q.options = s.options.clone();
        q.answer = s.answer;
        q.explanation = s.explanation.clone();
        q.source_page = Some(page);
        q.source_block = s.block;
        app.db.save_question(&mut q)?;
        made += 1;
    }
    if made > 0 {
        app.changed();
    }
    Ok(made)
}

/// A sugestão ainda é válida depois das edições do usuário.
fn valid(s: &Sugg) -> bool {
    match s {
        Sugg::Card(s) => match s.kind {
            NoteKind::Cloze => !cards::cloze_numbers(&s.front).is_empty(),
            _ => !s.front.trim().is_empty() && !s.back.trim().is_empty(),
        },
        Sugg::Question(q) => !q.stem.trim().is_empty() && q.options.iter().all(|o| !o.trim().is_empty()),
    }
}

// ───────────────────────────── janela ─────────────────────────────

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut r) = app.ai.review.take() else { return };
    poll(&mut r);
    // Nada começa sozinho: com o modelo pronto (inclusive logo depois de
    // baixar), a janela espera o usuário escolher as opções e clicar em Gerar.
    if r.status == Status::NeedModel && Engine::of(app).ready() {
        r.status = Status::Ready;
    }

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
                let questions = app.ai.review.as_ref().is_some_and(|r| r.make == Make::Questions);
                app.toast(if questions { tr_n!("ai.added_questions", n) } else { tr_n!("ai.added", n) });
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
        Scope::Blocks(ids) if ids.len() == 1 => tr!("ai.scope_block", title = r.title),
        Scope::Blocks(ids) => format!("{} · {}", r.title, tr_n!("ai.n_blocks", ids.len())),
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
    let questions = r.make == Make::Questions;
    if questions {
        let banks = app.db.list_banks(r.subject).unwrap_or_default();
        let subject_name = app.subject(r.subject).map(|s| s.name.clone()).unwrap_or_default();
        let cur = r.bank.and_then(|b| banks.iter().find(|x| x.id == b)).map(|b| b.name.clone()).unwrap_or_else(|| subject_name.clone());
        let color = app.subject(r.subject).map(|s| subject_color(s.color));
        select_field(&mut fu, "ai_bank", &cur, color, 240.0, 36.0, |ui| {
            for b in &banks {
                if menu_item(ui, &b.name, Some(b.id) == r.bank).clicked() {
                    r.bank = Some(b.id);
                }
            }
            if banks.is_empty() {
                menu_item(ui, &subject_name, true);
            }
        })
        .on_hover_text(tr!("quiz.bank"));
    } else {
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
    }
    let chosen = r.items.iter().filter(|i| i.keep && valid(&i.s)).count();
    fu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let label = match (questions, chosen) {
            (false, 0) => tr!("ai.add_none"),
            (false, n) => tr_n!("ai.add_n", n),
            (true, 0) => tr!("ai.add_questions"),
            (true, n) => tr_n!("ai.add_questions_n", n),
        };
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
        // Trocar entre cartões e questões recomeça a lista de sugestões.
        let mut mi = (r.make == Make::Questions) as usize;
        let (mc, mq) = (tr!("ai.make_cards"), tr!("ai.make_questions"));
        if segmented(ui, &[mc.as_str(), mq.as_str()], &mut mi, 26.0, true) {
            r.make = if mi == 1 { Make::Questions } else { Make::Cards };
            r.items.clear();
            r.status = Status::Ready;
        }
        let questions = r.make == Make::Questions;
        let mut ci = COUNTS.iter().position(|c| *c == r.count).unwrap_or(1);
        let labels: Vec<String> = COUNTS.iter().map(|c| if questions { tr_n!("ai.n_questions", *c as usize) } else { tr_n!("ai.n_cards", *c as usize) }).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        if segmented(ui, &refs, &mut ci, 26.0, false) {
            r.count = COUNTS[ci];
            app.cfg.ai_count = r.count;
            app.save_settings();
        }
        if questions {
            return;
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
            // Sem sugestões ainda, Gerar é a ação principal da janela.
            let first = r.items.is_empty();
            let label = tr!(if first { "ai.generate" } else { "ai.generate_more" });
            if button(ui, &label, if first { Btn::Primary } else { Btn::Secondary }, 32.0).clicked() {
                *regenerate = true;
            }
        }
        status_line(app, r, ui);
    });

    // ── Corpo ──
    let opts = Rect::from_min_size(pos2(rect.left(), bar.bottom()), vec2(rect.width(), 52.0));
    ui.painter().hline(opts.x_range(), opts.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut ou = ui.new_child(egui::UiBuilder::new().max_rect(opts.shrink2(vec2(24.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    ou.spacing_mut().item_spacing.x = 10.0;
    ou.add_enabled_ui(!busy && !need_model, |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let mut li = [Level::Easy, Level::Medium, Level::Hard].iter().position(|l| *l == r.level).unwrap_or(1);
        let (e, m, h) = (tr!("ai.level_easy"), tr!("ai.level_medium"), tr!("ai.level_hard"));
        if segmented(ui, &[e.as_str(), m.as_str(), h.as_str()], &mut li, 26.0, false) {
            r.level = [Level::Easy, Level::Medium, Level::Hard][li];
            app.cfg.ai_level = r.level;
            app.save_settings();
        }
        let w = ui.available_width();
        if text_field(ui, &mut r.guide, &tr!("ai.guide_hint"), w, 32.0).on_hover_text(tr!("ai.guide_tip")).changed() {
            app.ai.guide = r.guide.clone();
        }
    });

    let mid = Rect::from_min_max(pos2(rect.left(), opts.bottom()), pos2(rect.right(), foot.top()));
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
            writing_placeholder(ui, r.make);
        } else if r.items.is_empty() {
            empty_state(r, ui);
        }
    });
}

fn status_line(app: &App, r: &Review, ui: &mut Ui) {
    let p = pal();
    let (s, color) = match &r.status {
        Status::NeedModel => (tr!("ai.status_need_model"), p.muted),
        Status::Ready => (tr!("ai.status_ready"), p.muted),
        Status::Loading => (tr!("ai.status_loading", model = Engine::of(app).name()), p.muted),
        Status::Reading(f) => (tr!("ai.status_reading", pct = pct(*f, 0)), p.muted),
        Status::Writing(n) if r.make == Make::Questions => (tr_n!("ai.status_writing_questions", *n), p.accent),
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
            let kind = match &item.s {
                Sugg::Card(s) if s.kind == NoteKind::Cloze => tr!("kind.cloze"),
                Sugg::Card(_) => tr!("kind.basic"),
                Sugg::Question(_) => tr!("quiz.question"),
            };
            let block = match &item.s {
                Sugg::Card(s) => s.block,
                Sugg::Question(q) => q.block,
            };
            let kw = text_width(ui.painter(), &kind.to_uppercase(), serif(10.0, W::Semi)) + 12.0;
            let (kr, _) = ui.allocate_exact_size(vec2(kw, 20.0), Sense::hover());
            caps_sized(ui.painter(), kr.left_center(), Align2::LEFT_CENTER, &kind, 10.0, p.accent);
            if let Some(snip) = block.and_then(|b| snippets.get(&b)).filter(|s| !s.trim().is_empty()) {
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
        let (quote, grounded, missing) = match &mut item.s {
            Sugg::Card(s) if s.kind == NoteKind::Cloze => {
                edit_field(ui, ("ai_text", i), &mut s.front, &tr!("field.text"));
                // Como a frente vai aparecer na revisão.
                let front = cards::render_cloze(&s.front, cards::cloze_numbers(&s.front).first().copied().unwrap_or(1), cards::ClozeSide::Front).replace([cards::GAP_OPEN, cards::GAP_CLOSE], "");
                if cards::cloze_numbers(&s.front).is_empty() {
                    para(ui, &tr!("ai.cloze_missing"), serif(12.0, W::Italic), p.again);
                } else {
                    para(ui, &tr!("ai.cloze_preview", text = crate::rich::plain(&front)), serif(12.5, W::Regular), p.muted);
                }
                (s.quote.clone(), s.grounded, s.missing.clone())
            }
            Sugg::Card(s) => {
                edit_field(ui, ("ai_front", i), &mut s.front, &tr!("field.front"));
                edit_field(ui, ("ai_back", i), &mut s.back, &tr!("field.back"));
                (s.quote.clone(), s.grounded, s.missing.clone())
            }
            Sugg::Question(q) => {
                edit_field(ui, ("ai_stem", i), &mut q.stem, &tr!("quiz.stem_short"));
                for k in 0..q.options.len() {
                    option_field(ui, ("ai_opt", i, k), &mut q.options[k], k, &mut q.answer);
                }
                edit_field(ui, ("ai_expl", i), &mut q.explanation, &tr!("quiz.explanation_short"));
                (q.quote.clone(), q.grounded, q.missing.clone())
            }
        };
        // A prova: o trecho da nota que o modelo usou.
        if !quote.is_empty() {
            para(ui, &tr!("ai.quote", text = quote), serif(12.0, W::Italic), p.dim);
        }
        if !grounded {
            let msg = if missing.is_empty() { tr!("ai.not_in_notes") } else { tr!("ai.not_in_notes_words", words = missing.join(", ")) };
            para(ui, &msg, serif(12.0, W::Semi), p.again);
        }
        if item.dup {
            let msg = tr!(if matches!(item.s, Sugg::Question(_)) { "ai.duplicate_question" } else { "ai.duplicate" });
            para(ui, &msg, serif(12.0, W::Italic), p.hard);
        }
    });
    discard
}

/// Largura da coluna com o nome do campo ("ENUNCIADO" é o mais largo).
const LABEL_W: f32 = 80.0;

/// Campo de várias linhas com o visual `.field` e o nome do campo ao lado.
fn edit_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, name: &str) {
    let p = pal();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (lr, _) = ui.allocate_exact_size(vec2(LABEL_W, 30.0), Sense::hover());
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

/// Alternativa editável: a letra (clique marca como certa) e o texto.
fn option_field(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, k: usize, answer: &mut usize) {
    let p = pal();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // A letra fica encostada no campo, alinhado com os de cima.
        ui.add_space(LABEL_W - 24.0);
        let right = *answer == k;
        let (r, resp) = ui.allocate_exact_size(vec2(24.0, 30.0), Sense::click());
        let c = r.center();
        if right {
            ui.painter().circle_filled(c, 11.0, p.good);
        } else {
            ui.painter().circle_stroke(c, 10.5, Stroke::new(1.0, if resp.hovered() { p.good } else { p.border_strong }));
        }
        let letter = crate::gui::questions::letter(k);
        ui.painter().text(c, Align2::CENTER_CENTER, letter, serif(12.0, W::Semi), if right { p.on_accent } else { p.text2 });
        a11y(&resp, egui::accesskit::Role::RadioButton, &tr!("quiz.mark_right", letter = letter), Some(right));
        if resp.on_hover_text(tr!("quiz.mark_right", letter = letter)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *answer = k;
        }
        let w = ui.available_width();
        egui::Frame::NONE.fill(p.input).stroke(Stroke::new(1.0, if right { soft(p.good, 0.6) } else { p.border_control })).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(value).id(ui.make_persistent_id(id)).frame(egui::Frame::NONE).font(serif(14.0, W::Regular)).text_color(p.text).desired_rows(1).desired_width(w - 22.0));
        });
    });
}

fn writing_placeholder(ui: &mut Ui, make: Make) {
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
    let msg = tr!(if make == Make::Questions { "ai.writing_question" } else { "ai.writing" });
    text(ui.painter(), pos2(r.left() + 66.0, r.center().y), Align2::LEFT_CENTER, &msg, serif(13.0, W::Italic), p.muted, w - 80.0);
    ui.ctx().request_repaint();
}

fn empty_state(r: &Review, ui: &mut Ui) {
    let p = pal();
    let s = match &r.status {
        Status::Failed(_) => tr!("ai.empty_failed"),
        Status::Done { .. } if r.make == Make::Questions => tr!("ai.empty_done_questions"),
        Status::Done { .. } => tr!("ai.empty_done"),
        Status::Ready => tr!("ai.empty_ready"),
        _ => tr!("ai.empty"),
    };
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(ui.available_width().min(460.0));
        para(ui, &s, serif(14.0, W::Italic), p.muted);
    });
}

/// Sem modelo instalado: explica e oferece o download ali mesmo.
fn setup(app: &mut App, ui: &mut Ui) {
    let p = pal();
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.set_max_width(ui.available_width().min(720.0));
    label(ui, &tr!("ai.setup_title"), serif(17.0, W::Semi), p.text);
    para(ui, &tr!("ai.setup_text"), serif(13.5, W::Regular), p.text2);
    model_list(app, ui);
    para(ui, &tr!("ai.privacy"), serif(12.5, W::Italic), p.muted);
    // Modelo próprio ou IA online sem configurar: o caminho para a IA avançada.
    if !matches!(Engine::of(app), Engine::Listed(_)) {
        para(ui, &tr!("ai.setup_advanced"), serif(13.0, W::Regular), p.hard);
        if button(ui, &tr!("ai.open_advanced"), Btn::Secondary, 32.0).clicked() {
            app.ai.review = None;
            app.go(super::Screen::Settings);
        }
    }
}

// ───────────────────────────── modelos ─────────────────────────────

fn gb(bytes: u64) -> String {
    format!("{} GB", crate::i18n::fixed(bytes as f64 / 1e9, 1))
}

/// Download terminado: o modelo baixado vira o modelo em uso.
fn finish_download(app: &mut App) {
    let Some(d) = &app.ai.download else { return };
    let snap = d.snapshot();
    match snap.phase {
        Phase::Done => {
            // Da lista, pelo id; por link, pelo arquivo na pasta dos modelos.
            app.cfg.ai_model = if models::spec(&snap.model).is_some() { snap.model } else { format!("file:{}", models::models_dir().join(&snap.model).display()) };
            app.save_settings();
            app.ai.download = None;
            engine::unload();
            app.toast(tr!("ai.download_done"));
        }
        Phase::Cancelled => app.ai.download = None,
        _ => {}
    }
}

/// A lista de modelos com baixar / usar / apagar.
fn model_list(app: &mut App, ui: &mut Ui) {
    let p = pal();
    finish_download(app);
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

/// "IA avançada" nas Configurações: qualquer modelo .gguf (escolhido no disco
/// ou baixado por link) e IA online por API, com os serviços grátis em destaque.
pub fn advanced_section(app: &mut App, ui: &mut Ui) {
    let p = pal();
    finish_download(app);
    if let Some(rx) = &app.ai.models_probe {
        if let Ok(res) = rx.try_recv() {
            app.ai.models_probe = None;
            match res {
                Ok(list) if list.is_empty() => app.toast(tr!("ai.no_models_found")),
                Ok(list) => app.ai.online_models = list,
                Err(e) => app.error(e),
            }
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    let engine = Engine::of(app);
    super::settings::section(ui, &tr!("ai.advanced_title"), |ui| {
        para(ui, &tr!("ai.advanced_text"), serif(13.0, W::Regular), p.text2);
        let using = match &engine {
            Engine::Listed(m) => tr!("ai.using_listed", name = m.name),
            Engine::File(f) => tr!("ai.using_file", name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Engine::Online(a) => tr!("ai.using_online", name = a.model.trim(), service = online::provider(&a.provider).map(|s| s.name).filter(|n| !n.is_empty()).unwrap_or(a.url.trim())),
        };
        label(ui, &using, serif(13.0, W::Semi), if engine.ready() { p.good } else { p.hard });

        // ── Modelo próprio (.gguf) ──
        ui.add_space(6.0);
        caps_label(ui, &tr!("ai.own_model"));
        para(ui, &tr!("ai.own_model_text"), serif(12.5, W::Regular), p.muted);
        let mut files = models::custom_models();
        if let Engine::File(f) = &engine {
            if !files.contains(f) {
                files.insert(0, f.clone());
            }
        }
        for f in &files {
            let id = format!("file:{}", f.display());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let size = std::fs::metadata(f).map(|m| gb(m.len())).unwrap_or_default();
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let w = ui.available_width() - 110.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w.max(60.0), 30.0), Sense::hover());
                text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &format!("{name}  ·  {size}"), serif(13.5, W::Regular), p.text, r.width());
                resp.on_hover_text(f.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if app.cfg.ai_model == id {
                        label(ui, &tr!("ai.in_use"), serif(12.5, W::Semi), p.good);
                    } else if button(ui, &tr!("ai.use_model"), Btn::Secondary, 30.0).clicked() {
                        app.cfg.ai_model = id.clone();
                        app.save_settings();
                        engine::unload();
                    }
                });
            });
        }
        if files.is_empty() {
            label(ui, &tr!("ai.no_own_models"), serif(12.5, W::Italic), p.muted);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if button(ui, &tr!("ai.pick_gguf"), Btn::Secondary, 32.0).clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]).pick_file() {
                    app.cfg.ai_model = format!("file:{}", path.display());
                    app.save_settings();
                    engine::unload();
                }
            }
            if button(ui, &tr!("settings.open_folder"), Btn::Ghost, 32.0).clicked() {
                let dir = models::models_dir();
                let _ = std::fs::create_dir_all(&dir);
                super::settings::open_folder(&dir);
            }
        });
        // Baixar por link.
        let busy = app.ai.download.as_ref().is_some_and(|d| matches!(d.snapshot().phase, Phase::Downloading | Phase::Verifying));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let w = ui.available_width() - 130.0;
            text_field(ui, &mut app.ai.link, &tr!("ai.link_hint"), w.max(120.0), 32.0);
            let link = models::gguf_link(&app.ai.link);
            if button_sized(ui, &tr!("ai.download"), None, Btn::Primary, vec2(110.0, 32.0), link.is_some() && !busy).clicked() {
                if let Some((url, file)) = link {
                    let c = ui.ctx().clone();
                    app.ai.download = Some(models::Download::start_url(url, file, move || c.request_repaint()));
                }
            }
        });
        if !app.ai.link.trim().is_empty() && models::gguf_link(&app.ai.link).is_none() {
            label(ui, &tr!("ai.link_invalid"), serif(12.0, W::Italic), p.again);
        }
        if let Some(s) = app.ai.download.as_ref().map(|d| d.snapshot()).filter(|s| models::spec(&s.model).is_none()) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label_s = match &s.phase {
                    Phase::Verifying => tr!("ai.verifying"),
                    Phase::Failed(e) => tr!("ai.download_failed", error = e),
                    _ => format!("{} · {}", s.model, tr!("ai.downloading", done = gb(s.done), total = if s.total > 0 { gb(s.total) } else { "?".into() }, speed = format!("{} MB/s", crate::i18n::fixed(s.speed / 1e6, 1)))),
                };
                let w = ui.available_width() - 110.0;
                let (r, _) = ui.allocate_exact_size(vec2(w.max(60.0), 30.0), Sense::hover());
                text(ui.painter(), pos2(r.left(), r.top() + 8.0), Align2::LEFT_CENTER, &label_s, serif(11.5, W::Regular), if matches!(s.phase, Phase::Failed(_)) { p.again } else { p.muted }, r.width());
                progress(ui.painter(), Rect::from_min_size(pos2(r.left(), r.bottom() - 7.0), vec2(r.width(), 5.0)), if s.total > 0 { s.done as f32 / s.total as f32 } else { 0.0 }, p.accent);
                if matches!(s.phase, Phase::Failed(_)) {
                    if button(ui, &tr!("common.close"), Btn::Ghost, 30.0).clicked() {
                        app.ai.download = None;
                    }
                } else if button(ui, &tr!("common.cancel"), Btn::Secondary, 30.0).clicked() {
                    if let Some(d) = &app.ai.download {
                        d.cancel();
                    }
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            });
        }

        // ── IA online ──
        ui.add_space(10.0);
        caps_label(ui, &tr!("ai.online"));
        para(ui, &tr!("ai.online_text"), serif(12.5, W::Regular), p.muted);
        let mut api = app.cfg.ai_api.clone();
        let cur = online::provider(&api.provider);
        let cur_label = match cur {
            Some(s) if s.id == "custom" => tr!("ai.custom_service"),
            Some(s) if s.free => format!("{}  ·  {}", s.name, tr!("ai.free")),
            Some(s) => s.name.to_owned(),
            None => tr!("ai.pick_service"),
        };
        let w = ui.available_width();
        super::settings::row(ui, &tr!("ai.service"), |ui| {
            select_field(ui, "ai_service", &cur_label, None, 300.0, 32.0, |ui| {
                // Os grátis primeiro, marcados; depois os pagos e o personalizado.
                for s in online::PROVIDERS.iter().filter(|s| s.free).chain(online::PROVIDERS.iter().filter(|s| !s.free)) {
                    let name = if s.id == "custom" { tr!("ai.custom_service") } else { s.name.to_owned() };
                    let hint = if s.free { tr!("ai.free") } else if s.id == "custom" { String::new() } else { tr!("ai.paid") };
                    let dot = s.free.then_some(p.good);
                    if menu_item_ex(ui, &name, dot, api.provider == s.id, (!hint.is_empty()).then_some(hint.as_str())).clicked() {
                        // O modelo sugerido só entra se o atual era o sugerido do serviço anterior (ou vazio).
                        let old_default = cur.map(|c| c.model).unwrap_or("");
                        if api.model.trim().is_empty() || api.model == old_default {
                            api.model = s.model.to_owned();
                        }
                        api.provider = s.id.to_owned();
                        api.url = s.url.to_owned();
                        app.ai.online_models.clear();
                    }
                }
            });
        });
        if let Some(s) = cur.filter(|s| !s.key_url.is_empty()) {
            ui.horizontal(|ui| {
                if ui.link(egui::RichText::new(tr!("ai.get_key", service = s.name)).color(p.accent).size(12.5)).clicked() {
                    super::settings::open_folder(std::path::Path::new(s.key_url));
                }
            });
        }
        // Os três campos com a mesma largura, alinhados à direita como as outras linhas.
        let fw = (w * 0.45).min(420.0);
        super::settings::row(ui, &tr!("ai.api_url"), |ui| {
            text_field(ui, &mut api.url, "https://…/v1", fw, 32.0);
        });
        super::settings::row(ui, &tr!("ai.api_key"), |ui| {
            egui::Frame::NONE.fill(p.input).stroke(Stroke::new(1.0, p.border_control)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut api.key).password(true).frame(egui::Frame::NONE).font(mono(12.5)).hint_text(tr!("ai.api_key_hint")).desired_width(fw - 22.0));
            });
        });
        // O modelo ocupa a linha toda: os nomes (ex. "nvidia/…-reasoning:free") são longos.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (lr, _) = ui.allocate_exact_size(vec2(90.0, 32.0), Sense::hover());
            text(ui.painter(), lr.left_center(), Align2::LEFT_CENTER, &tr!("ai.api_model"), serif(13.5, W::Regular), p.text2, 90.0);
            let mw = (ui.available_width() - 140.0).max(160.0);
            let searching = app.ai.models_probe.is_some();
            if app.ai.online_models.is_empty() {
                text_field(ui, &mut api.model, &tr!("ai.api_model_hint"), mw, 32.0);
            } else {
                let list = app.ai.online_models.clone();
                let shown = if api.model.trim().is_empty() { tr!("ai.api_model_hint") } else { api.model.clone() };
                select_field(ui, "ai_online_model", &shown, None, mw, 32.0, |ui| {
                    for m in &list {
                        // Na OpenRouter, os grátis terminam em ":free".
                        let free = m.ends_with(":free");
                        if menu_item_ex(ui, m, free.then_some(p.good), *m == api.model, None).on_hover_text(m).clicked() {
                            api.model = m.clone();
                        }
                    }
                });
            }
            if button_sized(ui, &tr!(if searching { "ai.searching" } else { "ai.list_models" }), None, Btn::Ghost, vec2(130.0, 32.0), !searching && !api.url.trim().is_empty()).clicked() {
                let (tx, rx) = std::sync::mpsc::channel();
                let a = api.clone();
                let c = ui.ctx().clone();
                std::thread::spawn(move || {
                    let _ = tx.send(online::list_models(&a));
                    c.request_repaint();
                });
                app.ai.models_probe = Some(rx);
            }
        });
        if api != app.cfg.ai_api {
            app.cfg.ai_api = api.clone();
            app.save_settings();
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if app.cfg.ai_model == "online" {
                label(ui, &tr!("ai.in_use"), serif(12.5, W::Semi), p.good);
            } else if button_sized(ui, &tr!("ai.use_online"), None, Btn::Primary, vec2(button_width(ui, &tr!("ai.use_online"), None, 13.0, 14.0), 32.0), api.ready()).clicked() {
                app.cfg.ai_model = "online".into();
                app.save_settings();
            }
            if !matches!(engine, Engine::Listed(_)) && button(ui, &tr!("ai.back_to_listed"), Btn::Ghost, 32.0).clicked() {
                app.cfg.ai_model = models::DEFAULT_MODEL.into();
                app.save_settings();
            }
        });
        para(ui, &tr!("ai.online_note"), serif(12.0, W::Italic), p.muted);
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
