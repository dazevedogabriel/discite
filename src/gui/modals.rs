//! Diálogos: disciplina, baralho, exclusões, informações do cartão,
//! histórico de versões, visualizador de imagem, estudo personalizado e
//! importação de CSV.

use super::theme::*;
use super::widgets::*;
use super::{App, Screen, browser, review};
use crate::cards::NoteKind;
use crate::db::{Block, Note};
use eframe::egui::{self, Align2, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::path::PathBuf;

pub enum Modal {
    Subject { id: Option<i64>, name: String, color: usize },
    DeleteSubject(i64),
    Deck { id: Option<i64>, subject: i64, parent: Option<i64>, name: String },
    DeleteDeck(i64),
    DeletePage(i64),
    DeleteNotes(Vec<i64>),
    CardInfo(i64),
    Versions(i64),
    Viewer(String, f32),
    CustomStudy { deck: Option<i64>, kind: usize, days: u32, pct: u32, limit: u32, query: String },
    ImportCsv(Box<CsvImport>),
    NewPreset { name: String, from: i64 },
    RenamePreset { id: i64, name: String },
    MoveCards { ids: Vec<i64>, deck: i64 },
    AddTags { note_ids: Vec<i64>, tags: String },
}

pub struct CsvImport {
    subject: i64,
    path: PathBuf,
    rows: Vec<Vec<String>>,
    deck: i64,
    kind: NoteKind,
    /// Coluna → campo (None = ignorar; usize::MAX = tags).
    mapping: Vec<Option<usize>>,
    header: bool,
}

impl Modal {
    pub fn custom_study(deck: Option<i64>) -> Modal {
        Modal::CustomStudy { deck, kind: 0, days: 1, pct: 80, limit: 100, query: String::new() }
    }
}

fn frame() -> egui::Frame {
    let p = pal();
    egui::Frame::NONE
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border_control))
        .corner_radius(14)
        .inner_margin(24)
        .shadow(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: egui::Color32::from_black_alpha(if p.dark { 140 } else { 46 }) })
}

fn title(ui: &mut Ui, t: &str, sub: &str) {
    let p = pal();
    label(ui, t, serif(19.0, W::Semi), p.text);
    if !sub.is_empty() {
        para(ui, sub, serif(13.0, W::Regular), p.muted);
    }
    ui.add_space(6.0);
}

