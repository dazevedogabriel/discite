//! Disciplina (Baralhos.dc.html): cabeçalho com abas Caderno · Baralhos ·
//! Estatísticas, cards de retenção/hoje/carga e a árvore de baralhos.

use super::modals::Modal;
use super::theme::*;
use super::widgets::*;
use super::{App, Screen, Tab, page, review, stats_view};
use crate::{tr, tr_n};
use crate::db::Deck;
use crate::stats::{self, Scope};
use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use std::collections::HashMap;

#[derive(Default)]
pub struct SubjectState {
    pub last_tab: HashMap<i64, Tab>,
    pub last_page: HashMap<i64, i64>,
    cache: Option<(u64, i64, DeckCache)>,
}

#[derive(Default, Clone)]
struct DeckCache {
    retention: Option<f32>,
    forecast: Vec<u32>,
    cards: u32,
}

/// Abre o caderno na última página vista (ou na primeira).
pub fn open_notebook(app: &mut App, sid: i64) {
    if let Some(p) = &app.page {
        if p.meta.subject_id == sid {
            app.go(Screen::Subject(sid, Tab::Notebook));
            return;
        }
    }
    let pages = app.db.list_pages(sid).unwrap_or_default();
    let target = app.subject_view.last_page.get(&sid).copied().filter(|id| pages.iter().any(|p| p.id == *id)).or_else(|| pages.iter().find(|p| p.parent_id.is_none()).map(|p| p.id));
    match target {
        Some(id) => app.open_page(id),
        None => {
            if let Some(p) = app.page.as_mut() {
                p.flush(&app.db);
            }
            app.page = None;
            app.go(Screen::Subject(sid, Tab::Notebook));
        }
    }
}

