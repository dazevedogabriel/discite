//! Assistente que responde a partir das notas (canvas "Discite — Interface":
//! Assistente e Assistente lateral). O laço com a IA fica em `ai::agent`; aqui
//! ficam a conversa na tela, o campo de mensagem, o painel do caderno e os
//! pedidos de permissão. Quem grava as mudanças é esta tela (`Action::apply`),
//! na conexão normal do app; o laço só lê. As conversas ficam no banco
//! (`assistant_chats`).

use super::richtext::{self, Look};
use super::theme::*;
use super::widgets::*;
use super::{App, Screen};
use crate::ai::actions::{Action, Undo};
use crate::ai::agent::{self, Event, Reply};
use crate::{tr, tr_n};
use eframe::egui::{self, Align2, Color32, CornerRadius, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, text::LayoutJob, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::mpsc::Sender;

/// O que o assistente pode fazer sem perguntar (Configurações › Permissões).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Perm {
    #[default]
    Ask,
    Allow,
    Block,
}

#[derive(Default)]
pub struct State {
    pub draft: String,
    /// Painel lateral aberto no caderno (Ctrl+J).
    pub panel: bool,
    /// O que aparece na conversa.
    entries: Vec<Entry>,
    /// As mensagens no formato da API, para as perguntas seguintes.
    history: Vec<Value>,
    run: Option<agent::Run>,
    tokens: u64,
    /// A conversa aberta, depois de salva pela primeira vez.
    chat_id: Option<i64>,
    /// Lista de conversas salvas (None: ler de novo do banco).
    chats: Option<Vec<(i64, String, i64)>>,
    /// Pedido de permissão esperando resposta: entrada e para onde responder.
    pending: Option<(usize, Sender<Reply>)>,
    /// "Editar antes": a cópia que o usuário está mudando.
    editing: Option<Action>,
    /// "Sempre nesta conversa": criar e editar sem perguntar.
    always: bool,
    /// Como desfazer cada mudança aplicada (só enquanto a conversa está aberta).
    undo: HashMap<usize, Undo>,
    /// Avisos para o modelo na próxima mensagem (ex.: o usuário desfez algo).
    notes: Vec<String>,
    /// Imagens anexadas à próxima mensagem (arquivos em `media/`).
    attach: Vec<String>,
    /// @ menções: texto mostrado, tipo ("page" ou "deck") e id.
    mentions: Vec<(String, &'static str, i64)>,
    mention_query: String,
}

/// O que vai para o banco, em JSON.
#[derive(Serialize, Deserialize)]
struct Saved {
    entries: Vec<Entry>,
    history: Vec<Value>,
    tokens: u64,
}

#[derive(Clone, Serialize, Deserialize)]
enum Entry {
    User(String),
    /// Imagens anexadas à pergunta que vem logo depois.
    UserImages(Vec<String>),
    Tool { name: String, arg: String, count: usize },
    Ask { action: Action, status: Status },
    Answer(String),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Status {
    Pending,
    /// `auto`: sem perguntar (permissão "livre" ou "sempre nesta conversa").
    Allowed { auto: bool },
    Denied,
    Blocked,
    Failed(String),
    Undone,
}

fn input_id() -> Id {
    Id::unique("assistant_input")
}

/// O campo de mensagem tem o foco: a página não deve tomar Ctrl+Z nem a colagem.
pub fn typing(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.has_focus(input_id()))
}

/// Recebe o que o assistente mandou desde o último quadro.
pub fn poll(app: &mut App) {
    let Some(run) = &app.assistant.run else { return };
    let events: Vec<Event> = run.events.try_iter().collect();
    for e in events {
        let st = &mut app.assistant;
        match e {
            Event::Tool { name, arg, count } => st.entries.push(Entry::Tool { name, arg, count }),
            Event::Usage(n) => st.tokens += n,
            Event::Ask { action, reply } => ask(app, action, reply),
            Event::Answer { text, messages } => {
                st.history.extend(messages);
                st.entries.push(Entry::Answer(text));
                st.run = None;
                save(app);
            }
            Event::Failed(e) => {
                st.entries.push(Entry::Error(e));
                st.run = None;
                st.pending = None;
                save(app);
            }
        }
    }
}

/// Uma mudança proposta: recusa, aplica ou pergunta, conforme as permissões.
fn ask(app: &mut App, action: Action, reply: Sender<Reply>) {
    let perm = if action.deletes() { app.cfg.assistant_delete } else { app.cfg.assistant_write };
    let auto = perm == Perm::Allow || (!action.deletes() && app.assistant.always);
    let st = &mut app.assistant;
    let i = st.entries.len();
    if perm == Perm::Block {
        st.entries.push(Entry::Ask { action, status: Status::Blocked });
        let _ = reply.send(Reply::Refused("The user's permission settings block this kind of change. Tell the user they can change it under Permissions.".into()));
    } else if auto {
        st.entries.push(Entry::Ask { action, status: Status::Pending });
        apply(app, i, reply, true);
    } else {
        st.entries.push(Entry::Ask { action, status: Status::Pending });
        st.pending = Some((i, reply));
        st.editing = None;
    }
    save(app);
}

fn set_status(app: &mut App, i: usize, s: Status) {
    if let Some(Entry::Ask { status, .. }) = app.assistant.entries.get_mut(i) {
        *status = s;
    }
}

/// Grava a mudança da entrada `i` e conta ao modelo como foi.
fn apply(app: &mut App, i: usize, reply: Sender<Reply>, auto: bool) {
    let Some(Entry::Ask { action, .. }) = app.assistant.entries.get(i) else { return };
    match action.apply(&app.db) {
        Ok((msg, undo)) => {
            set_status(app, i, Status::Allowed { auto });
            app.assistant.undo.insert(i, undo);
            let _ = reply.send(Reply::Done(msg));
            app.changed();
        }
        Err(e) => {
            set_status(app, i, Status::Failed(format!("{e:#}")));
            let _ = reply.send(Reply::Refused(format!("Saving failed: {e:#}")));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Decision {
    Allow,
    Always,
    Deny,
}

fn decide(app: &mut App, d: Decision) {
    let Some((i, reply)) = app.assistant.pending.take() else { return };
    // A versão editada ("Editar antes") toma o lugar da proposta.
    if let (Some(edited), Some(Entry::Ask { action, .. })) = (app.assistant.editing.take(), app.assistant.entries.get_mut(i)) {
        *action = edited;
    }
    match d {
        Decision::Deny => {
            set_status(app, i, Status::Denied);
            let _ = reply.send(Reply::Refused("The user denied this change. Do not try again; ask what they want different.".into()));
        }
        Decision::Allow | Decision::Always => {
            if d == Decision::Always {
                app.assistant.always = true;
            }
            apply(app, i, reply, false);
        }
    }
    save(app);
}

fn undo(app: &mut App, i: usize) {
    let Some(u) = app.assistant.undo.remove(&i) else { return };
    match u.revert(&app.db) {
        Ok(()) => {
            if let Some(Entry::Ask { action, .. }) = app.assistant.entries.get(i) {
                let what = english(action);
                app.assistant.notes.push(format!("(The user undid an earlier change: {what}.)"));
            }
            set_status(app, i, Status::Undone);
            app.changed();
            save(app);
        }
        Err(e) => app.error(e),
    }
}

/// A mudança em poucas palavras, para avisar o modelo.
fn english(a: &Action) -> String {
    match a {
        Action::CreateCards { cards, deck, .. } => format!("created {} cards in \"{deck}\"", cards.len()),
        Action::EditCard { note_id, .. } => format!("edited note {note_id}"),
        Action::DeleteCards { note_ids, .. } => format!("deleted notes {note_ids:?}"),
        Action::CreateQuestions { questions, bank, .. } => format!("created {} questions in \"{bank}\"", questions.len()),
        Action::EditQuestion { id, .. } => format!("edited question {id}"),
        Action::DeleteQuestions { ids, .. } => format!("deleted questions {ids:?}"),
    }
}

/// Grava a conversa aberta (a primeira gravação cria a conversa).
fn save(app: &mut App) {
    if app.assistant.entries.is_empty() {
        return;
    }
    let title = chat_title(app);
    let st = &mut app.assistant;
    let id = *st.chat_id.get_or_insert_with(crate::db::new_id);
    let data = serde_json::to_string(&Saved { entries: st.entries.clone(), history: st.history.clone(), tokens: st.tokens }).unwrap_or_default();
    st.chats = None;
    if let Err(e) = app.db.save_chat(id, &title, &data) {
        app.error(e);
    }
}

/// Esquece o que é da conversa aberta (resposta em andamento, permissões, desfazer).
fn reset(st: &mut State) {
    st.run = None;
    st.pending = None;
    st.editing = None;
    st.always = false;
    st.undo.clear();
    st.notes.clear();
}

fn open_chat(app: &mut App, id: i64) {
    let saved = app.db.chat_data(id).ok().flatten().and_then(|d| serde_json::from_str::<Saved>(&d).ok());
    let Some(saved) = saved else {
        app.toast(tr!("assistant.open_failed"));
        return;
    };
    let st = &mut app.assistant;
    reset(st);
    st.entries = saved.entries;
    st.history = saved.history;
    st.tokens = saved.tokens;
    st.chat_id = Some(id);
}

fn delete_chat(app: &mut App, id: i64) {
    if let Err(e) = app.db.delete_chat(id) {
        return app.error(e);
    }
    if app.assistant.chat_id == Some(id) {
        new_chat(app);
    }
    app.assistant.chats = None;
}

fn new_chat(app: &mut App) {
    let st = &mut app.assistant;
    reset(st);
    st.entries.clear();
    st.history.clear();
    st.tokens = 0;
    st.chat_id = None;
}

fn send(app: &mut App, ctx: &egui::Context, from_panel: bool) {
    let question = app.assistant.draft.trim().to_owned();
    if question.is_empty() || app.assistant.run.is_some() {
        return;
    }
    if !(app.cfg.ai_model == "online" && app.cfg.ai_api.ready()) {
        app.toast(tr!("assistant.need_online"));
        return;
    }
    // No painel, a página aberta é o contexto; salva antes para o assistente ler a versão atual.
    let context = if from_panel {
        app.page.as_mut().map(|pe| {
            pe.flush(&app.db);
            agent::Context { page_id: pe.meta.id, title: pe.meta.title.clone() }
        })
    } else {
        None
    };
    let st = &mut app.assistant;
    // O que o modelo recebe: avisos pendentes, a pergunta e as @ menções com os ids.
    let mut text = question.clone();
    let refs: Vec<String> = st.mentions.iter().filter(|(t, ..)| question.contains(&format!("@{t}"))).map(|(t, kind, id)| format!("{kind} \"{t}\" ({kind}_id={id})")).collect();
    if !refs.is_empty() {
        text.push_str(&format!("\n\n(Mentioned: {})", refs.join("; ")));
    }
    if !st.notes.is_empty() {
        text = format!("{}\n\n{text}", st.notes.join("\n"));
    }
    let user = if st.attach.is_empty() {
        json!(text)
    } else {
        let mut parts = vec![json!({"type": "text", "text": text})];
        parts.extend(st.attach.iter().map(|a| json!({"type": "image_url", "image_url": {"url": format!("media:{a}")}})));
        st.entries.push(Entry::UserImages(st.attach.clone()));
        json!(parts)
    };
    st.draft.clear();
    st.notes.clear();
    st.attach.clear();
    st.mentions.clear();
    st.entries.push(Entry::User(question));
    let c = ctx.clone();
    let req = agent::Request { api: app.cfg.ai_api.clone(), root: app.db.root.clone(), history: st.history.clone(), user, context, clock: app.clock };
    st.run = Some(agent::ask(req, move || c.request_repaint()));
    save(app);
}

fn composer_height(app: &App, compact: bool) -> f32 {
    (if compact { 96.0 } else { 112.0 }) + if app.assistant.attach.is_empty() { 0.0 } else { 40.0 }
}

/// Tela inteira: conversas à esquerda, conversa no meio.
pub fn ui(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let full = ui.max_rect();
    // Janela estreita: a lista de conversas sai.
    let list_w = if full.width() < 760.0 { 0.0 } else { 220.0 };
    if list_w > 0.0 {
        let list = Rect::from_min_size(full.min, vec2(list_w, full.height()));
        ui.painter().vline(list.right() - 0.5, list.y_range(), Stroke::new(1.0, p.line));
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink2(vec2(14.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Min)));
        conversations(app, &mut lu);
    }
    let main = Rect::from_min_max(pos2(full.left() + list_w, full.top()), full.max);

    // Barra do topo: título, contexto, modelo e gasto.
    let bar = Rect::from_min_size(main.min, vec2(main.width(), 56.0));
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.line));
    let inner = bar.shrink2(vec2(22.0, 0.0));
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)));
    ru.spacing_mut().item_spacing.x = 10.0;
    label(&mut ru, &tr!("assistant.tokens", n = thousands(app.assistant.tokens as i64)), serif(12.0, W::Regular), p.muted);
    model_chip(app, &mut ru, 32.0);
    let tw = (inner.width() - (inner.right() - ru.min_rect().left()) - 16.0).max(80.0);
    text(ui.painter(), pos2(inner.left(), bar.center().y - 9.0), Align2::LEFT_CENTER, &chat_title(app), serif(16.0, W::Semi), p.text, tw);
    text(ui.painter(), pos2(inner.left(), bar.center().y + 11.0), Align2::LEFT_CENTER, &tr!("assistant.context_all"), serif(12.0, W::Regular), p.muted, tw);

    // Conversa e campo de mensagem numa coluna de até 680 px.
    let col_w = (main.width() - 48.0).min(680.0);
    let col_x = main.center().x - col_w / 2.0;
    let composer_h = composer_height(app, false);
    let body = Rect::from_min_max(pos2(col_x, bar.bottom() + 12.0), pos2(col_x + col_w, main.bottom() - composer_h - 26.0));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    transcript(app, &mut bu, false);
    let comp = Rect::from_min_max(pos2(col_x, main.bottom() - composer_h - 18.0), pos2(col_x + col_w, main.bottom() - 18.0));
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(comp).layout(egui::Layout::top_down(egui::Align::Min)));
    composer(app, &mut cu, false);
}

