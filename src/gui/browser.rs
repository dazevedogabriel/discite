//! Navegador de cartões (Navegador.dc.html): busca com a linguagem de
//! `search.rs`, filtros rápidos, tabela ordenável com seleção múltipla e o
//! painel de detalhes (D, S, R, curva de recuperabilidade e histórico).

use super::modals::Modal;
use super::theme::*;
use super::widgets::*;
use super::{App, review};
use crate::cards;
use crate::db::{Card, Note};
use crate::rich;
use crate::srs::{self, DAY_MS, Queue, State};
use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Col {
    Front,
    Deck,
    Due,
    Ivl,
    D,
    S,
    R,
    Lapses,
    Reps,
    Kind,
    Added,
}

impl Col {
    fn label(self) -> &'static str {
        match self {
            Col::Front => "Frente",
            Col::Deck => "Baralho",
            Col::Due => "Vence",
            Col::Ivl => "Int.",
            Col::D => "D",
            Col::S => "S",
            Col::R => "R",
            Col::Lapses => "L",
            Col::Reps => "Rev.",
            Col::Kind => "Tipo",
            Col::Added => "Criado",
        }
    }
    fn width(self) -> f32 {
        match self {
            Col::Front => 0.0,
            Col::Deck => 118.0,
            Col::Due => 62.0,
            Col::Ivl => 52.0,
            Col::D => 34.0,
            Col::S => 46.0,
            Col::R => 44.0,
            Col::Lapses => 26.0,
            Col::Reps => 36.0,
            Col::Kind => 90.0,
            Col::Added => 62.0,
        }
    }
    fn numeric(self) -> bool {
        matches!(self, Col::Ivl | Col::D | Col::S | Col::R | Col::Lapses | Col::Reps)
    }
}

const OPTIONAL: [Col; 10] = [Col::Deck, Col::Due, Col::Ivl, Col::D, Col::S, Col::R, Col::Lapses, Col::Reps, Col::Kind, Col::Added];

pub struct Row {
    pub card: Card,
    front: String,
    deck: String,
    kind: String,
    r: Option<f32>,
    note_id: i64,
}

pub struct BrowserState {
    pub query: String,
    pub dirty: bool,
    pub focus_search: bool,
    pub selected: Vec<i64>,
    pub select_note: Option<i64>,
    anchor: Option<usize>,
    rows: Vec<Row>,
    version: u64,
    sort: (Col, bool),
    cols: Vec<Col>,
    errors: Vec<String>,
    total: i64,
    leeches: i64,
}

impl Default for BrowserState {
    fn default() -> Self {
        BrowserState {
            query: String::new(),
            dirty: true,
            focus_search: false,
            selected: vec![],
            select_note: None,
            anchor: None,
            rows: vec![],
            version: 0,
            sort: (Col::Added, false),
            cols: vec![Col::Deck, Col::Due, Col::Ivl, Col::D, Col::S, Col::R, Col::Lapses],
            errors: vec![],
            total: 0,
            leeches: 0,
        }
    }
}