pub fn ui(app: &mut App, ui: &mut Ui, sid: i64, current: Tab) {
    let tab_sel = current;
    let Some(subject) = app.subject(sid).cloned() else {
        app.go(Screen::Dashboard);
        return;
    };
    if tab_sel == Tab::Notebook {
        page::ui(app, ui, sid);
        return;
    }
    let p = pal();
    let color = subject_color(subject.color);
    let decks: Vec<Deck> = app.decks.iter().filter(|d| d.subject_id == sid).cloned().collect();
    let deck_ids: Vec<i64> = decks.iter().map(|d| d.id).collect();
    let version = app.data_version;
    if app.subject_view.cache.as_ref().is_none_or(|(v, s, _)| *v != version || *s != sid) {
        let today = app.clock.today();
        let ov = stats::overview(&app.db, &Scope(Some(&deck_ids)), app.clock.day_start(today - 29), 30).ok();
        let fc = stats::forecast(&app.db, &Scope(Some(&deck_ids)), &app.clock, 7).unwrap_or_default();
        let cards = stats::card_counts(&app.db, &Scope(Some(&deck_ids))).map(|c| c.total()).unwrap_or(0);
        app.subject_view.cache = Some((version, sid, DeckCache { retention: ov.and_then(|o| o.retention), forecast: fc, cards }));
    }
    let cache = app.subject_view.cache.as_ref().map(|c| c.2.clone()).unwrap_or_default();

    // ── Cabeçalho ──
    let head = ui.available_rect_before_wrap();
    let head = Rect::from_min_size(head.min, vec2(head.width(), 128.0));
    ui.allocate_rect(head, Sense::hover());
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let x0 = head.left() + 28.0;
    let icon = Rect::from_min_size(pos2(x0, head.top() + 18.0), vec2(40.0, 40.0));
    ui.painter().rect_filled(icon, CornerRadius::same(10), color);
    let initial: String = subject.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    text(ui.painter(), icon.center(), Align2::CENTER_CENTER, &initial, serif(20.0, W::SemiItalic), p.window, 40.0);
    text(ui.painter(), pos2(icon.right() + 14.0, icon.top() + 11.0), Align2::LEFT_CENTER, &subject.name, serif(22.0, W::Semi), p.text, 500.0);
    let roots = decks.iter().filter(|d| d.parent_id.is_none()).count();
    let preset = roots.checked_sub(1).and_then(|_| decks.iter().find(|d| d.parent_id.is_none())).map(|d| app.preset_of_deck(d.id).clone());
    let preset_txt = preset
        .map(|pr| {
            tr!(
                "subject.preset_info",
                name = pr.name,
                algorithm = if pr.cfg.algorithm == crate::srs::Algorithm::Fsrs { "FSRS-6" } else { "SM-2" },
                pct = (pr.cfg.desired_retention * 100.0).round()
            )
        })
        .unwrap_or_default();
    let sub = tr_n!("subject.summary", decks.len(), cards = thousands(cache.cards as i64), preset = preset_txt);
    text(ui.painter(), pos2(icon.right() + 14.0, icon.top() + 31.0), Align2::LEFT_CENTER, &sub, serif(13.0, W::Regular), p.muted, 600.0);
    let btn_area = Rect::from_min_max(pos2(head.right() - 520.0, icon.top()), pos2(head.right() - 28.0, icon.bottom()));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(btn_area).layout(egui::Layout::right_to_left(egui::Align::Center)));
    bu.spacing_mut().item_spacing.x = 10.0;
    let c = app.subject_counts(sid);
    let study_label = tr!("subject.study", name = subject.name);
    let study_w = button_width(&bu, &study_label, None, 13.0, 14.0).min(260.0);
    if button_sized(&mut bu, &study_label, None, Btn::Primary, vec2(study_w, 34.0), c.new + c.learn + c.review > 0).clicked() {
        review::start(app, review::Scope::Subject(sid));
    }
    if button_kbd(&mut bu, &tr!("common.add_note"), Some("Ctrl ⇧ A"), Btn::Secondary, 34.0).clicked() {
        let deck = app.default_deck().filter(|d| deck_ids.contains(d)).or(deck_ids.first().copied());
        if let Some(d) = deck {
            app.note_editor = Some(super::note_editor::NoteEditor::new_note(d, None));
        }
    }
    let tabs = Rect::from_min_size(pos2(x0, head.bottom() - 41.0), vec2(400.0, 40.0));
    let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(tabs).layout(egui::Layout::left_to_right(egui::Align::Center)));
    tu.spacing_mut().item_spacing.x = 24.0;
    if tab(&mut tu, &tr!("nav.notebook"), false).clicked() {
        open_notebook(app, sid);
    }
    if tab(&mut tu, &tr!("nav.decks"), tab_sel == Tab::Decks).clicked() {
        app.go(Screen::Subject(sid, Tab::Decks));
    }
    if tab(&mut tu, &tr!("nav.stats"), tab_sel == Tab::Stats).clicked() {
        app.go(Screen::Subject(sid, Tab::Stats));
    }

    if tab_sel == Tab::Stats {
        stats_view::ui(app, ui, Some(sid));
        return;
    }

    let area = ui.available_rect_before_wrap();
    let content = Rect::from_min_max(area.min + vec2(28.0, 20.0), area.max - vec2(28.0, 24.0));

    // ── Cards de resumo ──
    let gap = 14.0;
    let cw = (content.width() - 2.0 * gap) / 3.0;
    let top = Rect::from_min_size(content.min, vec2(cw, 96.0));
    let desired = decks.iter().find(|d| d.parent_id.is_none()).map(|d| app.preset_of_deck(d.id).cfg.desired_retention).unwrap_or(0.9);
    // Retenção.
    paint_card(ui.painter(), top);
    caps(ui.painter(), top.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("subject.retention_30"), p.dim);
    match cache.retention {
        Some(r) => {
            let g = text(ui.painter(), top.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, &pct(r, 1), serif(30.0, W::Semi), p.text, 200.0);
            let diff = (r - desired) * 100.0;
            let (msg, col) = if diff >= -0.5 {
                (tr!("subject.above_goal", n = crate::srs::decimal(diff.max(0.0) as f64, 1)), p.good)
            } else {
                (tr!("subject.below_goal", n = crate::srs::decimal(-diff as f64, 1)), p.hard)
            };
            text(ui.painter(), pos2(g.right() + 10.0, top.top() + 52.0), Align2::LEFT_CENTER, &msg, serif(12.5, W::Regular), col, cw - g.width() - 40.0);
        }
        None => {
            text(ui.painter(), top.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, "—", serif(30.0, W::Semi), p.text, 200.0);
        }
    }
    text(ui.painter(), top.min + vec2(18.0, 76.0), Align2::LEFT_CENTER, &tr!("subject.desired", pct = (desired * 100.0).round()), serif(12.0, W::Italic), p.muted, cw - 36.0);
    // Hoje.
    let r2 = top.translate(vec2(cw + gap, 0.0));
    paint_card(ui.painter(), r2);
    caps(ui.painter(), r2.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("dash.today"), p.dim);
    let total = c.new + c.learn + c.review;
    let g = text(ui.painter(), r2.min + vec2(18.0, 48.0), Align2::LEFT_CENTER, &total.to_string(), serif(30.0, W::Semi), p.text, 120.0);
    text(ui.painter(), pos2(g.right() + 10.0, r2.top() + 52.0), Align2::LEFT_CENTER, &tr!("subject.cards_minutes", n = app.minutes_for(total)), serif(12.5, W::Regular), p.muted, cw - 80.0);
    text(ui.painter(), r2.min + vec2(18.0, 76.0), Align2::LEFT_CENTER, &tr!("subject.today_breakdown", review = c.review, new = c.new, learn = c.learn), serif(12.0, W::Italic), p.muted, cw - 36.0);
    // Carga.
    let r3 = r2.translate(vec2(cw + gap, 0.0));
    paint_card(ui.painter(), r3);
    caps(ui.painter(), r3.min + vec2(18.0, 20.0), Align2::LEFT_CENTER, &tr!("subject.load_7"), p.dim);
    let vals: Vec<f32> = cache.forecast.iter().map(|v| *v as f32).collect();
    let chart = Rect::from_min_max(r3.min + vec2(18.0, 36.0), r3.max - vec2(18.0, 16.0));
    super::charts::bars(ui.painter(), chart, &vals, color, 6.0, |i| if i == 0 { 1.0 } else { 0.55 });
    if vals.iter().all(|v| *v == 0.0) {
        text(ui.painter(), chart.center(), Align2::CENTER_CENTER, &tr!("subject.nothing_scheduled"), serif(12.0, W::Italic), p.muted, chart.width());
    }

    // ── Tabela de baralhos ──
    let table = Rect::from_min_max(pos2(content.left(), top.bottom() + 18.0), content.max);
    paint_card(ui.painter(), table);
    let cols = [table.right() - 18.0 - 150.0 - 90.0 - 110.0 - 90.0, 90.0, 110.0, 90.0, 150.0];
    let hy = table.top() + 20.0;
    caps(ui.painter(), pos2(table.left() + 18.0, hy), Align2::LEFT_CENTER, &tr!("deck.deck"), p.dim);
    let xs = [table.right() - 18.0 - 150.0 - 110.0 - 90.0, table.right() - 18.0 - 150.0 - 90.0, table.right() - 18.0 - 150.0];
    caps(ui.painter(), pos2(xs[0], hy), Align2::RIGHT_CENTER, &tr!("deck.new"), p.dim);
    caps(ui.painter(), pos2(xs[1], hy), Align2::RIGHT_CENTER, &tr!("deck.learning"), p.dim);
    caps(ui.painter(), pos2(xs[2], hy), Align2::RIGHT_CENTER, &tr!("deck.review"), p.dim);
    let _ = cols;
    ui.painter().hline(table.x_range(), table.top() + 40.0, Stroke::new(1.0, p.border));
    let footer_h = 54.0;
    let body = Rect::from_min_max(pos2(table.left() + 1.0, table.top() + 41.0), pos2(table.right() - 1.0, table.bottom() - footer_h));
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("decks", sid)).auto_shrink([false, false]).show(&mut bu, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut rows = Vec::new();
        for d in decks.iter().filter(|d| d.parent_id.is_none()) {
            flatten(&decks, d, 0, &mut rows);
        }
        for (d, lvl) in rows {
            deck_row(app, ui, &d, lvl, &decks, xs);
        }
        if decks.is_empty() {
            ui.add_space(30.0);
            label(ui, &tr!("deck.none"), serif(13.0, W::Italic), p.muted);
        }
    });
    ui.painter().hline(table.x_range(), table.bottom() - footer_h, Stroke::new(1.0, p.border));
    let foot = Rect::from_min_max(pos2(table.left() + 18.0, table.bottom() - footer_h), pos2(table.right() - 18.0, table.bottom()));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
    fu.spacing_mut().item_spacing.x = 8.0;
    if button(&mut fu, &tr!("deck.add"), Btn::Secondary, 30.0).clicked() {
        app.modal = Some(Modal::Deck { id: None, subject: sid, parent: None, name: String::new() });
    }
    if button(&mut fu, &tr!("deck.import_csv"), Btn::Ghost, 30.0).clicked() {
        super::modals::import_csv(app, sid);
    }
    let bury = decks.first().map(|d| app.preset_of_deck(d.id).cfg.bury_siblings).unwrap_or(true);
    if bury {
        text(fu.painter(), pos2(foot.right(), foot.center().y), Align2::RIGHT_CENTER, &tr!("deck.bury_note"), serif(12.0, W::Italic), p.muted, 400.0);
    }
}