/// Painel lateral do caderno.
pub fn panel(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = pal();
    ui.painter().rect_filled(rect, 0, p.sidebar);
    ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, p.line));
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 48.0));
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(10.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    hu.spacing_mut().item_spacing.x = 2.0;
    if icon_button(&mut hu, "×", &tr!("assistant.close_panel"), 30.0).clicked() {
        app.assistant.panel = false;
    }
    if icon_button(&mut hu, "↗", &tr!("assistant.open_page"), 30.0).clicked() {
        app.assistant.panel = false;
        app.go(Screen::Assistant);
    }
    if !app.assistant.entries.is_empty() && icon_button(&mut hu, "+", &tr!("assistant.new_chat"), 30.0).clicked() {
        new_chat(app);
    }
    let g = text(ui.painter(), pos2(head.left() + 16.0, head.center().y), Align2::LEFT_CENTER, &tr!("assistant.title"), serif(15.0, W::Semi), p.text, 140.0);
    caps(ui.painter(), pos2(g.right() + 8.0, head.center().y + 1.0), Align2::LEFT_CENTER, &tr!("assistant.this_page"), p.dim);

    let composer_h = composer_height(app, true);
    let body = Rect::from_min_max(pos2(rect.left() + 16.0, head.bottom() + 8.0), pos2(rect.right() - 10.0, rect.bottom() - composer_h - 20.0));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    transcript(app, &mut bu, true);
    let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - composer_h - 12.0), rect.max);
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, p.line));
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(12.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    composer(app, &mut cu, true);
}