/// Executa uma busca e devolve os cartões (com o filtro de R aplicado).
pub fn search_cards(app: &App, q: &str) -> anyhow::Result<Vec<Card>> {
    let parsed = crate::search::parse(q, &app.decks, &app.subjects, &app.clock, srs::now_ms());
    if let Some(e) = parsed.errors.first() {
        anyhow::bail!("{e}");
    }
    let ids: Vec<i64> = {
        let mut st = app.db.conn.prepare(&format!("SELECT c.id FROM cards c JOIN notes n ON n.id = c.note_id WHERE {} ORDER BY c.id", parsed.sql))?;
        st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut cards = app.db.cards_by_ids(&ids)?;
    if !parsed.r_filter.is_empty() {
        let now = srs::now_ms();
        cards.retain(|c| {
            let r = srs::retrievability(&c.s, app.decay_of_deck(c.deck_id), &app.clock, now);
            parsed.r_filter.iter().all(|(cmp, v, neg)| {
                let ok = r.is_some_and(|r| cmp.test(r, *v));
                ok != *neg
            })
        });
    }
    Ok(cards)
}

fn refresh(app: &mut App) {
    if !app.browser.dirty && app.browser.version == app.data_version {
        return;
    }
    app.browser.dirty = false;
    app.browser.version = app.data_version;
    app.browser.total = app.db.conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0)).unwrap_or(0);
    app.browser.leeches = app.db.conn.query_row("SELECT COUNT(*) FROM cards WHERE leech = 1", [], |r| r.get(0)).unwrap_or(0);
    let q = app.browser.query.clone();
    match search_cards(app, &q) {
        Ok(cards) => {
            app.browser.errors.clear();
            let note_ids: Vec<i64> = cards.iter().map(|c| c.note_id).collect();
            let notes: HashMap<i64, Note> = app.db.notes_by_ids(&note_ids).unwrap_or_default();
            let now = srs::now_ms();
            app.browser.rows = cards
                .into_iter()
                .map(|c| {
                    let n = notes.get(&c.note_id);
                    let front = n
                        .map(|n| {
                            let f = cards::render(n.kind, &n.fields, c.ord);
                            let q = rich::plain(&f.question);
                            if q.trim().is_empty() { n.kind.label().to_owned() } else { q.replace('\n', " ") }
                        })
                        .unwrap_or_default();
                    Row {
                        r: srs::retrievability(&c.s, app.decay_of_deck(c.deck_id), &app.clock, now),
                        deck: app.deck(c.deck_id).map(|d| d.name.clone()).unwrap_or_default(),
                        kind: n.map(|n| n.kind.label().to_owned()).unwrap_or_default(),
                        note_id: c.note_id,
                        front,
                        card: c,
                    }
                })
                .collect();
            sort_rows(&mut app.browser);
            if let Some(nid) = app.browser.select_note.take() {
                app.browser.selected = app.browser.rows.iter().filter(|r| r.note_id == nid).map(|r| r.card.id).take(1).collect();
            }
            let ids: std::collections::HashSet<i64> = app.browser.rows.iter().map(|r| r.card.id).collect();
            app.browser.selected.retain(|id| ids.contains(id));
        }
        Err(e) => {
            app.browser.errors = vec![format!("{e:#}")];
            app.browser.rows.clear();
        }
    }
}

fn sort_rows(b: &mut BrowserState) {
    let (col, asc) = b.sort;
    let key = |r: &Row| -> f64 {
        match col {
            Col::Due => {
                if r.card.s.state == State::New {
                    f64::MAX / 2.0 + r.card.position as f64
                } else {
                    r.card.s.due as f64
                }
            }
            Col::Ivl => r.card.s.interval as f64,
            Col::D => r.card.s.difficulty.unwrap_or(-1.0) as f64,
            Col::S => r.card.s.stability.unwrap_or(-1.0) as f64,
            Col::R => r.r.map(|v| v as f64).unwrap_or(2.0),
            Col::Lapses => r.card.s.lapses as f64,
            Col::Reps => r.card.s.reps as f64,
            Col::Added => r.card.created_at as f64,
            _ => 0.0,
        }
    };
    match col {
        Col::Front => b.rows.sort_by(|a, c| a.front.to_lowercase().cmp(&c.front.to_lowercase())),
        Col::Deck => b.rows.sort_by(|a, c| a.deck.cmp(&c.deck)),
        Col::Kind => b.rows.sort_by(|a, c| a.kind.cmp(&c.kind)),
        _ => b.rows.sort_by(|a, c| key(a).partial_cmp(&key(c)).unwrap_or(std::cmp::Ordering::Equal)),
    }
    if !asc {
        b.rows.reverse();
    }
}

fn due_label(app: &App, c: &Card) -> String {
    match c.s.state {
        State::New => "novo".into(),
        _ if c.queue == Queue::Suspended => "suspenso".into(),
        _ => {
            let today = app.clock.today();
            let d = app.clock.day_of(c.s.due) - today;
            match d {
                i64::MIN..=0 => "hoje".into(),
                1 => "amanhã".into(),
                2..=60 => format!("em {d} d"),
                _ => short_date(c.s.due),
            }
        }
    }
}

/// "26 d", "2,9 m", "1,5 a" (cabe na coluna estreita).
fn compact_days(d: f32) -> String {
    let d = d as f64;
    if d < 60.0 {
        format!("{} d", d.round())
    } else if d < 365.0 {
        format!("{} m", srs::decimal(d / 30.4, 1))
    } else {
        format!("{} a", srs::decimal(d / 365.0, 1))
    }
}

fn r_color(r: f32) -> Color32 {
    let p = pal();
    if r < 0.7 {
        p.again
    } else if r < 0.85 {
        p.hard
    } else {
        p.good
    }
}