fn flatten(all: &[Deck], d: &Deck, lvl: usize, out: &mut Vec<(Deck, usize)>) {
    out.push((d.clone(), lvl));
    if d.collapsed {
        return;
    }
    for k in all.iter().filter(|k| k.parent_id == Some(d.id)) {
        flatten(all, k, lvl + 1, out);
    }
}

fn deck_row(app: &mut App, ui: &mut Ui, d: &Deck, lvl: usize, all: &[Deck], xs: [f32; 3]) {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line_row));
    let has_kids = all.iter().any(|k| k.parent_id == Some(d.id));
    let x = rect.left() + 17.0 + lvl as f32 * 22.0;
    if has_kids {
        let caret = Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(18.0, 24.0));
        let cr = ui.interact(caret, ui.scope_id().with(("caret", d.id)), Sense::click());
        text(ui.painter(), caret.center(), Align2::CENTER_CENTER, if d.collapsed { "▸" } else { "▾" }, serif(10.0, W::Regular), p.dim, 18.0);
        if cr.clicked() {
            let mut nd = d.clone();
            nd.collapsed = !nd.collapsed;
            let _ = app.db.update_deck(&nd);
            app.reload();
            return;
        }
    }
    let bold = lvl == 0 || has_kids;
    text(ui.painter(), pos2(x + 20.0, rect.center().y), Align2::LEFT_CENTER, &d.name, serif(14.0, if bold { W::Semi } else { W::Regular }), p.text, xs[0] - x - 120.0);
    let c = app.deck_counts(d.id);
    for (i, (v, col)) in [(c.new, p.easy), (c.learn, p.hard), (c.review, p.good)].into_iter().enumerate() {
        text(ui.painter(), pos2(xs[i], rect.center().y), Align2::RIGHT_CENTER, &v.to_string(), serif(14.0, W::Regular), if v > 0 { col } else { p.dim }, 60.0);
    }
    let actions = Rect::from_min_max(pos2(rect.right() - 168.0, rect.top()), pos2(rect.right() - 17.0, rect.bottom()));
    let mut au = ui.new_child(egui::UiBuilder::new().max_rect(actions).layout(egui::Layout::right_to_left(egui::Align::Center)));
    au.spacing_mut().item_spacing.x = 6.0;
    let more = icon_button(&mut au, "⋯", &tr!("deck.options"), 28.0);
    egui::Popup::menu(&more).frame(popup_frame()).width(220.0).show(|ui| deck_menu(app, ui, d));
    if hovered && c.new + c.learn + c.review > 0 && button(&mut au, &tr!("nav.study"), Btn::Primary, 28.0).clicked() {
        review::start(app, review::Scope::Deck(d.id));
    }
    a11y(&resp, egui::accesskit::Role::Row, &d.name, None);
    if resp.double_clicked() {
        review::start(app, review::Scope::Deck(d.id));
    }
    resp.context_menu(|ui| deck_menu(app, ui, d));
}