/// A primeira pergunta dá nome à conversa.
fn chat_title(app: &App) -> String {
    app.assistant
        .entries
        .iter()
        .find_map(|e| match e {
            Entry::User(q) => Some(q.lines().next().unwrap_or_default().chars().take(60).collect()),
            _ => None,
        })
        .unwrap_or_else(|| tr!("assistant.new_chat"))
}

fn perm_word(p: Perm, delete: bool) -> String {
    tr!(match (p, delete) {
        (Perm::Ask, true) => "assistant.mode_ask_always",
        (Perm::Ask, false) => "assistant.mode_ask",
        (Perm::Allow, _) => "assistant.mode_allow",
        (Perm::Block, _) => "assistant.mode_block",
    })
}

/// Menu das permissões (o "editar" da lista e o "Pedir permissão ▾" do campo).
fn perm_menu(app: &mut App, ui: &mut Ui) {
    ui.set_min_width(230.0);
    caps_label(ui, &tr!("assistant.perm_write_title"));
    for p in [Perm::Ask, Perm::Allow, Perm::Block] {
        if menu_item(ui, &perm_word(p, false), app.cfg.assistant_write == p).clicked() {
            app.cfg.assistant_write = p;
            app.save_settings();
        }
    }
    menu_sep(ui);
    caps_label(ui, &tr!("assistant.perm_delete_title"));
    for p in [Perm::Ask, Perm::Block] {
        if menu_item(ui, &perm_word(p, true), app.cfg.assistant_delete == p).clicked() {
            app.cfg.assistant_delete = p;
            app.save_settings();
        }
    }
}