pub fn ui(app: &mut App, ui: &mut Ui) {
    refresh(app);
    let p = pal();
    let full = ui.max_rect();

    // ── Topo ──
    let head = Rect::from_min_size(full.min, vec2(full.width(), 132.0));
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.line));
    let inner = head.shrink2(vec2(20.0, 14.0));
    let t = text(ui.painter(), pos2(inner.left(), inner.top() + 14.0), Align2::LEFT_CENTER, "Cartões", serif(22.0, W::Semi), p.text, 200.0);
    let count = format!("{} de {} cartões{}", thousands(app.browser.rows.len() as i64), thousands(app.browser.total), if app.browser.selected.len() > 1 { format!(" · {} selecionados", app.browser.selected.len()) } else { String::new() });
    text(ui.painter(), pos2(t.right() + 12.0, inner.top() + 16.0), Align2::LEFT_CENTER, &count, serif(13.0, W::Regular), p.muted, 400.0);
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(inner.min, vec2(inner.width(), 30.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    bu.spacing_mut().item_spacing.x = 8.0;
    if button(&mut bu, "Adicionar nota", Btn::Primary, 32.0).clicked() {
        app.add_note();
    }
    let cr = button(&mut bu, "Colunas", Btn::Secondary, 32.0);
    egui::Popup::menu(&cr).frame(popup_frame()).width(200.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for c in OPTIONAL {
            let mut on = app.browser.cols.contains(&c);
            if checkbox(ui, &mut on, c.label()).clicked() {
                if on {
                    app.browser.cols.push(c);
                    app.browser.cols.sort_by_key(|x| OPTIONAL.iter().position(|o| o == x));
                } else {
                    app.browser.cols.retain(|x| *x != c);
                }
            }
        }
    });

    // Busca.
    let sr = Rect::from_min_size(pos2(inner.left(), inner.top() + 38.0), vec2(inner.width(), 38.0));
    let mut su = ui.new_child(egui::UiBuilder::new().max_rect(sr));
    let sid = egui::Id::unique("browser_search");
    let focused = ui.ctx().memory(|m| m.has_focus(sid));
    su.painter().rect(sr, CornerRadius::same(8), p.input, Stroke::new(1.0, if focused { p.accent } else { p.border_control }), StrokeKind::Inside);
    if focused {
        su.painter().rect_stroke(sr.expand(2.0), CornerRadius::same(10), Stroke::new(3.0, p.accent_soft), StrokeKind::Inside);
    }
    text(su.painter(), pos2(sr.left() + 14.0, sr.center().y), Align2::LEFT_CENTER, "⌕", serif(15.0, W::Regular), p.muted, 20.0);
    let mut tu = su.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(sr.left() + 36.0, sr.top()), pos2(sr.right() - 70.0, sr.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let mut lay = query_layouter();
    let resp = tu.add(
        egui::TextEdit::singleline(&mut app.browser.query)
            .id(sid)
            .frame(egui::Frame::NONE)
            .layouter(&mut lay)
            .hint_text(egui::RichText::new("deck:Farmacologia is:due prop:r<0.8 lapses>1 tag:prova \"texto\"").font(mono(12.5)).color(p.dim))
            .desired_width(sr.width() - 110.0),
    );
    kbd(su.painter(), pos2(sr.right() - 12.0, sr.center().y), Align2::RIGHT_CENTER, "Ctrl F");
    if app.browser.focus_search {
        resp.request_focus();
        app.browser.focus_search = false;
    }
    if resp.changed() {
        app.browser.dirty = true;
    }
    if ui.ctx().input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        resp.request_focus();
    }

    // Filtros rápidos.
    let leeches = app.browser.leeches;
    let fr = Rect::from_min_size(pos2(inner.left(), sr.bottom() + 10.0), vec2(inner.width(), 24.0));
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(fr).layout(egui::Layout::left_to_right(egui::Align::Center)));
    fu.spacing_mut().item_spacing.x = 6.0;
    let leech_label = format!("Sanguessugas · {leeches}");
    for (label_s, term) in [
        ("Vencidos hoje", "is:due"),
        ("Esquecidos hoje", "rated:1:1"),
        ("R < 80%", "prop:r<0.8"),
        ("Novos", "is:new"),
        ("Marcados", "is:flagged"),
        (leech_label.as_str(), "is:leech"),
        ("Suspensos", "is:suspended"),
    ] {
        let on = app.browser.query.split_whitespace().any(|t| t == term);
        if chip(&mut fu, label_s, on, 24.0).clicked() {
            if on {
                app.browser.query = app.browser.query.split_whitespace().filter(|t| *t != term).collect::<Vec<_>>().join(" ");
            } else {
                app.browser.query = format!("{} {term}", app.browser.query.trim()).trim().to_owned();
            }
            app.browser.dirty = true;
        }
    }
    if let Some(e) = app.browser.errors.first() {
        text(fu.painter(), pos2(fr.right(), fr.center().y), Align2::RIGHT_CENTER, e, serif(12.0, W::Italic), p.again, 360.0);
    }

    // ── Tabela + painel ──
    let body = Rect::from_min_max(pos2(full.left(), head.bottom()), full.max);
    let panel_w = 330.0;
    let table = Rect::from_min_max(body.min, pos2(body.right() - panel_w, body.bottom()));
    let panel = Rect::from_min_max(pos2(body.right() - panel_w, body.top()), body.max);
    table_ui(app, ui, table);
    ui.painter().rect_filled(panel, 0, p.card);
    ui.painter().vline(panel.left(), panel.y_range(), Stroke::new(1.0, p.line));
    let mut pu = ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink2(vec2(18.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    side_panel(app, &mut pu);

    // Teclado: ↑/↓ movem a seleção; Delete exclui.
    if !ui.ctx().egui_wants_keyboard_input() && app.modal.is_none() && app.note_editor.is_none() && app.palette.is_none() {
        let n = app.browser.rows.len();
        let cur = app.browser.selected.first().and_then(|id| app.browser.rows.iter().position(|r| r.card.id == *id));
        let mv = |d: i64| cur.map(|c| (c as i64 + d).clamp(0, n as i64 - 1) as usize).unwrap_or(0);
        if n > 0 && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            let i = mv(1);
            app.browser.selected = vec![app.browser.rows[i].card.id];
            app.browser.anchor = Some(i);
        }
        if n > 0 && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            let i = mv(-1);
            app.browser.selected = vec![app.browser.rows[i].card.id];
            app.browser.anchor = Some(i);
        }
        if !app.browser.selected.is_empty() && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            app.modal = Some(Modal::DeleteNotes(selected_notes(app)));
        }
        if ui.ctx().input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            app.browser.selected = app.browser.rows.iter().map(|r| r.card.id).collect();
        }
    }
}