fn deck_menu(app: &mut App, ui: &mut Ui, d: &Deck) {
    if menu_item(ui, &tr!("nav.study"), false).clicked() {
        review::start(app, review::Scope::Deck(d.id));
    }
    if menu_item(ui, &tr!("deck.custom_study"), false).clicked() {
        app.modal = Some(Modal::custom_study(Some(d.id)));
    }
    if menu_item(ui, &tr!("deck.add_note_here"), false).clicked() {
        app.note_editor = Some(super::note_editor::NoteEditor::new_note(d.id, None));
    }
    if menu_item(ui, &tr!("deck.view_cards"), false).clicked() {
        app.browser.query = format!("deck:\"{}\"", crate::search::deck_path(&app.decks, d.id));
        app.browser.dirty = true;
        app.go(Screen::Browser);
    }
    menu_sep(ui);
    if menu_item(ui, &tr!("deck.new_subdeck"), false).clicked() {
        app.modal = Some(Modal::Deck { id: None, subject: d.subject_id, parent: Some(d.id), name: String::new() });
    }
    if menu_item(ui, &tr!("common.rename"), false).clicked() {
        app.modal = Some(Modal::Deck { id: Some(d.id), subject: d.subject_id, parent: d.parent_id, name: d.name.clone() });
    }
    if menu_item(ui, &tr!("deck.preset_options"), false).clicked() {
        let pid = d.preset_id;
        app.go(Screen::Preset(pid));
    }
    ui.menu_button(tr!("deck.use_preset"), |ui| {
        for pr in app.presets.clone() {
            if menu_item(ui, &pr.name, pr.id == d.preset_id).clicked() {
                for id in crate::db::subtree(&app.decks, d.id) {
                    if let Some(mut x) = app.deck(id).cloned() {
                        x.preset_id = pr.id;
                        let _ = app.db.update_deck(&x);
                    }
                }
                app.reload();
            }
        }
    });
    menu_sep(ui);
    if menu_item(ui, &tr!("deck.delete"), false).clicked() {
        app.modal = Some(Modal::DeleteDeck(d.id));
    }
}