fn conversations(app: &mut App, ui: &mut Ui) {
    let p = pal();
    ui.spacing_mut().item_spacing.y = 6.0;
    let w = ui.available_width();
    if button_sized(ui, &tr!("assistant.new_chat_button"), None, Btn::Secondary, vec2(w, 34.0), true).clicked() {
        new_chat(app);
    }
    ui.add_space(8.0);
    caps_label(ui, &tr!("assistant.chats"));
    if app.assistant.chats.is_none() {
        app.assistant.chats = Some(app.db.list_chats().unwrap_or_default());
    }
    let chats = app.assistant.chats.clone().unwrap_or_default();
    if chats.is_empty() {
        para(ui, &tr!("assistant.no_chats"), serif(12.5, W::Italic), p.muted);
    }
    // A lista rola; o pé com as permissões fica fixo.
    let list_h = (ui.max_rect().bottom() - 100.0 - ui.cursor().top()).max(40.0);
    egui::ScrollArea::vertical().id_salt("assistant_chats").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (id, title, at) in chats {
            let on = app.assistant.chat_id == Some(id);
            let (r, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
            if on || resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(7), if on { p.selected } else { p.hover });
            }
            text(ui.painter(), pos2(r.left() + 10.0, r.top() + 14.0), Align2::LEFT_CENTER, &title, serif(13.0, if on { W::Semi } else { W::Regular }), if on { p.text } else { p.text2 }, r.width() - 20.0);
            text(ui.painter(), pos2(r.left() + 10.0, r.top() + 31.0), Align2::LEFT_CENTER, &ago(at), serif(11.0, W::Regular), p.muted, r.width() - 20.0);
            a11y(&resp, egui::accesskit::Role::Link, &title, Some(on));
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && !on {
                open_chat(app, id);
            }
            resp.context_menu(|ui| {
                ui.set_max_width(200.0);
                if menu_item(ui, &tr!("assistant.delete_chat"), false).clicked() {
                    delete_chat(app, id);
                }
            });
        }
    });

    // Permissões, no pé da coluna.
    let bottom = ui.max_rect().bottom();
    let r = Rect::from_min_max(pos2(ui.max_rect().left() + 6.0, bottom - 92.0), pos2(ui.max_rect().right() - 6.0, bottom));
    ui.painter().hline(ui.max_rect().x_range(), r.top(), Stroke::new(1.0, p.line));
    let font = serif(12.0, W::Regular);
    let y = r.top() + 18.0;
    text(ui.painter(), pos2(r.left(), y), Align2::LEFT_CENTER, &tr!("assistant.permissions"), font.clone(), p.muted, r.width());
    let er = text(ui.painter(), pos2(r.right(), y), Align2::RIGHT_CENTER, &tr!("assistant.edit"), font.clone(), p.accent, 80.0);
    let edit = ui.interact(er.expand(4.0), ui.scope_id().with("perm_edit"), Sense::click());
    a11y(&edit, egui::accesskit::Role::Button, &tr!("assistant.edit"), None);
    let edit = edit.on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&edit).width(240.0).frame(popup_frame()).show(|ui| perm_menu(app, ui));
    let (w_perm, d_perm) = (app.cfg.assistant_write, app.cfg.assistant_delete);
    let color = |p2: Perm| match p2 {
        Perm::Allow => p.good,
        Perm::Ask => p.hard,
        Perm::Block => p.again,
    };
    let lines = [
        (tr!("assistant.perm_read"), p.good),
        (tr!("assistant.perm_write_line", mode = perm_word(w_perm, false)), color(w_perm)),
        (tr!("assistant.perm_delete_line", mode = perm_word(d_perm, true)), color(d_perm)),
    ];
    for (i, (line, c)) in lines.into_iter().enumerate() {
        let y = y + 21.0 * (i + 1) as f32;
        ui.painter().circle_filled(pos2(r.left() + 3.0, y), 3.0, c);
        text(ui.painter(), pos2(r.left() + 12.0, y), Align2::LEFT_CENTER, &line, font.clone(), p.muted, r.width() - 12.0);
    }
}

/// Lacunas `{{c1::texto}}` como `[texto]`, para ler na proposta.
fn show_cloze(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{c") {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let (Some(colon), Some(end)) = (after.find("::"), after.find("}}")) else {
            out.push_str(after);
            return out;
        };
        let inner = &after[(colon + 2).min(end)..end];
        out.push('[');
        out.push_str(inner.split("::").next().unwrap_or(inner));
        out.push(']');
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Título curto (verbo) e o que a proposta muda, em uma linha.
fn action_texts(a: &Action) -> (String, String, String) {
    match a {
        Action::CreateCards { deck, cards, .. } => (tr!("assistant.act_create_cards"), tr_n!("assistant.ask_create_cards", cards.len(), deck = deck), tr_n!("assistant.done_create_cards", cards.len(), deck = deck)),
        Action::EditCard { .. } => (tr!("assistant.act_edit_card"), tr!("assistant.ask_edit_card"), tr!("assistant.done_edit_card")),
        Action::DeleteCards { note_ids, .. } => (tr!("assistant.act_delete_cards"), tr_n!("assistant.ask_delete_cards", note_ids.len()), tr_n!("assistant.done_delete_cards", note_ids.len())),
        Action::CreateQuestions { bank, questions, .. } => (tr!("assistant.act_create_questions"), tr_n!("assistant.ask_create_questions", questions.len(), bank = bank), tr_n!("assistant.done_create_questions", questions.len(), bank = bank)),
        Action::EditQuestion { .. } => (tr!("assistant.act_edit_question"), tr!("assistant.ask_edit_question"), tr!("assistant.done_edit_question")),
        Action::DeleteQuestions { ids, .. } => (tr!("assistant.act_delete_questions"), tr_n!("assistant.ask_delete_questions", ids.len()), tr_n!("assistant.done_delete_questions", ids.len())),
    }
}