/// Linha de botões à direita: devolve (confirmou, cancelou).
fn actions(ui: &mut Ui, ok: &str, danger: bool, enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let w = button_width(ui, ok, None, 13.0, 14.0);
        if button_sized(ui, ok, None, if danger { Btn::Danger } else { Btn::Primary }, vec2(w, 36.0), enabled).clicked() {
            res.0 = true;
        }
        if button(ui, "Cancelar", Btn::Secondary, 36.0).clicked() {
            res.1 = true;
        }
    });
    res
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut modal) = app.modal.take() else { return };
    let p = pal();
    let mut keep = true;
    let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let resp = egui::Modal::new(egui::Id::unique("modal")).backdrop_color(p.scrim).frame(frame()).show(ctx, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        match &mut modal {
            Modal::Subject { id, name, color } => {
                ui.set_width(420.0);
                title(ui, if id.is_some() { "Editar disciplina" } else { "Nova disciplina" }, "Disciplinas agrupam o caderno e os baralhos de um assunto.");
                caps_label(ui, "Nome");
                let r = text_field(ui, name, "Ex.: Anatomia", 372.0, 36.0);
                if id.is_none() && !r.has_focus() && name.is_empty() {
                    r.request_focus();
                }
                caps_label(ui, "Cor");
                ui.horizontal(|ui| {
                    for i in 0..8 {
                        let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                        ui.painter().circle_filled(r.center(), 11.0, subject_color(i));
                        if *color == i {
                            ui.painter().circle_stroke(r.center(), 14.5, Stroke::new(2.0, p.text));
                        }
                        a11y(&resp, egui::accesskit::Role::RadioButton, &format!("Cor {}", i + 1), Some(*color == i));
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            *color = i;
                        }
                    }
                });
                let (ok, cancel) = actions(ui, if id.is_some() { "Salvar" } else { "Criar" }, false, !name.trim().is_empty());
                if cancel {
                    keep = false;
                }
                if (ok || enter) && !name.trim().is_empty() {
                    let r = match id {
                        Some(id) => app.subject(*id).cloned().map(|mut s| {
                            s.name = name.trim().to_owned();
                            s.color = *color;
                            app.db.update_subject(&s).map(|_| s.id)
                        }),
                        None => Some(app.db.add_subject(name.trim(), *color)),
                    };
                    match r {
                        Some(Ok(sid)) => {
                            // O baralho raiz acompanha o nome da disciplina.
                            if let Some(mut d) = app.decks.iter().find(|d| d.subject_id == sid && d.parent_id.is_none()).cloned() {
                                if id.is_some() && app.root_decks(sid).len() == 1 && d.name != name.trim() {
                                    d.name = name.trim().to_owned();
                                    let _ = app.db.update_deck(&d);
                                }
                            }
                            app.reload();
                            if id.is_none() {
                                app.open_subject(sid);
                            }
                        }
                        Some(Err(e)) => app.error(e),
                        None => {}
                    }
                    keep = false;
                }
            }
            Modal::DeleteSubject(id) => {
                ui.set_width(440.0);
                let name = app.subject(*id).map(|s| s.name.clone()).unwrap_or_default();
                let cards = crate::stats::card_counts(&app.db, &crate::stats::Scope(Some(&app.subject_decks(*id)))).map(|c| c.total()).unwrap_or(0);
                let pages = app.db.list_pages(*id).map(|v| v.len()).unwrap_or(0);
                title(ui, &format!("Excluir “{name}”?"), &format!("Apaga {pages} páginas, os baralhos e {cards} cartões (com o histórico). Um backup é feito antes."));
                let (ok, cancel) = actions(ui, "Excluir disciplina", true, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    app.maybe_backup(true);
                    if app.page.as_ref().is_some_and(|p| p.meta.subject_id == *id) {
                        app.page = None;
                    }
                    if let Err(e) = app.db.delete_subject(*id) {
                        app.error(e);
                    }
                    app.reload();
                    app.go(Screen::Dashboard);
                    keep = false;
                }
            }
            Modal::Deck { id, subject, parent, name } => {
                ui.set_width(420.0);
                let sub = match parent {
                    Some(pid) => format!("Dentro de {}", app.deck_path(*pid)),
                    None => format!("Em {}", app.subject(*subject).map(|s| s.name.as_str()).unwrap_or("")),
                };
                title(ui, if id.is_some() { "Renomear baralho" } else { "Novo baralho" }, &sub);
                let r = text_field(ui, name, "Nome do baralho", 372.0, 36.0);
                if !r.has_focus() && name.is_empty() {
                    r.request_focus();
                }
                let (ok, cancel) = actions(ui, if id.is_some() { "Salvar" } else { "Criar" }, false, !name.trim().is_empty());
                if cancel {
                    keep = false;
                }
                if (ok || enter) && !name.trim().is_empty() {
                    let r = match id {
                        Some(id) => app.deck(*id).cloned().map(|mut d| {
                            d.name = name.trim().to_owned();
                            app.db.update_deck(&d)
                        }),
                        None => Some(app.db.add_deck(*subject, *parent, name.trim()).map(|_| ())),
                    };
                    if let Some(Err(e)) = r {
                        app.error(e);
                    }
                    app.reload();
                    keep = false;
                }
            }
            Modal::DeleteDeck(id) => {
                ui.set_width(440.0);
                let ids = crate::db::subtree(&app.decks, *id);
                let cards = crate::stats::card_counts(&app.db, &crate::stats::Scope(Some(&ids))).map(|c| c.total()).unwrap_or(0);
                title(ui, &format!("Excluir “{}”?", app.deck_path(*id)), &format!("Apaga {} sub-baralhos e {cards} cartões, com o histórico de revisões.", ids.len() - 1));
                let (ok, cancel) = actions(ui, "Excluir", true, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    app.maybe_backup(true);
                    let decks = app.decks.clone();
                    if let Err(e) = app.db.delete_deck(*id, &decks) {
                        app.error(e);
                    }
                    app.reload();
                    keep = false;
                }
            }
            Modal::DeletePage(id) => {
                ui.set_width(420.0);
                let t = app.db.page(*id).ok().flatten().map(|p| p.title).unwrap_or_default();
                title(ui, &format!("Excluir “{t}”?"), "A página e as subpáginas vão para a lixeira interna. Os cartões criados a partir delas continuam existindo.");
                let (ok, cancel) = actions(ui, "Excluir página", true, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    if app.page.as_ref().is_some_and(|p| p.meta.id == *id) {
                        app.page = None;
                    }
                    if let Err(e) = app.db.delete_page(*id) {
                        app.error(e);
                    }
                    app.data_version += 1;
                    keep = false;
                }
            }
            Modal::DeleteNotes(ids) => {
                ui.set_width(420.0);
                title(ui, &format!("Excluir {} {}?", ids.len(), if ids.len() == 1 { "nota" } else { "notas" }), "Os cartões e o histórico de revisões delas são apagados.");
                let (ok, cancel) = actions(ui, "Excluir", true, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    if let Err(e) = app.db.delete_notes(ids) {
                        app.error(e);
                    }
                    app.browser.selected.clear();
                    app.browser.dirty = true;
                    app.changed();
                    keep = false;
                }
            }
            Modal::CardInfo(id) => {
                ui.set_width(520.0);
                match app.db.card(*id).ok().flatten() {
                    Some(card) => {
                        egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| browser::card_details(app, ui, &card, false));
                    }
                    None => {
                        label(ui, "Cartão não encontrado.", serif(14.0, W::Regular), p.muted);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, "Fechar", Btn::Secondary, 34.0).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Versions(page) => {
                ui.set_width(520.0);
                title(ui, "Histórico da página", "Uma versão é guardada a cada 10 minutos de edição e ao sair. Restaurar substitui o conteúdo atual (que também vira uma versão).");
                let versions = app.db.versions(*page).unwrap_or_default();
                if versions.is_empty() {
                    label(ui, "Nenhuma versão ainda.", serif(13.0, W::Italic), p.muted);
                }
                let mut restore: Option<(String, Vec<Block>)> = None;
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (vid, at, t, blocks) in &versions {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                        ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, p.line_row));
                        let when = {
                            use chrono::TimeZone;
                            chrono::Local.timestamp_millis_opt(*at).single().map(|t| t.format("%d/%m/%Y %H:%M").to_string()).unwrap_or_default()
                        };
                        text(ui.painter(), pos2(r.left(), r.center().y - 8.0), Align2::LEFT_CENTER, &when, serif(13.0, W::Semi), p.text, 200.0);
                        text(ui.painter(), pos2(r.left(), r.center().y + 9.0), Align2::LEFT_CENTER, &format!("{t} · {} blocos", blocks.len()), serif(12.0, W::Regular), p.muted, 300.0);
                        let br = Rect::from_min_size(pos2(r.right() - 96.0, r.center().y - 14.0), vec2(96.0, 28.0));
                        let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br));
                        if button_sized(&mut bu, "Restaurar", None, Btn::Secondary, vec2(96.0, 28.0), true).clicked() {
                            restore = Some((t.clone(), blocks.clone()));
                        }
                        let _ = vid;
                    }
                });
                if let Some((t, blocks)) = restore {
                    if let Some(ed) = app.page.as_mut().filter(|e| e.meta.id == *page) {
                        ed.snapshot(&app.db);
                        ed.blocks = blocks.clone();
                        ed.title = t.clone();
                    }
                    let _ = app.db.save_blocks(*page, &t, &blocks);
                    if let Ok(Some(meta)) = app.db.page(*page) {
                        if let Ok(ed) = super::page::PageEditor::open(&app.db, meta) {
                            if app.page.as_ref().is_some_and(|e| e.meta.id == *page) {
                                app.page = Some(ed);
                            }
                        }
                    }
                    app.data_version += 1;
                    app.toast("Versão restaurada");
                    keep = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, "Fechar", Btn::Secondary, 34.0).clicked() {
                        keep = false;
                    }
                });
            }
            Modal::Viewer(name, zoom) => {
                let screen = ctx.content_rect();
                let max = vec2(screen.width() - 120.0, screen.height() - 160.0);
                ui.set_max_size(max);
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    *zoom = (*zoom * (1.0 + scroll / 400.0)).clamp(0.2, 6.0);
                }
                egui::ScrollArea::both().max_width(max.x).max_height(max.y - 40.0).show(ui, |ui| {
                    let img = egui::Image::new(crate::media::uri(&app.db.media_dir(), name)).fit_to_original_size(*zoom);
                    ui.add(img);
                });
                ui.horizontal(|ui| {
                    label(ui, &format!("{}% · role para ampliar", (*zoom * 100.0).round()), serif(12.0, W::Italic), p.muted);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, "Fechar", Btn::Secondary, 32.0).clicked() {
                            keep = false;
                        }
                        if button(ui, "100%", Btn::Ghost, 32.0).clicked() {
                            *zoom = 1.0;
                        }
                    });
                });
            }
            Modal::CustomStudy { deck, kind, days, pct: pc, limit, query } => {
                ui.set_width(480.0);
                let scope = deck.map(|d| app.deck_path(d)).unwrap_or_else(|| "Todas as disciplinas".into());
                title(ui, "Estudo personalizado", &format!("Monta uma sessão fora da fila do dia. Escopo: {scope}."));
                let opts = ["Esquecidos nos últimos dias", "Revisar antes do vencimento", "Novos adicionados recentemente", "Cartões em risco (R baixa)", "Cartões marcados", "Busca personalizada"];
                for (i, o) in opts.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(432.0, 30.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                    }
                    ui.painter().circle_stroke(pos2(r.left() + 12.0, r.center().y), 7.0, Stroke::new(1.2, if *kind == i { p.accent } else { p.border_strong }));
                    if *kind == i {
                        ui.painter().circle_filled(pos2(r.left() + 12.0, r.center().y), 4.0, p.accent);
                    }
                    text(ui.painter(), pos2(r.left() + 28.0, r.center().y), Align2::LEFT_CENTER, o, serif(13.5, W::Regular), p.text2, 400.0);
                    a11y(&resp, egui::accesskit::Role::RadioButton, o, Some(*kind == i));
                    if resp.clicked() {
                        *kind = i;
                    }
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    match *kind {
                        0..=2 => {
                            label(ui, if *kind == 1 { "Próximos" } else { "Últimos" }, serif(13.0, W::Regular), p.text2);
                            number_field(ui, "days", days, "dias", 110.0);
                        }
                        3 => {
                            label(ui, "R abaixo de", serif(13.0, W::Regular), p.text2);
                            number_field(ui, "pct", pc, "%", 90.0);
                        }
                        5 => {
                            text_field(ui, query, "is:due tag:prova prop:r<0.8", 432.0, 36.0);
                        }
                        _ => {}
                    }
                });
                ui.horizontal(|ui| {
                    label(ui, "Máximo de cartões", serif(13.0, W::Regular), p.text2);
                    number_field(ui, "limit", limit, "", 90.0);
                });
                let (ok, cancel) = actions(ui, "Estudar", false, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    let base = deck.map(|d| format!("deck:\"{}\" ", crate::search::deck_path(&app.decks, d))).unwrap_or_default();
                    let q = match *kind {
                        0 => format!("{base}rated:{}:1", (*days).max(1)),
                        1 => format!("{base}is:review prop:due<={}", (*days).max(1)),
                        2 => format!("{base}is:new added:{}", (*days).max(1)),
                        3 => format!("{base}is:review prop:r<{}", (*pc).clamp(1, 100) as f32 / 100.0),
                        4 => format!("{base}is:flagged"),
                        _ => format!("{base}{query}"),
                    };
                    match browser::search_cards(app, &q) {
                        Ok(mut cards) => {
                            cards.retain(|c| c.queue != crate::srs::Queue::Suspended);
                            cards.truncate((*limit).max(1) as usize);
                            if cards.is_empty() {
                                app.toast("Nenhum cartão corresponde a esse critério");
                            } else {
                                let ids = cards.iter().map(|c| c.id).collect();
                                keep = false;
                                app.modal = None;
                                review::start(app, review::Scope::Cards(ids, format!("Estudo personalizado · {}", opts[*kind])));
                                return;
                            }
                        }
                        Err(e) => app.error(e),
                    }
                }
            }
            Modal::ImportCsv(imp) => {
                ui.set_width(620.0);
                csv_ui(app, ui, imp, &mut keep);
            }
            Modal::NewPreset { name, from } => {
                ui.set_width(420.0);
                title(ui, "Novo preset", "Copia as opções do preset atual. Depois, escolha quais baralhos usam cada preset.");
                let r = text_field(ui, name, "Ex.: Medicina", 372.0, 36.0);
                if !r.has_focus() && name.is_empty() {
                    r.request_focus();
                }
                let (ok, cancel) = actions(ui, "Criar", false, !name.trim().is_empty());
                if cancel {
                    keep = false;
                }
                if (ok || enter) && !name.trim().is_empty() {
                    let base = app.preset(*from).cloned();
                    let cfg = base.as_ref().map(|b| b.cfg.clone()).unwrap_or_default();
                    let params = base.map(|b| b.params).unwrap_or_default();
                    match app.db.add_preset(name.trim(), &cfg, &params) {
                        Ok(id) => {
                            app.reload();
                            app.preset_view = None;
                            app.go(Screen::Preset(id));
                        }
                        Err(e) => app.error(e),
                    }
                    keep = false;
                }
            }
            Modal::RenamePreset { id, name } => {
                ui.set_width(420.0);
                title(ui, "Renomear preset", "");
                text_field(ui, name, "Nome", 372.0, 36.0);
                let (ok, cancel) = actions(ui, "Salvar", false, !name.trim().is_empty());
                if cancel {
                    keep = false;
                }
                if (ok || enter) && !name.trim().is_empty() {
                    if let Some(mut pr) = app.preset(*id).cloned() {
                        pr.name = name.trim().to_owned();
                        let _ = app.db.save_preset(&pr);
                    }
                    app.reload();
                    app.preset_view = None;
                    keep = false;
                }
            }
            Modal::MoveCards { ids, deck } => {
                ui.set_width(460.0);
                title(ui, &format!("Mover {} cartões", ids.len()), "Escolha o baralho de destino.");
                let decks = app.decks.clone();
                let cur = app.deck_path(*deck);
                let color = app.subject_color_of_deck(*deck);
                select_field(ui, "move_deck", &cur, Some(color), 412.0, 36.0, |ui| {
                    for s in app.subjects.clone() {
                        for d in decks.iter().filter(|d| d.subject_id == s.id) {
                            if menu_item_ex(ui, &app.deck_path(d.id), Some(subject_color(s.color)), d.id == *deck, None).clicked() {
                                *deck = d.id;
                            }
                        }
                    }
                });
                let (ok, cancel) = actions(ui, "Mover", false, true);
                if cancel {
                    keep = false;
                }
                if ok {
                    if let Err(e) = app.db.move_cards(ids, *deck) {
                        app.error(e);
                    }
                    app.browser.dirty = true;
                    app.changed();
                    keep = false;
                }
            }
            Modal::AddTags { note_ids, tags } => {
                ui.set_width(420.0);
                title(ui, &format!("Adicionar tags a {} notas", note_ids.len()), "Separe por espaço. Prefixe com - para remover (ex.: -antiga).");
                let r = text_field(ui, tags, "prova-p2 revisar", 372.0, 36.0);
                if !r.has_focus() && tags.is_empty() {
                    r.request_focus();
                }
                let (ok, cancel) = actions(ui, "Aplicar", false, !tags.trim().is_empty());
                if cancel {
                    keep = false;
                }
                if (ok || enter) && !tags.trim().is_empty() {
                    let notes = app.db.notes_by_ids(note_ids).unwrap_or_default();
                    for (_, mut n) in notes {
                        let mut list: Vec<String> = n.tag_list();
                        for t in tags.split_whitespace() {
                            if let Some(rm) = t.strip_prefix('-') {
                                list.retain(|x| x != rm);
                            } else {
                                list.push(t.trim_start_matches('#').to_owned());
                            }
                        }
                        n.tags = list.join(" ");
                        let deck = app.db.note_cards(n.id).ok().and_then(|c| c.first().map(|c| c.deck_id)).unwrap_or(1);
                        let _ = app.db.update_note(&mut n, deck);
                    }
                    app.browser.dirty = true;
                    app.changed();
                    keep = false;
                }
            }
        }
    });
    if resp.should_close() {
        keep = false;
    }
    if keep && app.modal.is_none() {
        app.modal = Some(modal);
    }
}