/// Destaca `chave:` em cor de destaque, como no design.
fn query_layouter() -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let p = pal();
        let mut job = egui::text::LayoutJob::default();
        let s = buf.as_str();
        let mut i = 0;
        let bytes = s.as_bytes();
        while i < s.len() {
            // Palavra até o próximo espaço.
            let end = s[i..].find(' ').map(|e| i + e).unwrap_or(s.len());
            let word = &s[i..end];
            let key_len = word.find([':', '<', '>']).filter(|k| *k > 0 && word[..*k].chars().all(|c| c.is_alphabetic() || c == '-'));
            let fmt = |c: Color32| egui::TextFormat { font_id: mono(12.5), color: c, ..Default::default() };
            match key_len {
                Some(k) => {
                    job.append(&word[..k + 1], 0.0, fmt(p.accent));
                    job.append(&word[k + 1..], 0.0, fmt(p.text));
                }
                None => job.append(word, 0.0, fmt(p.text)),
            }
            if end < s.len() {
                job.append(" ", 0.0, fmt(p.text));
            }
            i = end + 1;
            let _ = bytes;
        }
        if job.sections.is_empty() {
            job.append("", 0.0, egui::TextFormat { font_id: mono(12.5), color: p.text, ..Default::default() });
        }
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

fn col_x(cols: &[Col], rect: Rect) -> Vec<(Col, f32, f32)> {
    let fixed: f32 = cols.iter().map(|c| c.width() + 8.0).sum();
    let front_w = (rect.width() - 36.0 - fixed).max(120.0);
    let mut out = vec![(Col::Front, rect.left() + 20.0, front_w)];
    let mut x = rect.left() + 20.0 + front_w + 8.0;
    for c in cols {
        out.push((*c, x, c.width()));
        x += c.width() + 8.0;
    }
    out
}