/// As linhas da proposta, com uma marca para o que veio de imagem (sem conferência).
fn action_items(a: &Action) -> Vec<(String, bool)> {
    let letter = |i: usize| (b'A' + i as u8) as char;
    match a {
        Action::CreateCards { cards, .. } => cards.iter().map(|c| (if c.cloze { show_cloze(&c.front) } else { format!("{}  →  {}", c.front, c.back) }, c.from_image)).collect(),
        Action::EditCard { cloze, before, after, .. } => {
            let mut v = vec![];
            for k in 0..2 {
                if before[k] != after[k] {
                    let (b, a) = if *cloze { (show_cloze(&before[k]), show_cloze(&after[k])) } else { (before[k].clone(), after[k].clone()) };
                    v.push((format!("{b}  ⟶  {a}"), false));
                }
            }
            v
        }
        Action::DeleteCards { previews, .. } | Action::DeleteQuestions { previews, .. } => previews.iter().map(|p| (p.clone(), false)).collect(),
        Action::CreateQuestions { questions, .. } => questions
            .iter()
            .map(|q| {
                let opts: Vec<String> = q.options.iter().enumerate().map(|(i, o)| format!("{}{}) {o}", if i == q.answer { "✓ " } else { "" }, letter(i))).collect();
                (format!("{}\n{}", q.stem, opts.join("   ")), q.from_image)
            })
            .collect(),
        Action::EditQuestion { before, after, .. } => {
            let mut v = vec![];
            if before.stem != after.stem {
                v.push((format!("{}  ⟶  {}", before.stem, after.stem), false));
            }
            for i in 0..4 {
                if before.options[i] != after.options[i] {
                    v.push((format!("{}) {}  ⟶  {}", letter(i), before.options[i], after.options[i]), false));
                }
            }
            if before.answer != after.answer {
                v.push((format!("✓ {}  ⟶  ✓ {}", letter(before.answer), letter(after.answer)), false));
            }
            if before.explanation != after.explanation {
                v.push((format!("{}  ⟶  {}", before.explanation, after.explanation), false));
            }
            v
        }
    }
}

/// Os textos que "Editar antes" deixa mudar.
fn editable(a: &mut Action) -> Vec<(String, &mut String)> {
    let mut v: Vec<(String, &mut String)> = vec![];
    match a {
        Action::CreateCards { cards, .. } => {
            for (i, c) in cards.iter_mut().enumerate() {
                if c.cloze {
                    v.push((format!("{} · {}", i + 1, tr!("field.text")), &mut c.front));
                } else {
                    v.push((format!("{} · {}", i + 1, tr!("field.front")), &mut c.front));
                    v.push((format!("{} · {}", i + 1, tr!("field.back")), &mut c.back));
                }
            }
        }
        Action::EditCard { cloze, after, .. } => {
            let [a0, a1] = after;
            v.push((tr!(if *cloze { "field.text" } else { "field.front" }), a0));
            v.push((tr!(if *cloze { "field.extra" } else { "field.back" }), a1));
        }
        Action::CreateQuestions { questions, .. } => {
            for (i, q) in questions.iter_mut().enumerate() {
                v.push((format!("{} · {}", i + 1, tr!("assistant.stem")), &mut q.stem));
                for (k, o) in q.options.iter_mut().enumerate() {
                    v.push((format!("{} · {})", i + 1, (b'A' + k as u8) as char), o));
                }
            }
        }
        Action::EditQuestion { after, .. } => {
            v.push((tr!("assistant.stem"), &mut after.stem));
            for (k, o) in after.options.iter_mut().enumerate() {
                v.push((format!("{})", (b'A' + k as u8) as char), o));
            }
            v.push((tr!("assistant.explanation"), &mut after.explanation));
        }
        Action::DeleteCards { .. } | Action::DeleteQuestions { .. } => {}
    }
    v
}