// ───────────────────────────── CSV ─────────────────────────────

/// Abre o seletor de arquivo e o diálogo de mapeamento.
pub fn import_csv(app: &mut App, subject: i64) {
    let Some(path) = rfd::FileDialog::new().add_filter("CSV / TSV", &["csv", "tsv", "txt"]).pick_file() else { return };
    let text = match std::fs::read(&path) {
        Ok(b) => String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_owned(),
        Err(e) => {
            app.error(e);
            return;
        }
    };
    let rows = parse_csv(&text);
    if rows.is_empty() {
        app.toast("Arquivo vazio");
        return;
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let deck = app.root_decks(subject).first().map(|d| d.id).or_else(|| app.decks.first().map(|d| d.id)).unwrap_or(0);
    let mapping = (0..cols).map(|i| if i < 3 { Some(i) } else { None }).collect();
    app.modal = Some(Modal::ImportCsv(Box::new(CsvImport { subject, path, rows, deck, kind: NoteKind::Basic, mapping, header: false })));
}

/// CSV com aspas; separador detectado (tab, ponto e vírgula ou vírgula).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let sep = [('\t', first.matches('\t').count()), (';', first.matches(';').count()), (',', first.matches(',').count())].into_iter().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                if row.iter().any(|c| !c.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        if row.iter().any(|c| !c.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

fn csv_ui(app: &mut App, ui: &mut Ui, imp: &mut CsvImport, keep: &mut bool) {
    let p = pal();
    let fname = imp.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    title(ui, "Importar CSV", &format!("{fname} · {} linhas", imp.rows.len()));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let kind = imp.kind;
        select_field(ui, "csv_kind", kind.label(), None, 200.0, 36.0, |ui| {
            for k in [NoteKind::Basic, NoteKind::BasicReversed, NoteKind::Cloze, NoteKind::TypeIn] {
                if menu_item(ui, k.label(), k == kind).clicked() {
                    imp.kind = k;
                }
            }
        });
        let decks = app.decks.clone();
        let cur = app.deck_path(imp.deck);
        select_field(ui, "csv_deck", &cur, Some(app.subject_color_of_deck(imp.deck)), 360.0, 36.0, |ui| {
            for d in decks.iter() {
                if menu_item(ui, &app.deck_path(d.id), d.id == imp.deck).clicked() {
                    imp.deck = d.id;
                }
            }
        });
    });
    checkbox(ui, &mut imp.header, "A primeira linha é cabeçalho");
    caps_label(ui, "Colunas → campos");
    let fields = imp.kind.field_names();
    let cols = imp.mapping.len();
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for c in 0..cols {
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    let cur = match imp.mapping[c] {
                        None => "Ignorar".to_owned(),
                        Some(usize::MAX) => "Tags".to_owned(),
                        Some(f) => fields.get(f).map(|s| s.to_string()).unwrap_or_else(|| "Ignorar".into()),
                    };
                    select_field(ui, ("csv_col", c), &cur, None, 150.0, 32.0, |ui| {
                        if menu_item(ui, "Ignorar", imp.mapping[c].is_none()).clicked() {
                            imp.mapping[c] = None;
                        }
                        for (fi, f) in fields.iter().enumerate() {
                            if menu_item(ui, f, imp.mapping[c] == Some(fi)).clicked() {
                                imp.mapping[c] = Some(fi);
                            }
                        }
                        if menu_item(ui, "Tags", imp.mapping[c] == Some(usize::MAX)).clicked() {
                            imp.mapping[c] = Some(usize::MAX);
                        }
                    });
                    for r in imp.rows.iter().take(4) {
                        let v = r.get(c).cloned().unwrap_or_default();
                        label(ui, &v.chars().take(40).collect::<String>(), serif(12.0, W::Regular), p.muted);
                    }
                });
            }
        });
    });
    let (ok, cancel) = actions(ui, "Importar", false, imp.mapping.iter().any(|m| *m == Some(0)));
    if cancel {
        *keep = false;
    }
    if ok {
        let mut made = 0;
        let mut notes = 0;
        let skip = if imp.header { 1 } else { 0 };
        for r in imp.rows.iter().skip(skip) {
            let mut n = Note::new(imp.kind);
            for (c, m) in imp.mapping.iter().enumerate() {
                let v = r.get(c).cloned().unwrap_or_default();
                match m {
                    Some(usize::MAX) => n.tags = format!("{} {v}", n.tags),
                    Some(f) if *f < n.fields.len() => n.fields[*f] = v.trim().to_owned(),
                    _ => {}
                }
            }
            if n.fields[0].trim().is_empty() || crate::cards::card_ords(n.kind, &n.fields).is_empty() {
                continue;
            }
            match app.db.add_note(&mut n, imp.deck) {
                Ok(c) => {
                    made += c;
                    notes += 1;
                }
                Err(e) => {
                    app.error(e);
                    break;
                }
            }
        }
        app.changed();
        app.toast(format!("{notes} notas importadas · {made} cartões"));
        let _ = imp.subject;
        *keep = false;
    }
    let _ = StrokeKind::Inside;
}

#[cfg(test)]
mod tests {
    #[test]
    fn csv_quotes_and_separators() {
        let rows = super::parse_csv("frente;verso\n\"a; b\";\"linha\n2\"\n");
        assert_eq!(rows, vec![vec!["frente".to_owned(), "verso".to_owned()], vec!["a; b".to_owned(), "linha\n2".to_owned()]]);
    }
}