fn table_ui(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = pal();
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
    ui.painter().rect_filled(header, 0, p.card);
    ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, p.border));
    let cols = app.browser.cols.clone();
    let xs = col_x(&cols, rect);
    for (c, x, w) in &xs {
        let sorted = app.browser.sort.0 == *c;
        let label_s = if sorted { format!("{} {}", c.label(), if app.browser.sort.1 { "▴" } else { "▾" }) } else { c.label().to_owned() };
        let hr = Rect::from_min_size(pos2(*x, header.top()), vec2(*w, 34.0));
        let resp = ui.interact(hr, ui.scope_id().with(("hdr", *c as u8)), Sense::click());
        let col = if sorted || resp.hovered() { p.text2 } else { p.dim };
        if c.numeric() {
            caps(ui.painter(), pos2(hr.right(), hr.center().y), Align2::RIGHT_CENTER, &label_s, col);
        } else {
            caps(ui.painter(), pos2(hr.left(), hr.center().y), Align2::LEFT_CENTER, &label_s, col);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if sorted {
                app.browser.sort.1 = !app.browser.sort.1;
            } else {
                app.browser.sort = (*c, matches!(c, Col::Front | Col::Deck | Col::Kind | Col::R | Col::Due));
            }
            sort_rows(&mut app.browser);
        }
    }
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let n = app.browser.rows.len();
    if n == 0 {
        text(bu.painter(), body.center() - vec2(0.0, 40.0), Align2::CENTER_CENTER, if app.browser.query.trim().is_empty() { "Nenhum cartão ainda. Adicione notas para começar." } else { "Nenhum cartão corresponde à busca." }, serif(14.0, W::Italic), p.muted, body.width());
        return;
    }
    let mut clicked: Option<(usize, Modifiers, bool)> = None;
    egui::ScrollArea::vertical().id_salt("browser_rows").auto_shrink([false, false]).show_rows(&mut bu, 38.0, n, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let row = &app.browser.rows[i];
            let (r, resp) = ui.allocate_exact_size(vec2(rect.width(), 38.0), Sense::click());
            let sel = app.browser.selected.contains(&row.card.id);
            if sel {
                ui.painter().rect_filled(r, 0, p.accent_soft);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0, p.hover);
            }
            ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_row));
            let cy = r.center().y;
            let suspended = row.card.queue == Queue::Suspended;
            let tc = if suspended { p.dim } else { p.text };
            for (c, x, w) in &xs {
                let (s, color, right): (String, Color32, bool) = match c {
                    Col::Front => {
                        let mut s = row.front.clone();
                        if row.card.flagged {
                            s = format!("★ {s}");
                        }
                        (s, tc, false)
                    }
                    Col::Deck => (row.deck.clone(), p.muted, false),
                    Col::Due => (due_label(app, &row.card), p.text2, false),
                    Col::Ivl => (if row.card.s.state != State::Review || row.card.s.interval < 1.0 { "—".into() } else { compact_days(row.card.s.interval) }, p.text2, true),
                    Col::D => (row.card.s.difficulty.map(|d| srs::decimal(d as f64, 1)).unwrap_or_else(|| "—".into()), p.text2, true),
                    Col::S => (row.card.s.stability.map(|s| srs::decimal(s as f64, 1)).unwrap_or_else(|| "—".into()), p.text2, true),
                    Col::R => (row.r.map(|r| pct(r, 0)).unwrap_or_else(|| "—".into()), row.r.map(r_color).unwrap_or(p.dim), true),
                    Col::Lapses => (row.card.s.lapses.to_string(), p.muted, true),
                    Col::Reps => (row.card.s.reps.to_string(), p.muted, true),
                    Col::Kind => (row.kind.clone(), p.muted, false),
                    Col::Added => (short_date(row.card.created_at), p.muted, false),
                };
                let font = if *c == Col::R { serif(13.0, W::Semi) } else if *c == Col::Front { serif(13.0, W::Regular) } else { serif(12.5, W::Regular) };
                if right {
                    text(ui.painter(), pos2(x + w, cy), Align2::RIGHT_CENTER, &s, font, color, *w + 6.0);
                } else {
                    text(ui.painter(), pos2(*x, cy), Align2::LEFT_CENTER, &s, font, color, *w);
                }
            }
            if resp.clicked() {
                clicked = Some((i, ui.ctx().input(|inp| inp.modifiers), false));
            }
            if resp.double_clicked() {
                clicked = Some((i, Modifiers::NONE, true));
            }
            resp.context_menu(|ui| {
                if !app.browser.selected.contains(&app.browser.rows[i].card.id) {
                    app.browser.selected = vec![app.browser.rows[i].card.id];
                }
                bulk_menu(app, ui);
            });
        }
    });
    if let Some((i, m, double)) = clicked {
        let id = app.browser.rows[i].card.id;
        if m.command {
            if let Some(pos) = app.browser.selected.iter().position(|x| *x == id) {
                app.browser.selected.remove(pos);
            } else {
                app.browser.selected.push(id);
            }
            app.browser.anchor = Some(i);
        } else if m.shift {
            let a = app.browser.anchor.unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            app.browser.selected = app.browser.rows[lo..=hi].iter().map(|r| r.card.id).collect();
        } else {
            app.browser.selected = vec![id];
            app.browser.anchor = Some(i);
        }
        if double {
            let note_id = app.browser.rows[i].note_id;
            let deck = app.browser.rows[i].card.deck_id;
            if let Ok(Some(n)) = app.db.note(note_id) {
                app.note_editor = Some(super::note_editor::NoteEditor::edit(n, deck));
            }
        }
    }
}