/// Pedido de permissão (pendente) ou o resultado dele (como uma linha de ferramenta).
fn ask_entry(app: &mut App, ui: &mut Ui, i: usize, w: f32, compact: bool) {
    let p = pal();
    let small = if compact { 12.5 } else { 13.0 };
    let Some(Entry::Ask { action, status }) = app.assistant.entries.get(i).cloned() else { return };
    let (verb, question, done) = action_texts(&action);
    let live = app.assistant.pending.as_ref().is_some_and(|(k, _)| *k == i);
    if status == Status::Pending && live {
        let mut decision = None;
        egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.hard)).corner_radius(CornerRadius::same(12)).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(w - 26.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            if !compact {
                caps_label(ui, &tr!("assistant.permission"));
            }
            para(ui, &question, serif(small + 1.5, W::Semi), p.text);
            let inner_w = ui.available_width();
            let editing = app.assistant.editing.is_some();
            if editing {
                let mut edited = app.assistant.editing.take();
                if let Some(a) = edited.as_mut() {
                    for (name, value) in editable(a) {
                        label(ui, &name, serif(11.5, W::Regular), p.muted);
                        text_field(ui, value, "", inner_w, 30.0);
                    }
                }
                app.assistant.editing = edited;
            } else {
                for (k, (line, from_image)) in action_items(&action).into_iter().enumerate() {
                    let mut job = LayoutJob::simple(format!("{}. {line}", k + 1), serif(small, W::Regular), p.text2, inner_w);
                    if from_image {
                        job.append(&format!("  · {}", tr!("assistant.from_image")), 0.0, egui::TextFormat { font_id: serif(small - 1.0, W::Italic), color: p.hard, ..Default::default() });
                    }
                    ui.add(egui::Label::new(job).selectable(true));
                }
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if button_kbd(ui, &tr!("assistant.allow"), Some("1"), Btn::Primary, 30.0).clicked() {
                    decision = Some(Decision::Allow);
                }
                if !action.deletes() && button_kbd(ui, &tr!("assistant.always"), Some("2"), Btn::Secondary, 30.0).clicked() {
                    decision = Some(Decision::Always);
                }
                if button_kbd(ui, &tr!("assistant.deny"), Some("3"), Btn::Danger, 30.0).clicked() {
                    decision = Some(Decision::Deny);
                }
                if !action.deletes() {
                    let label = tr!(if editing { "assistant.edit_done" } else { "assistant.edit_first" });
                    if button(ui, &label, Btn::Ghost, 30.0).clicked() {
                        app.assistant.editing = if editing { app.assistant.editing.take() } else { Some(action.clone()) };
                        if editing {
                            // "Concluir": a edição vira a proposta mostrada.
                            if let (Some(e), Some(Entry::Ask { action: a, .. })) = (app.assistant.editing.take(), app.assistant.entries.get_mut(i)) {
                                *a = e;
                            }
                        }
                    }
                }
            });
        });
        // 1, 2, 3 respondem, quando não se está digitando (o campo vazio solta o foco).
        let ctx = ui.ctx().clone();
        if typing(&ctx) && app.assistant.draft.trim().is_empty() {
            ctx.memory_mut(|m| m.surrender_focus(input_id()));
        }
        if decision.is_none() && !ctx.egui_wants_keyboard_input() {
            decision = ctx.input_mut(|inp| {
                if inp.consume_key(Modifiers::NONE, Key::Num1) {
                    Some(Decision::Allow)
                } else if !action.deletes() && inp.consume_key(Modifiers::NONE, Key::Num2) {
                    Some(Decision::Always)
                } else if inp.consume_key(Modifiers::NONE, Key::Num3) {
                    Some(Decision::Deny)
                } else {
                    None
                }
            });
        }
        if let Some(d) = decision {
            decide(app, d);
        }
        return;
    }
    let (word, dot) = match &status {
        Status::Pending => (tr!("assistant.st_unanswered"), p.dim),
        Status::Allowed { auto: true } => (tr!("assistant.st_auto"), p.good),
        Status::Allowed { auto: false } => (tr!("assistant.st_allowed"), p.good),
        Status::Denied => (tr!("assistant.st_denied"), p.again),
        Status::Blocked => (tr!("assistant.st_blocked"), p.again),
        Status::Failed(e) => (tr!("assistant.failed", error = e), p.again),
        Status::Undone => (tr!("assistant.st_undone"), p.dim),
    };
    let (r, _) = ui.allocate_exact_size(vec2(w, small + 4.0), Sense::hover());
    ui.painter().circle_filled(pos2(r.left() + 3.5, r.center().y), 3.5, dot);
    let t = text(ui.painter(), pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &verb, serif(small, W::Semi), p.text, w * 0.5);
    text(ui.painter(), pos2(t.right() + 8.0, r.center().y), Align2::LEFT_CENTER, &word, serif(small, W::Regular), if dot == p.again { p.again } else { p.muted }, (r.right() - t.right() - 8.0).max(10.0));
    ui.add_space(-6.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, small + 4.0), Sense::hover());
    let summary = if matches!(status, Status::Allowed { .. } | Status::Undone) { done } else { question };
    text(ui.painter(), pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &format!("⎿ {summary}"), serif(small, W::Regular), p.muted, w - 14.0);
    if app.assistant.undo.contains_key(&i) {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if button(ui, &tr!("assistant.undo"), Btn::Ghost, 26.0).clicked() {
                undo(app, i);
            }
        });
    }
}

/// A conversa, de cima para baixo, rolando sozinha até o fim.
fn transcript(app: &mut App, ui: &mut Ui, compact: bool) {
    let p = pal();
    let size = if compact { 13.5 } else { 15.0 };
    let small = if compact { 12.5 } else { 13.0 };
    egui::ScrollArea::vertical().id_salt(("assistant_transcript", compact)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.add_space(if compact { 8.0 } else { 10.0 });
        if app.assistant.entries.is_empty() {
            ui.add_space(if compact { 0.0 } else { 30.0 });
            label(ui, &tr!("assistant.empty_title"), serif(if compact { 15.0 } else { 20.0 }, W::Semi), p.text);
            para(ui, &tr!("assistant.empty_text"), serif(if compact { 13.0 } else { 14.5 }, W::Regular), p.text2);
            return;
        }
        let w = ui.available_width() - if compact { 6.0 } else { 0.0 };
        let media = app.db.media_dir();
        for i in 0..app.assistant.entries.len() {
            match app.assistant.entries[i].clone() {
                Entry::User(q) => {
                    // Balão à direita, com no máximo 3/4 da largura.
                    let max = (w * 0.78).max(120.0);
                    let g = ui.painter().layout(q.clone(), serif(size, W::Regular), p.text, max - 24.0);
                    let bw = g.size().x + 24.0;
                    let (row, _) = ui.allocate_exact_size(vec2(w, g.size().y + 20.0), Sense::hover());
                    let bubble = Rect::from_min_max(pos2(row.right() - bw, row.top()), row.max);
                    ui.painter().rect(bubble, CornerRadius { nw: 14, ne: 14, sw: 14, se: 4 }, p.chip, Stroke::new(1.0, p.border), StrokeKind::Inside);
                    let job = LayoutJob::simple(q, serif(size, W::Regular), p.text, max - 24.0);
                    // `put` centraliza; num filho de cima para baixo o texto fica à esquerda.
                    ui.scope_builder(egui::UiBuilder::new().max_rect(bubble.shrink2(vec2(12.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                        ui.add(egui::Label::new(job).selectable(true));
                    });
                }
                Entry::UserImages(files) => {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        for f in files.iter().rev() {
                            richtext::image(ui, &crate::media::uri(&media, f), 160.0, 100.0, 10, Sense::hover());
                        }
                    });
                }
                Entry::Tool { name, arg, count } => {
                    let (title, summary) = match name.as_str() {
                        "search_notes" => (tr!("assistant.tool_search"), tr_n!("assistant.n_pages", count)),
                        "read_page" => (tr!("assistant.tool_read"), tr_n!("ai.n_blocks", count)),
                        "list_decks_and_banks" => (tr!("assistant.tool_list"), tr_n!("assistant.n_decks", count)),
                        "search_cards" => (tr!("assistant.tool_cards"), tr_n!("assistant.n_notes", count)),
                        "search_questions" => (tr!("assistant.tool_questions"), tr_n!("assistant.n_questions", count)),
                        other => (other.to_owned(), String::new()),
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(w, small + 4.0), Sense::hover());
                    ui.painter().circle_filled(pos2(r.left() + 3.5, r.center().y), 3.5, if count > 0 { p.good } else { p.hard });
                    let t = text(ui.painter(), pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &title, serif(small, W::Semi), p.text, w * 0.5);
                    text(ui.painter(), pos2(t.right() + 8.0, r.center().y), Align2::LEFT_CENTER, &arg, mono(small - 1.5), p.muted, (r.right() - t.right() - 8.0).max(10.0));
                    ui.add_space(-6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(w, small + 4.0), Sense::hover());
                    text(ui.painter(), pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, &format!("⎿ {summary}"), serif(small, W::Regular), p.muted, w - 14.0);
                }
                Entry::Ask { .. } => ask_entry(app, ui, i, w, compact),
                Entry::Answer(a) => {
                    ui.add(egui::Label::new(markdown(&a, size, p.text2, w)).selectable(true));
                }
                Entry::Error(e) => {
                    let job = LayoutJob::simple(tr!("assistant.failed", error = e), serif(small, W::Regular), p.again, w);
                    ui.add(egui::Label::new(job).selectable(true));
                }
            }
        }
        if app.assistant.run.is_some() && app.assistant.pending.is_none() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(small).color(p.muted));
                label(ui, &tr!("assistant.thinking"), serif(small, W::Italic), p.muted);
            });
        }
    });
}

/// Modelo em uso: o da IA online, ou um convite para configurar. Leva às Configurações.
fn model_chip(app: &mut App, ui: &mut Ui, height: f32) {
    let ready = app.cfg.ai_model == "online" && app.cfg.ai_api.ready();
    let name = if ready { app.cfg.ai_api.model.trim().to_owned() } else { tr!("assistant.no_model") };
    let dot: Option<Color32> = Some(if ready { pal().good } else { pal().dim });
    let resp = chip_dot(ui, &name, dot, false, height).on_hover_text(tr!("assistant.model_tip"));
    if resp.clicked() {
        app.go(Screen::Settings);
    }
}

fn attach_file(app: &mut App) {
    let Some(path) = rfd::FileDialog::new().add_filter(tr!("note.images_filter"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() else { return };
    match crate::media::store_file(&app.db.media_dir(), &path) {
        Ok(s) => {
            let _ = app.db.add_media(&s);
            app.assistant.attach.push(s.file_name);
        }
        Err(e) => app.error(e),
    }
}

/// Lista de páginas e baralhos para a @ menção.
fn mention_menu(app: &mut App, ui: &mut Ui) {
    ui.set_min_width(260.0);
    let resp = text_field(ui, &mut app.assistant.mention_query, &tr!("assistant.mention_search"), 260.0, 30.0);
    resp.request_focus();
    let q = crate::ai::ground::fold(app.assistant.mention_query.trim());
    let pages = app.db.all_pages().unwrap_or_default();
    let mut items: Vec<(String, &'static str, i64)> = pages.into_iter().map(|pg| (pg.title, "page", pg.id)).collect();
    items.extend(app.decks.iter().map(|d| (crate::search::deck_path(&app.decks, d.id), "deck", d.id)));
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for (title, kind, id) in items.into_iter().filter(|(t, ..)| q.is_empty() || crate::ai::ground::fold(t).contains(&q)).take(40) {
            let hint = tr!(if kind == "page" { "assistant.kind_page" } else { "assistant.kind_deck" });
            if menu_item_ex(ui, &title, None, false, Some(&hint)).clicked() {
                let st = &mut app.assistant;
                if !st.draft.is_empty() && !st.draft.ends_with(' ') {
                    st.draft.push(' ');
                }
                st.draft.push_str(&format!("@{title} "));
                st.mentions.push((title, kind, id));
                st.mention_query.clear();
                ui.close();
            }
        }
    });
}

/// Pedidos prontos ("/ comandos"): trocam o texto do campo.
fn command_menu(app: &mut App, ui: &mut Ui) {
    ui.set_min_width(300.0);
    for key in ["assistant.cmd_cards", "assistant.cmd_questions", "assistant.cmd_summary", "assistant.cmd_quiz_me", "assistant.cmd_weak"] {
        let t = tr!(key);
        if menu_item(ui, &t, false).clicked() {
            // Acrescenta ao que já está escrito (ex.: uma @ menção).
            let d = &mut app.assistant.draft;
            if !d.trim().is_empty() && !d.ends_with(' ') {
                d.push(' ');
            }
            d.push_str(&t);
            ui.ctx().memory_mut(|m| m.request_focus(input_id()));
            ui.close();
        }
    }
}

fn composer(app: &mut App, ui: &mut Ui, compact: bool) {
    let p = pal();
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let font = serif(if compact { 13.5 } else { 14.5 }, W::Regular);
    ui.painter().rect(rect, CornerRadius::same(if compact { 12 } else { 14 }), p.input, Stroke::new(1.0, p.border_control), StrokeKind::Inside);
    let row_h = if compact { 28.0 } else { 30.0 };
    // Imagem colada (Ctrl+V) com o campo em foco vai para a mensagem.
    if typing(&ctx) {
        if let Some(name) = app.pasted_image.take() {
            app.assistant.attach.push(name);
        }
    }
    // Enter envia; Shift+Enter quebra a linha. Tomado antes do campo, que o usaria.
    let mut go = typing(&ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let mut top = rect.top() + 10.0;
    if !app.assistant.attach.is_empty() {
        let strip = Rect::from_min_size(pos2(rect.left() + 10.0, top), vec2(rect.width() - 20.0, 30.0));
        let mut su = ui.new_child(egui::UiBuilder::new().max_rect(strip).layout(egui::Layout::left_to_right(egui::Align::Center)));
        su.spacing_mut().item_spacing.x = 6.0;
        let media = app.db.media_dir();
        let mut remove = None;
        for (k, f) in app.assistant.attach.iter().enumerate() {
            richtext::image(&mut su, &crate::media::uri(&media, f), 40.0, 28.0, 4, Sense::hover());
            if icon_button(&mut su, "×", &tr!("assistant.remove_image"), 22.0).clicked() {
                remove = Some(k);
            }
        }
        if let Some(k) = remove {
            app.assistant.attach.remove(k);
        }
        top += 40.0;
    }
    let field = Rect::from_min_max(pos2(rect.left() + 12.0, top), pos2(rect.right() - 12.0, rect.bottom() - row_h - 10.0));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(field));
    let hint = tr!(if compact { "assistant.placeholder_short" } else { "assistant.placeholder" });
    let resp = fu.add_sized(
        field.size(),
        egui::TextEdit::multiline(&mut app.assistant.draft).id(input_id()).frame(egui::Frame::NONE).font(font).hint_text(egui::RichText::new(hint).color(p.muted)),
    );
    a11y(&resp, egui::accesskit::Role::MultilineTextInput, &tr!("assistant.input_label"), None);

    let row = Rect::from_min_max(pos2(rect.left() + 6.0, rect.bottom() - row_h - 6.0), pos2(rect.right() - 6.0, rect.bottom() - 6.0));
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
    ru.spacing_mut().item_spacing.x = 2.0;
    if button(&mut ru, &tr!("assistant.attach"), Btn::Ghost, row_h).on_hover_text(keys(&tr!("assistant.attach_tip"))).clicked() {
        attach_file(app);
    }
    let m = button(&mut ru, &tr!(if compact { "assistant.mention_short" } else { "assistant.mention" }), Btn::Ghost, row_h);
    egui::Popup::menu(&m).width(280.0).frame(popup_frame()).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| mention_menu(app, ui));
    if !compact {
        let c = button(&mut ru, &tr!("assistant.commands"), Btn::Ghost, row_h);
        egui::Popup::menu(&c).width(320.0).frame(popup_frame()).show(|ui| command_menu(app, ui));
    }
    let busy = app.assistant.run.is_some();
    ru.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let enabled = !busy && !app.assistant.draft.trim().is_empty();
        let send = button_sized(ui, "↑", None, Btn::Primary, vec2(row_h, row_h), enabled);
        a11y(&send, egui::accesskit::Role::Button, &tr!("assistant.send"), None);
        go |= send.on_hover_text(tr!("assistant.send")).clicked() && enabled;
        if !compact {
            let mode = tr!(match app.cfg.assistant_write {
                Perm::Ask => "assistant.composer_ask",
                Perm::Allow => "assistant.composer_allow",
                Perm::Block => "assistant.composer_block",
            });
            let b = button(ui, &format!("{mode} ▾"), Btn::Ghost, row_h);
            egui::Popup::menu(&b).width(240.0).frame(popup_frame()).show(|ui| perm_menu(app, ui));
        }
    });
    if go && !busy {
        send(app, &ctx, compact);
    }
}