fn selected_notes(app: &App) -> Vec<i64> {
    let mut v: Vec<i64> = app.browser.rows.iter().filter(|r| app.browser.selected.contains(&r.card.id)).map(|r| r.note_id).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn bulk_menu(app: &mut App, ui: &mut Ui) {
    let ids = app.browser.selected.clone();
    let n = ids.len();
    if menu_item(ui, &format!("Estudar {} {}", n, if n == 1 { "cartão" } else { "cartões" }), false).clicked() {
        review::start(app, review::Scope::Cards(ids.clone(), "Seleção do navegador".into()));
    }
    menu_sep(ui);
    if menu_item(ui, "Mover para baralho…", false).clicked() {
        let deck = app.browser.rows.iter().find(|r| r.card.id == ids[0]).map(|r| r.card.deck_id).unwrap_or(0);
        app.modal = Some(Modal::MoveCards { ids: ids.clone(), deck });
    }
    if menu_item(ui, "Adicionar ou remover tags…", false).clicked() {
        app.modal = Some(Modal::AddTags { note_ids: selected_notes(app), tags: String::new() });
    }
    let all_susp = app.browser.rows.iter().filter(|r| ids.contains(&r.card.id)).all(|r| r.card.queue == Queue::Suspended);
    if menu_item(ui, if all_susp { "Reativar" } else { "Suspender" }, false).clicked() {
        let _ = app.db.set_queue(&ids, if all_susp { Queue::Normal } else { Queue::Suspended });
        app.changed();
    }
    let all_flag = app.browser.rows.iter().filter(|r| ids.contains(&r.card.id)).all(|r| r.card.flagged);
    if menu_item(ui, if all_flag { "Desmarcar" } else { "Marcar" }, false).clicked() {
        let _ = app.db.set_flag(&ids, !all_flag);
        app.changed();
    }
    if menu_item(ui, "Redefinir como novos", false).clicked() {
        let _ = app.db.reset_cards(&ids);
        app.changed();
        app.toast("Cartões redefinidos (o histórico continua no registro)");
    }
    menu_sep(ui);
    if menu_item(ui, "Excluir notas…", false).clicked() {
        app.modal = Some(Modal::DeleteNotes(selected_notes(app)));
    }
}

fn side_panel(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let sel = app.browser.selected.clone();
    if sel.is_empty() {
        ui.add_space(30.0);
        para(ui, "Selecione um cartão para ver dificuldade, estabilidade, a curva de recuperabilidade e o histórico de revisões.", serif(13.0, W::Italic), p.muted);
        ui.add_space(10.0);
        para(ui, "Ctrl+clique e Shift+clique selecionam vários; clique com o botão direito para ações em lote.", serif(12.0, W::Regular), p.dim);
        return;
    }
    if sel.len() > 1 {
        ui.spacing_mut().item_spacing.y = 10.0;
        caps_label(ui, "Seleção");
        label(ui, &format!("{} cartões selecionados", sel.len()), serif(17.0, W::Semi), p.text);
        let w = ui.available_width();
        if button_sized(ui, "Estudar seleção", None, Btn::Primary, vec2(w, 34.0), true).clicked() {
            review::start(app, review::Scope::Cards(sel.clone(), "Seleção do navegador".into()));
        }
        let more = button_sized(ui, "Ações em lote ▾", None, Btn::Secondary, vec2(w, 34.0), true);
        egui::Popup::menu(&more).frame(popup_frame()).width(w).show(|ui| bulk_menu(app, ui));
        return;
    }
    let Some(card) = app.browser.rows.iter().find(|r| r.card.id == sel[0]).map(|r| r.card.clone()) else { return };
    egui::ScrollArea::vertical().id_salt("browser_panel").auto_shrink([false, false]).show(ui, |ui| card_details(app, ui, &card, true));
}

/// Detalhes de um cartão: usado no painel do navegador e no diálogo "Info".
pub fn card_details(app: &mut App, ui: &mut Ui, card: &Card, actions: bool) {
    let p = pal();
    let Ok(Some(note)) = app.db.note(card.note_id) else { return };
    let face = cards::render(note.kind, &note.fields, card.ord);
    let w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 14.0;
    let deck = app.deck(card.deck_id).map(|d| d.name.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(w - 70.0, 20.0), Sense::hover());
        caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &format!("{} · {deck}", note.kind.label()), p.dim);
        if button(ui, "Editar", Btn::Ghost, 26.0).clicked() {
            app.note_editor = Some(super::note_editor::NoteEditor::edit(note.clone(), card.deck_id));
            app.modal = None;
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let q = rich::plain(&face.question);
        para(ui, if q.trim().is_empty() { note.kind.label() } else { &q }, serif(15.0, W::Medium), p.text);
        let a = face.cloze_back.as_deref().map(rich::plain).unwrap_or_else(|| rich::plain(&face.answer));
        if !a.trim().is_empty() {
            para(ui, &a, serif(13.5, W::Regular), p.muted);
        }
    });
    let now = srs::now_ms();
    let decay = app.decay_of_deck(card.deck_id);
    let r_now = srs::retrievability(&card.s, decay, &app.clock, now);
    // Três mini-cards.
    let (row, _) = ui.allocate_exact_size(vec2(w, 52.0), Sense::hover());
    let bw = (w - 16.0) / 3.0;
    for (i, (lab, val, col)) in [
        ("Dificuldade", card.s.difficulty.map(|d| srs::decimal(d as f64, 1)).unwrap_or_else(|| "—".into()), p.text),
        ("Estabilidade", card.s.stability.map(|s| srs::days_label(s as f64)).unwrap_or_else(|| "—".into()), p.text),
        ("R agora", r_now.map(|r| pct(r, 0)).unwrap_or_else(|| "—".into()), r_now.map(r_color).unwrap_or(p.text)),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::from_min_size(pos2(row.left() + i as f32 * (bw + 8.0), row.top()), vec2(bw, 52.0));
        ui.painter().rect(r, CornerRadius::same(8), p.window, Stroke::new(1.0, p.border), StrokeKind::Inside);
        caps_sized(ui.painter(), pos2(r.left() + 10.0, r.top() + 15.0), Align2::LEFT_CENTER, lab, 9.5, p.dim);
        text(ui.painter(), pos2(r.left() + 10.0, r.top() + 35.0), Align2::LEFT_CENTER, &val, serif(17.0, W::Semi), col, bw - 14.0);
    }
    // Curva de R.
    let hist = app.db.card_history(card.id).unwrap_or_default();
    let desired = app.preset_of_deck(card.deck_id).cfg.desired_retention;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
        text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, "Recuperabilidade prevista", serif(12.0, W::Regular), p.muted, w * 0.7);
        text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &format!("meta {}", pct(desired, 0)), serif(12.0, W::Regular), p.muted, w * 0.3);
        let (chart, _) = ui.allocate_exact_size(vec2(w, 92.0), Sense::hover());
        r_curve(ui, chart, card, &hist, decay, desired);
    });
    // Histórico.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        caps_label(ui, "Histórico");
        ui.add_space(6.0);
        if hist.is_empty() {
            label(ui, "Ainda não revisado.", serif(12.5, W::Italic), p.muted);
        }
        for h in hist.iter().rev().take(12) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
            ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line_row));
            let (lab, col) = match h.rating {
                1 => ("Errei", p.again),
                2 => ("Difícil", p.hard),
                3 => ("Bom", p.good),
                _ => ("Fácil", p.easy),
            };
            text(ui.painter(), pos2(r.left(), r.center().y), Align2::LEFT_CENTER, &short_date(h.at), serif(12.5, W::Regular), p.muted, 66.0);
            text(ui.painter(), pos2(r.left() + 72.0, r.center().y), Align2::LEFT_CENTER, lab, serif(12.5, W::Semi), col, 70.0);
            text(ui.painter(), pos2(r.left() + 146.0, r.center().y), Align2::LEFT_CENTER, &srs::interval_label(h.next_due - h.at), serif(12.5, W::Regular), p.text2, 80.0);
            let rr = h.retrievability.map(|v| pct(v, 0)).unwrap_or_else(|| "—".into());
            text(ui.painter(), pos2(r.right(), r.center().y), Align2::RIGHT_CENTER, &rr, serif(12.5, W::Regular), p.muted, 50.0);
        }
    });
    // Datas.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let due = if card.s.state == State::New { "—".into() } else { format!("{} ({})", short_date(card.s.due), due_label(app, card)) };
        for (k, v) in [
            ("Criado", short_date(card.created_at)),
            ("Vence", due),
            ("Revisões", format!("{} · {} lapsos", card.s.reps, card.s.lapses)),
            ("Tempo total", format!("{} s", hist.iter().map(|h| h.duration_ms).sum::<i64>() / 1000)),
            ("Tags", if note.tags.is_empty() { "—".into() } else { note.tags.clone() }),
        ] {
            let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, k, serif(12.0, W::Regular), p.muted, 90.0);
            text(ui.painter(), pos2(r.left() + 96.0, r.center().y), Align2::LEFT_CENTER, &v, serif(12.0, W::Regular), p.text2, w - 96.0);
        }
    });
    if actions {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let bw = (w - 12.0) / 3.0;
            let susp = card.queue == Queue::Suspended;
            if button_sized(ui, if susp { "Reativar" } else { "Suspender" }, None, Btn::Secondary, vec2(bw, 30.0), true).clicked() {
                let _ = app.db.set_queue(&[card.id], if susp { Queue::Normal } else { Queue::Suspended });
                app.changed();
            }
            if button_sized(ui, "Redefinir", None, Btn::Secondary, vec2(bw, 30.0), true).clicked() {
                let _ = app.db.reset_cards(&[card.id]);
                app.changed();
            }
            if button_sized(ui, "Ver origem", None, Btn::Secondary, vec2(bw, 30.0), note.source_page.is_some()).clicked() {
                if let Some(pid) = note.source_page {
                    app.open_page(pid);
                }
            }
        });
    }
}