/// Markdown simples da resposta num só texto (para selecionar e copiar tudo):
/// títulos `#`, listas `-`/`*`/`1.`, blocos de código e a marcação inline do
/// app (`**negrito**`, `*itálico*`, `` `código` ``).
fn markdown(text: &str, size: f32, color: Color32, wrap: f32) -> LayoutJob {
    let p = pal();
    let mut out = LayoutJob::default();
    let mut code = false;
    let look = |size: f32, semi: bool, color: Color32| Look { size, semi, italic: false, color, line_height: None };
    for line in text.lines() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if !out.text.is_empty() {
            out.append("\n", 0.0, egui::TextFormat { font_id: serif(size, W::Regular), color, ..Default::default() });
        }
        if code {
            out.append(t, 0.0, egui::TextFormat { font_id: mono(size * 0.84), color, background: p.chip, ..Default::default() });
            continue;
        }
        let body = t.trim_start();
        let indent = " ".repeat((t.len() - body.len()).min(8));
        let numbered = body.find(". ").filter(|&d| (1..=3).contains(&d) && body[..d].chars().all(|c| c.is_ascii_digit()));
        let (prefix, rest, lk) = if let Some(h) = body.strip_prefix("### ").or_else(|| body.strip_prefix("## ")).or_else(|| body.strip_prefix("# ")) {
            (String::new(), h, look(size + 1.5, true, p.text))
        } else if let Some(item) = body.strip_prefix("- ").or_else(|| body.strip_prefix("* ")) {
            (format!("{indent}•  "), item, look(size, false, color))
        } else if let Some(d) = numbered {
            (format!("{indent}{}.  ", &body[..d]), &body[d + 2..], look(size, false, color))
        } else {
            (String::new(), body, look(size, false, color))
        };
        if !prefix.is_empty() {
            out.append(&prefix, 0.0, egui::TextFormat { font_id: serif(size, W::Regular), color: p.muted, ..Default::default() });
        }
        let (job, _) = richtext::job(rest, &lk, false, wrap);
        for sec in job.sections {
            out.append(&job.text[sec.byte_range.start.0..sec.byte_range.end.0], sec.leading_space, sec.format);
        }
    }
    out.wrap.max_width = wrap;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_keeps_text_and_drops_markers() {
        let job = markdown("### Título\nO **mosaico** fluido:\n- bicamada\n2. colesterol\n```\nx = 1\n```", 15.0, Color32::WHITE, 400.0);
        assert_eq!(job.text, "Título\nO mosaico fluido:\n•  bicamada\n2.  colesterol\nx = 1");
        // "mosaico" em negrito, o resto não.
        let bold = job.sections.iter().find(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0] == "mosaico").unwrap();
        let plain = job.sections.iter().find(|s| job.text[s.byte_range.start.0..s.byte_range.end.0].starts_with("O ")).unwrap();
        assert_ne!(bold.format.font_id, plain.format.font_id);
    }

    #[test]
    fn clozes_read_as_brackets() {
        assert_eq!(show_cloze("A {{c1::mitose}} gera {{c2::duas::número}} células"), "A [mitose] gera [duas] células");
        assert_eq!(show_cloze("sem lacuna"), "sem lacuna");
    }
}