/// Curva de R desde a primeira revisão até o próximo vencimento: contínua no
/// passado, tracejada no futuro, com as revisões marcadas pela nota dada.
fn r_curve(ui: &Ui, rect: Rect, card: &Card, hist: &[crate::db::Rev], decay: f32, desired: f32) {
    let p = pal();
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, p.border));
    if hist.is_empty() || card.s.stability.is_none() {
        text(painter, rect.center(), Align2::CENTER_CENTER, "Sem revisões ainda", serif(12.0, W::Italic), p.dim, rect.width());
        return;
    }
    let now = srs::now_ms();
    let t0 = hist[0].at;
    let t1 = card.s.due.max(now + DAY_MS);
    let span = (t1 - t0).max(DAY_MS) as f32;
    let y_min = 0.5f32;
    let x_of = |t: i64| rect.left() + (t - t0) as f32 / span * rect.width();
    let y_of = |r: f32| rect.bottom() - 2.0 - ((r - y_min) / (1.0 - y_min)).clamp(0.0, 1.0) * (rect.height() - 6.0);
    super::charts::dashed_h(painter, rect.left(), rect.right(), y_of(desired), p.accent.gamma_multiply(0.7));
    for (i, h) in hist.iter().enumerate() {
        let Some(s) = h.s_after.filter(|s| *s > 0.0) else { continue };
        let start = h.at;
        let end = hist.get(i + 1).map(|n| n.at).unwrap_or(t1);
        let mem = fsrs::MemoryState { stability: s, difficulty: h.d_after.unwrap_or(5.0) };
        let steps = 24;
        let mut past = Vec::new();
        let mut future = Vec::new();
        for k in 0..=steps {
            let t = start + (end - start) * k / steps;
            let r = fsrs::current_retrievability(mem, (t - start) as f32 / DAY_MS as f32, decay);
            let pt = pos2(x_of(t), y_of(r));
            if t <= now {
                past.push(pt);
            } else {
                if future.is_empty() {
                    if let Some(l) = past.last() {
                        future.push(*l);
                    }
                }
                future.push(pt);
            }
        }
        if past.len() >= 2 {
            painter.add(egui::Shape::line(past, Stroke::new(1.6, p.text2)));
        }
        if future.len() >= 2 {
            painter.extend(egui::Shape::dashed_line(&future, Stroke::new(1.6, p.text2), 3.0, 3.0));
        }
        let col = match h.rating {
            1 => p.again,
            2 => p.hard,
            3 => p.good,
            _ => p.easy,
        };
        let before = h.retrievability.unwrap_or(1.0);
        painter.circle_filled(pos2(x_of(h.at), y_of(before)), 3.0, col);
    }
    let xn = x_of(now);
    if xn < rect.right() - 20.0 {
        super::charts::dashed_v(painter, xn, rect.top(), rect.bottom(), p.border_strong);
        text(painter, pos2(xn + 4.0, rect.top() + 6.0), Align2::LEFT_CENTER, "hoje", serif(10.0, W::Regular), p.muted, 40.0);
    }
}
