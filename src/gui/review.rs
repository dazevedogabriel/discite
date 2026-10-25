//! Sessão de revisão (Revisao.dc.html).
//!
//! A fila é montada como no Anki: revisões vencidas (ordenadas pelo preset,
//! por padrão a menor recuperabilidade primeiro) e novos intercalados, ambos
//! dentro dos limites diários; cartões em aprendizado entram quando vencem
//! (ou até 20 min antes, se não houver mais nada).

use super::richtext::{self, Look};
use super::theme::*;
use super::widgets::*;
use super::{App, Screen};
use crate::{tr, tr_n};
use crate::cards::{self, Diff, Face, NoteKind};
use crate::db::{self, Card, Note, Rev};
use crate::srs::{self, Outcome, Queue, ReviewOrder, State};
use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    All,
    Subject(i64),
    Deck(i64),
    /// Lista explícita (estudo personalizado, "Estudar estes 3").
    Cards(Vec<i64>, String),
}

pub struct Current {
    pub card: Card,
    pub note: Note,
    face: Face,
    shown: Instant,
    revealed: bool,
    preview: [Outcome; 4],
    /// Instante em que a pré-visualização foi calculada (base dos rótulos).
    preview_at: i64,
    early: bool,
    data_version: u64,
}

struct Undo {
    before: Card,
    rev_id: i64,
    buried: Vec<i64>,
    old_tags: Option<String>,
}

pub struct Session {
    pub scope: Scope,
    title: String,
    color: Color32,
    deck_ids: Vec<i64>,
    queue: VecDeque<i64>,
    pub current: Option<Current>,
    undo: Vec<Undo>,
    pub done: u32,
    again: u32,
    time_ms: i64,
    due_map: HashMap<i64, usize>,
    started: Instant,
    pub finished: bool,
    /// Próximo cartão em aprendizado (quando só falta esperar).
    waiting_until: Option<i64>,
    explicit: Option<Vec<i64>>,
    counts_cache: Option<(u64, usize, Instant, (usize, usize, usize))>,
}

impl Session {
    pub fn remaining(&self) -> usize {
        self.queue.len() + self.current.is_some() as usize
    }
}

fn scope_decks(app: &App, scope: &Scope) -> Vec<i64> {
    match scope {
        Scope::All => app.decks.iter().map(|d| d.id).collect(),
        Scope::Subject(s) => app.subject_decks(*s),
        Scope::Deck(d) => db::subtree(&app.decks, *d),
        Scope::Cards(..) => app.decks.iter().map(|d| d.id).collect(),
    }
}

fn ids(v: &[i64]) -> String {
    v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

pub fn start(app: &mut App, scope: Scope) {
    if let Some(p) = app.page.as_mut() {
        p.flush(&app.db);
    }
    let deck_ids = scope_decks(app, &scope);
    let now = srs::now_ms();
    let day_end = app.clock.today_end();
    let mut queue: VecDeque<i64> = VecDeque::new();
    let (title, color);
    let explicit;
    match &scope {
        Scope::Cards(list, label) => {
            let cards = app.db.cards_by_ids(list).unwrap_or_default();
            let mut ok: Vec<Card> = cards.into_iter().filter(|c| c.queue != Queue::Suspended).collect();
            ok.sort_by_key(|c| list.iter().position(|x| *x == c.id));
            queue.extend(ok.iter().filter(|c| !matches!(c.s.state, State::Learning | State::Relearning) || c.s.due > now).map(|c| c.id));
            explicit = Some(ok.iter().map(|c| c.id).collect::<Vec<_>>());
            title = label.clone();
            color = ok.first().map(|c| app.subject_color_of_deck(c.deck_id)).unwrap_or(pal().accent);
        }
        _ => {
            explicit = None;
            let set: std::collections::HashSet<i64> = deck_ids.iter().copied().collect();
            let roots: Vec<i64> = app.decks.iter().filter(|d| set.contains(&d.id) && d.parent_id.is_none_or(|p| !set.contains(&p))).map(|d| d.id).collect();
            let mut reviews: Vec<Card> = Vec::new();
            let mut news: Vec<Card> = Vec::new();
            let first_cfg = roots.first().map(|r| app.preset_of_deck(*r).cfg.clone()).unwrap_or_default();
            for r in &roots {
                let lim = app.deck_counts(*r);
                let sub = db::subtree(&app.decks, *r);
                let cfg = app.preset_of_deck(*r).cfg.clone();
                let mut rv = app.db.cards_where(&format!("c.deck_id IN ({}) AND c.queue = 0 AND c.state = 2 AND c.due < {day_end}", ids(&sub))).unwrap_or_default();
                sort_reviews(app, &mut rv, cfg.review_order);
                rv.truncate(lim.review as usize);
                reviews.extend(rv);
                let order = if cfg.new_order == srs::NewOrder::Random { "random()" } else { "c.position, c.id" };
                let nw = app.db.cards_where(&format!("c.deck_id IN ({}) AND c.queue = 0 AND c.state = 0 ORDER BY {order} LIMIT {}", ids(&sub), lim.new)).unwrap_or_default();
                news.extend(nw);
            }
            if roots.len() > 1 {
                sort_reviews(app, &mut reviews, first_cfg.review_order);
            }
            // Irmãos de uma mesma nota não entram juntos na fila do dia.
            if first_cfg.bury_siblings {
                let mut seen = std::collections::HashSet::new();
                reviews.retain(|c| seen.insert(c.note_id));
                news.retain(|c| seen.insert(c.note_id));
            }
            let r: Vec<i64> = reviews.iter().map(|c| c.id).collect();
            let n: Vec<i64> = news.iter().map(|c| c.id).collect();
            queue = mix(r, n, first_cfg.new_mix);
            (title, color) = match &scope {
                Scope::All => (tr!("review.all_subjects"), pal().accent),
                Scope::Subject(s) => (app.subject(*s).map(|s| s.name.clone()).unwrap_or_default(), app.subject(*s).map(|s| subject_color(s.color)).unwrap_or(pal().accent)),
                Scope::Deck(d) => (app.deck_path(*d), app.subject_color_of_deck(*d)),
                Scope::Cards(..) => unreachable!(),
            };
        }
    }
    let due_map = app.db.due_per_day(&app.clock).unwrap_or_default();
    let mut s = Session {
        scope,
        title,
        color,
        deck_ids,
        queue,
        current: None,
        undo: vec![],
        done: 0,
        again: 0,
        time_ms: 0,
        due_map,
        started: Instant::now(),
        finished: false,
        waiting_until: None,
        explicit,
        counts_cache: None,
    };
    next_card(app, &mut s, false);
    if s.current.is_none() && s.waiting_until.is_none() {
        app.toast(tr!("toast.nothing_to_study"));
        return;
    }
    if app.screen != Screen::Review {
        app.back_screen = app.screen;
    }
    app.session = Some(s);
    app.go(Screen::Review);
}

fn sort_reviews(app: &App, v: &mut [Card], order: ReviewOrder) {
    let now = srs::now_ms();
    match order {
        ReviewOrder::Retrievability => {
            let r = |c: &Card| srs::retrievability(&c.s, app.decay_of_deck(c.deck_id), &app.clock, now).unwrap_or(1.0);
            v.sort_by(|a, b| r(a).partial_cmp(&r(b)).unwrap_or(std::cmp::Ordering::Equal));
        }
        ReviewOrder::Due => v.sort_by_key(|c| c.s.due),
        ReviewOrder::Random => {
            use rand::seq::SliceRandom;
            v.shuffle(&mut rand::rng());
        }
        ReviewOrder::Deck => v.sort_by_key(|c| (c.deck_id, c.s.due)),
    }
}

fn mix(reviews: Vec<i64>, news: Vec<i64>, how: srs::NewMix) -> VecDeque<i64> {
    match how {
        srs::NewMix::Before => news.into_iter().chain(reviews).collect(),
        srs::NewMix::After => reviews.into_iter().chain(news).collect(),
        srs::NewMix::Mixed => {
            if news.is_empty() || reviews.is_empty() {
                return reviews.into_iter().chain(news).collect();
            }
            let every = (reviews.len() as f32 / news.len() as f32).max(1.0);
            let mut out = VecDeque::new();
            let mut ni = news.into_iter().peekable();
            let mut next_at = every / 2.0;
            for (i, r) in reviews.into_iter().enumerate() {
                if (i as f32) >= next_at {
                    if let Some(n) = ni.next() {
                        out.push_back(n);
                    }
                    next_at += every;
                }
                out.push_back(r);
            }
            out.extend(ni);
            out
        }
    }
}

/// Escolhe e prepara o próximo cartão.
fn next_card(app: &App, s: &mut Session, ahead: bool) {
    s.current = None;
    s.waiting_until = None;
    let now = srs::now_ms();
    let scope_sql = match &s.explicit {
        Some(list) if !list.is_empty() => format!("c.id IN ({})", ids(list)),
        Some(_) => "0".into(),
        None => format!("c.deck_id IN ({})", ids(&s.deck_ids)),
    };
    let learn_due = |until: i64| app.db.cards_where(&format!("{scope_sql} AND c.queue = 0 AND c.state IN (1, 3) AND c.due <= {until} ORDER BY c.due LIMIT 1")).ok().and_then(|v| v.into_iter().next());
    let mut pick = learn_due(now);
    if pick.is_none() {
        while let Some(id) = s.queue.pop_front() {
            if let Ok(Some(c)) = app.db.card(id) {
                if c.queue == Queue::Normal {
                    pick = Some(c);
                    break;
                }
            }
        }
    }
    if pick.is_none() {
        pick = learn_due(now + if ahead { i64::MAX / 4 } else { srs::LEARN_AHEAD_MS });
    }
    let Some(card) = pick else {
        // Ainda há cartões em aprendizado para mais tarde, hoje?
        let later = app.db.cards_where(&format!("{scope_sql} AND c.queue = 0 AND c.state IN (1, 3) AND c.due < {} ORDER BY c.due LIMIT 1", app.clock.today_end())).ok().and_then(|v| v.into_iter().next());
        match later {
            Some(c) => s.waiting_until = Some(c.s.due),
            None => s.finished = true,
        }
        return;
    };
    let Ok(Some(note)) = app.db.note(card.note_id) else {
        s.finished = true;
        return;
    };
    let face = cards::render(note.kind, &note.fields, card.ord);
    let early = card.s.state == State::Review && card.s.due >= app.clock.today_end();
    let preview = preview_for(app, s, &card, early);
    s.current = Some(Current { card, note, face, shown: Instant::now(), revealed: false, preview, preview_at: srs::now_ms(), early, data_version: app.data_version });
}

fn preview_for(app: &App, s: &Session, card: &Card, early: bool) -> [Outcome; 4] {
    let preset = app.preset_of_deck(card.deck_id);
    let (f, decay) = srs::fsrs_for(&preset.params);
    let map = &s.due_map;
    let due = |d: i64| map.get(&d).copied().unwrap_or(0);
    let ctx = srs::Ctx { cfg: &preset.cfg, fsrs: &f, decay, clock: app.clock, now: srs::now_ms(), seed: card.id as u64 ^ (card.s.reps as u64) << 48, due_on_day: &due, filtered: early };
    srs::preview(&card.s, &ctx)
}

fn answer(app: &mut App, rating: u32) {
    let Some(mut s) = app.session.take() else { return };
    let Some(cur) = s.current.take() else {
        app.session = Some(s);
        return;
    };
    // Recalcula com o relógio de agora (a pré-visualização pode ter minutos).
    let out = preview_for(app, &s, &cur.card, cur.early)[(rating - 1) as usize].clone();
    let preset = app.preset_of_deck(cur.card.deck_id).clone();
    let before = cur.card.clone();
    let mut card = cur.card.clone();
    card.s = out.sched.clone();
    let mut old_tags = None;
    if out.became_leech {
        card.leech = true;
        if preset.cfg.leech_action == srs::LeechAction::Suspend {
            card.queue = Queue::Suspended;
        }
        let mut note = cur.note.clone();
        let leech_tag = tr!("review.leech_tag");
        if !note.tags.split_whitespace().any(|t| t == leech_tag) {
            old_tags = Some(note.tags.clone());
            note.tags = format!("{} {leech_tag}", note.tags);
            let _ = app.db.update_note(&mut note, card.deck_id);
        }
        app.toast(tr!("toast.leech"));
    }
    let duration = (cur.shown.elapsed().as_millis() as i64).min(app.cfg.answer_cap_secs.max(5) as i64 * 1000);
    let now = srs::now_ms();
    let rev = Rev {
        id: 0,
        card_id: card.id,
        at: now,
        rating,
        kind: out.kind,
        elapsed_days: out.elapsed_days,
        scheduled_days: out.scheduled_days,
        duration_ms: duration,
        state_before: before.s.state,
        state_after: card.s.state,
        s_before: before.s.stability,
        s_after: card.s.stability,
        d_before: before.s.difficulty,
        d_after: card.s.difficulty,
        retrievability: out.retrievability,
        next_due: card.s.due,
        algorithm: if preset.cfg.algorithm == srs::Algorithm::Fsrs { "fsrs-6".into() } else { "sm2".into() },
        preset_id: preset.id,
    };
    let result = (|| -> anyhow::Result<(i64, Vec<i64>)> {
        app.db.save_card(&card)?;
        let rev_id = app.db.add_rev(&rev)?;
        let buried = if preset.cfg.bury_siblings { app.db.bury_siblings(&card)? } else { vec![] };
        Ok((rev_id, buried))
    })();
    match result {
        Ok((rev_id, buried)) => {
            s.queue.retain(|id| !buried.contains(id));
            if before.s.state == State::Review {
                if let Some(n) = s.due_map.get_mut(&app.clock.day_of(before.s.due)) {
                    *n = n.saturating_sub(1);
                }
            }
            if card.s.state == State::Review {
                *s.due_map.entry(app.clock.day_of(card.s.due)).or_insert(0) += 1;
            }
            s.undo.push(Undo { before, rev_id, buried, old_tags });
            s.done += 1;
            s.again += (rating == 1) as u32;
            s.time_ms += duration;
        }
        Err(e) => app.error(e),
    }
    app.changed();
    next_card(app, &mut s, false);
    app.session = Some(s);
}

fn undo(app: &mut App) {
    let Some(mut s) = app.session.take() else { return };
    let Some(u) = s.undo.pop() else {
        app.session = Some(s);
        app.toast(tr!("toast.nothing_to_undo"));
        return;
    };
    let r = (|| -> anyhow::Result<()> {
        app.db.save_card(&u.before)?;
        app.db.undo_rev(u.rev_id)?;
        app.db.set_queue(&u.buried, Queue::Normal)?;
        if let Some(tags) = u.old_tags {
            if let Some(mut n) = app.db.note(u.before.note_id)? {
                n.tags = tags;
                app.db.update_note(&mut n, u.before.deck_id)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = r {
        app.error(e);
    }
    if let Some(c) = s.current.take() {
        s.queue.push_front(c.card.id);
    }
    s.queue.extend(u.buried.iter().copied());
    s.done = s.done.saturating_sub(1);
    s.finished = false;
    s.waiting_until = None;
    // Mostra de novo o cartão desfeito.
    if let Ok(Some(note)) = app.db.note(u.before.note_id) {
        let face = cards::render(note.kind, &note.fields, u.before.ord);
        let early = u.before.s.state == State::Review && u.before.s.due >= app.clock.today_end();
        let preview = preview_for(app, &s, &u.before, early);
        s.current = Some(Current { card: u.before, note, face, shown: Instant::now(), revealed: false, preview, preview_at: srs::now_ms(), early, data_version: app.data_version });
    }
    app.changed();
    app.session = Some(s);
    app.toast(tr!("toast.answer_undone"));
}

fn set_current_queue(app: &mut App, q: Queue, msg: &str) {
    let Some(mut s) = app.session.take() else { return };
    if let Some(c) = &s.current {
        if let Err(e) = app.db.set_queue(&[c.card.id], q) {
            app.error(e);
        }
        app.toast(msg);
    }
    app.changed();
    next_card(app, &mut s, false);
    app.session = Some(s);
}

pub fn exit(app: &mut App) {
    if let Some(s) = app.session.take() {
        if s.done > 0 {
            app.toast(tr_n!("toast.session_ended", s.done, pct = pct(1.0 - s.again as f32 / s.done.max(1) as f32, 0)));
        }
    }
    let back = match app.back_screen {
        Screen::Review => Screen::Dashboard,
        b => b,
    };
    app.go(back);
}

pub fn ui(app: &mut App, ui: &mut Ui) {
    let p = pal();
    if app.session.is_none() {
        app.go(Screen::Dashboard);
        return;
    }
    // A nota foi editada (E)? Recarrega o rosto do cartão.
    let dv = app.data_version;
    if let Some(cur) = app.session.as_mut().and_then(|s| s.current.as_mut()) {
        if cur.data_version != dv {
            cur.data_version = dv;
            if let Ok(Some(n)) = app.db.note(cur.note.id) {
                cur.face = cards::render(n.kind, &n.fields, cur.card.ord);
                cur.note = n;
            }
            if let Ok(Some(c)) = app.db.card(cur.card.id) {
                cur.card.flagged = c.flagged;
                cur.card.deck_id = c.deck_id;
            }
        }
    }
    // Esperando um cartão em aprendizado que já venceu?
    if let Some(s) = app.session.as_mut() {
        if s.waiting_until.is_some_and(|t| t <= srs::now_ms()) || (s.finished && s.current.is_none() && s.waiting_until.is_none() && false) {
            let mut taken = app.session.take().unwrap();
            next_card(app, &mut taken, false);
            app.session = Some(taken);
        }
    }

    keyboard(app, ui);
    if app.session.is_none() {
        return;
    }
    let area = ui.max_rect();

    // ── Barra superior ──
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 60.0));
    let (title, color, done, remaining, cur_state) = {
        let s = app.session.as_ref().unwrap();
        (s.title.clone(), s.color, s.done, s.remaining(), s.current.as_ref().map(|c| c.card.s.state))
    };
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(bar.min + vec2(24.0, 0.0), pos2(bar.center().x - 180.0, bar.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if button_kbd(&mut lu, &tr!("review.exit"), Some("Esc"), Btn::Ghost, 34.0).clicked() {
        exit(app);
        return;
    }
    lu.add_space(6.0);
    let (dr, _) = lu.allocate_exact_size(vec2(lu.available_width(), 20.0), Sense::hover());
    lu.painter().circle_filled(pos2(dr.left() + 4.0, dr.center().y), 4.0, color);
    let parts: Vec<&str> = title.split(" › ").collect();
    let mut x = dr.left() + 15.0;
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let r = text(lu.painter(), pos2(x, dr.center().y), Align2::LEFT_CENTER, part, serif(13.0, W::Regular), if last { p.text2 } else { p.muted }, (dr.right() - x).max(10.0));
        x = r.right() + 6.0;
        if !last {
            let r = text(lu.painter(), pos2(x, dr.center().y), Align2::LEFT_CENTER, "›", serif(13.0, W::Regular), p.dim, 20.0);
            x = r.right() + 6.0;
        }
    }

    // Contadores.
    let (n_new, n_learn, n_rev) = counts_left(app);
    let center = bar.center();
    let items = [(n_new, tr!("dash.new"), p.easy, State::New), (n_learn, tr!("dash.learning"), p.hard, State::Learning), (n_rev, tr!("dash.review"), p.good, State::Review)];
    let widths: Vec<f32> = items.iter().map(|(n, l, _, _)| text_width(ui.painter(), &n.to_string(), serif(17.0, W::Semi)) + 6.0 + text_width(ui.painter(), l, serif(13.0, W::Regular))).collect();
    let total_w: f32 = widths.iter().sum::<f32>() + 36.0;
    let mut x = center.x - total_w / 2.0;
    for ((n, l, c, st), w) in items.iter().zip(&widths) {
        let r1 = text(ui.painter(), pos2(x, center.y), Align2::LEFT_CENTER, &n.to_string(), serif(17.0, W::Semi), *c, 60.0);
        text(ui.painter(), pos2(r1.right() + 6.0, center.y + 1.0), Align2::LEFT_CENTER, l, serif(13.0, W::Regular), p.muted, 100.0);
        let active = match cur_state {
            Some(State::New) => *st == State::New,
            Some(State::Learning) | Some(State::Relearning) => *st == State::Learning,
            Some(State::Review) => *st == State::Review,
            None => false,
        };
        if active {
            ui.painter().hline(x..=x + w, center.y + 14.0, Stroke::new(2.0, *c));
        }
        x += w + 18.0;
    }

    // Direita: cronômetro + ações.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(bar.center().x + 180.0, bar.top()), bar.max - vec2(24.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    ru.spacing_mut().item_spacing.x = 6.0;
    let has_cur = app.session.as_ref().is_some_and(|s| s.current.is_some());
    let has_origin = app.session.as_ref().and_then(|s| s.current.as_ref()).is_some_and(|c| c.note.source_page.is_some());
    if has_origin && button_kbd(&mut ru, &tr!("review.origin"), Some("O"), Btn::Ghost, 34.0).clicked() {
        open_origin(app);
    }
    if has_cur && button_kbd(&mut ru, &tr!("review.info"), Some("I"), Btn::Ghost, 34.0).clicked() {
        open_info(app);
    }
    if has_cur && button_kbd(&mut ru, &tr!("review.edit"), Some("E"), Btn::Ghost, 34.0).clicked() {
        open_edit(app);
    }
    if let Some(cur) = app.session.as_ref().and_then(|s| s.current.as_ref()) {
        let secs = cur.shown.elapsed().as_secs().min(app.cfg.answer_cap_secs as u64);
        ru.add_space(8.0);
        let (r, _) = ru.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
        text(ru.painter(), r.right_center(), Align2::RIGHT_CENTER, &format!("{}:{:02}", secs / 60, secs % 60), mono(12.0), p.muted, 50.0);
    }

    // Progresso.
    let prog = Rect::from_min_size(pos2(area.left(), bar.bottom()), vec2(area.width(), 3.0));
    ui.painter().rect_filled(prog, 0, p.line);
    let frac = done as f32 / (done as usize + remaining).max(1) as f32;
    ui.painter().rect_filled(Rect::from_min_size(prog.min, vec2(prog.width() * frac, 3.0)), 0, p.accent);

    let body = Rect::from_min_max(pos2(area.left(), prog.bottom()), area.max);
    let state = app.session.as_ref().map(|s| (s.current.is_some(), s.waiting_until, s.finished)).unwrap();
    match state {
        (true, _, _) => card_view(app, ui, body),
        (false, Some(until), _) => waiting(app, ui, body, until),
        _ => finished(app, ui, body),
    }
}

fn counts_left(app: &mut App) -> (usize, usize, usize) {
    let dv = app.data_version;
    if let Some(s) = &app.session {
        if let Some((v, len, at, c)) = s.counts_cache {
            if v == dv && len == s.queue.len() && at.elapsed().as_secs() < 2 {
                return c;
            }
        }
    }
    let c = counts_left_uncached(app);
    if let Some(s) = app.session.as_mut() {
        s.counts_cache = Some((dv, s.queue.len(), Instant::now(), c));
    }
    c
}

fn counts_left_uncached(app: &App) -> (usize, usize, usize) {
    let Some(s) = &app.session else { return (0, 0, 0) };
    let now = srs::now_ms();
    let cards = app.db.cards_by_ids(&s.queue.iter().copied().collect::<Vec<_>>()).unwrap_or_default();
    let mut n = cards.iter().filter(|c| c.s.state == State::New).count();
    let mut r = cards.len() - n;
    let scope_sql = match &s.explicit {
        Some(list) if !list.is_empty() => format!("c.id IN ({})", ids(list)),
        Some(_) => "0".into(),
        None => format!("c.deck_id IN ({})", ids(&s.deck_ids)),
    };
    let mut l = app
        .db
        .conn
        .query_row(&format!("SELECT COUNT(*) FROM cards c WHERE {scope_sql} AND c.queue = 0 AND c.state IN (1, 3) AND c.due < {}", app.clock.today_end().max(now)), [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as usize;
    if let Some(c) = &s.current {
        match c.card.s.state {
            State::New => n += 1,
            State::Review => r += 1,
            _ => {
                if l == 0 {
                    l = 1;
                }
            }
        }
    }
    (n, l, r)
}

fn keyboard(app: &mut App, ui: &Ui) {
    if app.note_editor.is_some() || app.modal.is_some() || app.palette.is_some() {
        return;
    }
    let ctx = ui.ctx();
    let typing = ctx.memory(|m| m.focused().is_some());
    let (revealed, has_cur, type_in) = app
        .session
        .as_ref()
        .and_then(|s| s.current.as_ref())
        .map(|c| (c.revealed, true, c.face.type_answer.is_some()))
        .unwrap_or((false, false, false));
    let key = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if key(Key::Escape) {
        if typing {
            ctx.memory_mut(|m| m.stop_text_input());
        } else {
            exit(app);
            return;
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
        undo(app);
        return;
    }
    if !has_cur {
        return;
    }
    if !revealed {
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        if enter || (!typing && key(Key::Space)) {
            reveal(app);
        }
        return;
    }
    if type_in && typing {
        ctx.memory_mut(|m| m.stop_text_input());
    }
    if key(Key::Space) || key(Key::Enter) || key(Key::Num3) {
        answer(app, 3);
        return;
    }
    for (k, r) in [(Key::Num1, 1), (Key::Num2, 2), (Key::Num4, 4)] {
        if key(k) {
            answer(app, r);
            return;
        }
    }
    if typing {
        return;
    }
    if key(Key::E) {
        open_edit(app);
    } else if key(Key::I) {
        open_info(app);
    } else if key(Key::O) {
        open_origin(app);
    }
    let texts: Vec<String> = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
    for t in texts {
        match t.as_str() {
            "@" => set_current_queue(app, Queue::Suspended, &tr!("toast.card_suspended")),
            "-" => set_current_queue(app, Queue::BuriedUser, &tr!("toast.card_buried")),
            "*" => toggle_flag(app),
            _ => {}
        }
    }
}

fn reveal(app: &mut App) {
    if let Some(c) = app.session.as_mut().and_then(|s| s.current.as_mut()) {
        c.revealed = true;
    }
}

fn toggle_flag(app: &mut App) {
    let Some(c) = app.session.as_mut().and_then(|s| s.current.as_mut()) else { return };
    c.card.flagged = !c.card.flagged;
    let (id, on) = (c.card.id, c.card.flagged);
    let _ = app.db.set_flag(&[id], on);
    app.toast(tr!(if on { "toast.card_flagged" } else { "toast.card_unflagged" }));
    app.data_version += 1;
}

fn open_edit(app: &mut App) {
    if let Some(c) = app.session.as_ref().and_then(|s| s.current.as_ref()) {
        app.note_editor = Some(super::note_editor::NoteEditor::edit(c.note.clone(), c.card.deck_id));
    }
}

fn open_info(app: &mut App) {
    if let Some(c) = app.session.as_ref().and_then(|s| s.current.as_ref()) {
        app.modal = Some(super::modals::Modal::CardInfo(c.card.id));
    }
}

fn open_origin(app: &mut App) {
    if let Some(page) = app.session.as_ref().and_then(|s| s.current.as_ref()).and_then(|c| c.note.source_page) {
        app.open_page(page);
    }
}

fn card_view(app: &mut App, ui: &mut Ui, body: Rect) {
    let p = pal();
    let footer_h = 150.0;
    let main = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - footer_h));
    let media = app.db.media_dir();
    let card_w = 720.0f32.min(main.width() - 48.0);
    let (kind_label, tags, revealed, face, card, note) = {
        let c = app.session.as_ref().unwrap().current.as_ref().unwrap();
        let st = c.card.s.state.label();
        (format!("{} · {}", c.note.kind.label(), st), c.note.tag_list(), c.revealed, c.face.clone(), c.card.clone(), c.note.clone())
    };
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(main.shrink(24.0)).layout(egui::Layout::top_down(egui::Align::Center)));
    egui::ScrollArea::vertical().id_salt(("card", card.id)).auto_shrink([false, false]).show(&mut mu, |ui| {
        let avail_h = ui.available_height();
        // Centraliza verticalmente usando a altura medida no quadro anterior.
        let h_key = egui::Id::unique(("card_h", card.id, revealed));
        let est = ui.data(|d| d.get_temp::<f32>(h_key)).unwrap_or(360.0);
        ui.vertical_centered(|ui| {
            if avail_h > est + 20.0 {
                ui.add_space(((avail_h - est) / 2.0 - 10.0).max(0.0));
            }
            let frame = egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(16).inner_margin(egui::Margin { left: 56, right: 56, top: 40, bottom: 36 });
            let shown = ui.allocate_ui(vec2(card_w, 0.0), |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(card_w - 112.0);
                    ui.set_min_height(360.0 - 76.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 22.0;
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() * 0.6, 22.0), Sense::hover());
                        caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &kind_label, p.dim);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if card.flagged {
                                chip(ui, &tr!("review.flagged"), true, 22.0);
                            }
                            for t in tags.iter().rev().take(3) {
                                chip(ui, &format!("#{t}"), false, 22.0);
                            }
                        });
                    });
                    let q_look = Look { size: 27.0, semi: false, italic: false, color: p.text, line_height: Some(38.0) };
                    if let Some((img, masks, target)) = &face.occlusion {
                        if !face.question.trim().is_empty() {
                            richtext::field(ui, &face.question, Look { size: 20.0, ..q_look }, &media, 300.0);
                        }
                        occlusion_view(ui, &media, img, masks, *target, revealed);
                    } else if revealed && face.cloze_back.is_some() {
                        richtext::field(ui, face.cloze_back.as_deref().unwrap_or(""), q_look, &media, 360.0);
                    } else {
                        richtext::field(ui, &face.question, q_look, &media, 360.0);
                    }
                    if let Some(expected) = &face.type_answer {
                        type_box(app_typed(ui), ui, expected, revealed);
                    }
                    if revealed {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
                        if face.cloze_back.is_none() && face.occlusion.is_none() && !face.answer.trim().is_empty() {
                            richtext::field(ui, &face.answer, Look { size: 21.0, semi: false, italic: false, color: p.text, line_height: Some(31.0) }, &media, 360.0);
                        }
                        if !face.extra.trim().is_empty() {
                            richtext::field(ui, &face.extra, Look { size: 15.0, semi: false, italic: true, color: p.muted, line_height: Some(23.0) }, &media, 260.0);
                        }
                        let preset = app.preset_of_deck(card.deck_id);
                        let decay = if preset.params.len() >= 21 { preset.params[20] } else { fsrs::FSRS6_DEFAULT_DECAY };
                        let r = srs::retrievability(&card.s, decay, &app.clock, srs::now_ms());
                        let mut meta = Vec::new();
                        if let Some(r) = r {
                            meta.push(tr!("review.predicted_r", pct = pct(r, 0)));
                        }
                        if let Some(s) = card.s.stability {
                            meta.push(format!("S {}", srs::days_label(s as f64)));
                        }
                        if let Some(d) = card.s.difficulty {
                            meta.push(format!("D {}", srs::decimal(d as f64, 1)));
                        }
                        meta.push(if card.s.lapses > 0 { tr_n!("review.lapses", card.s.lapses) } else if card.s.reps == 0 { tr!("review.first_time") } else { tr!("review.nth_review", n = card.s.reps + 1) });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 18.0;
                            for m in meta {
                                label(ui, &m, serif(12.0, W::Regular), p.dim);
                            }
                        });
                    }
                    });
                });
            });
            ui.data_mut(|d| d.insert_temp(h_key, shown.response.rect.height()));
        });
    });
    let _ = note;

    // ── Rodapé ──
    let foot = Rect::from_min_max(pos2(body.left(), body.bottom() - footer_h), body.max);
    let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(24.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    fu.spacing_mut().item_spacing.y = 14.0;
    if !revealed {
        if button_sized(&mut fu, &tr!("review.show_answer"), Some(&tr!("common.key_space")), Btn::Primary, vec2(320.0, 52.0), true).clicked() {
            reveal(app);
        }
        let (r, _) = fu.allocate_exact_size(vec2(400.0, 30.0), Sense::hover());
        text(fu.painter(), r.center(), Align2::CENTER_CENTER, &tr!("review.try_recall"), serif(13.0, W::Italic), p.muted, 400.0);
    } else {
        let (preview, preview_at) = app.session.as_ref().unwrap().current.as_ref().map(|c| (c.preview.clone(), c.preview_at)).unwrap();
        let w = card_w;
        let (row, _) = fu.allocate_exact_size(vec2(w, 72.0), Sense::hover());
        let gap = 10.0;
        let bw = (w - 3.0 * gap) / 4.0;
        let now = preview_at;
        let mut clicked = None;
        for (i, (key, col)) in [("rating.again", p.again), ("rating.hard", p.hard), ("rating.good", p.good), ("rating.easy", p.easy)].into_iter().enumerate() {
            let label_s = &tr!(key);
            let r = Rect::from_min_size(pos2(row.left() + i as f32 * (bw + gap), row.top()), vec2(bw, 72.0));
            let resp = fu.interact(r, fu.scope_id().with(("rate", i)), Sense::click());
            let bg = soft(col, if resp.hovered() { 0.22 } else { 0.13 });
            fu.painter().rect(r, CornerRadius::same(12), bg, Stroke::new(1.0, p.border), StrokeKind::Inside);
            fu.painter().rect_filled(Rect::from_min_size(r.min + vec2(1.0, 0.0), vec2(r.width() - 2.0, 3.0)), CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }, col);
            if i == 2 {
                fu.painter().rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0, col), StrokeKind::Inside);
            }
            let ivl = srs::interval_label(preview[i].delay_ms(now));
            text(fu.painter(), pos2(r.center().x, r.top() + 24.0), Align2::CENTER_CENTER, &ivl, serif(12.5, W::Semi), col, bw);
            let lw = text_width(fu.painter(), label_s, serif(16.0, W::Semi));
            let kw = kbd_width(fu.painter(), "1");
            let x0 = r.center().x - (lw + 8.0 + kw) / 2.0;
            text(fu.painter(), pos2(x0, r.top() + 46.0), Align2::LEFT_CENTER, label_s, serif(16.0, W::Semi), p.text, lw + 4.0);
            kbd(fu.painter(), pos2(x0 + lw + 8.0, r.top() + 46.0), Align2::LEFT_CENTER, &(i + 1).to_string());
            a11y(&resp, egui::accesskit::Role::Button, &tr!("review.rating_next", rating = label_s, interval = ivl), None);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i as u32 + 1);
            }
        }
        if let Some(r) = clicked {
            answer(app, r);
        }
    }
    fu.horizontal(|ui| {
        let w = 4.0 * 110.0;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = 20.0;
        kbd_hint(ui, "Ctrl Z", &tr!("review.hint_undo"));
        kbd_hint(ui, "@", &tr!("review.hint_suspend"));
        kbd_hint(ui, "-", &tr!("review.hint_bury"));
        kbd_hint(ui, "*", &tr!("review.hint_flag"));
    });
}

/// O texto digitado vive na sessão; este atalho devolve um ponteiro para ele.
fn app_typed(ui: &Ui) -> egui::Id {
    ui.scope_id().with("typed")
}

fn type_box(id: egui::Id, ui: &mut Ui, expected: &str, revealed: bool) {
    let p = pal();
    let mut typed: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
    if !revealed {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut typed)
                .font(serif(18.0, W::Regular))
                .hint_text(egui::RichText::new(tr!("review.type_answer")).italics().color(p.dim))
                .desired_width(ui.available_width())
                .margin(vec2(12.0, 10.0)),
        );
        if !resp.has_focus() && typed.is_empty() {
            resp.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(id, typed));
    } else {
        let (a, b) = cards::diff(&typed, expected);
        let mut job = egui::text::LayoutJob::default();
        let fmt = |c: Color32, bg: Color32| egui::TextFormat { font_id: mono(17.0), color: c, background: bg, ..Default::default() };
        if typed.trim().is_empty() {
            job.append(&tr!("review.nothing_typed"), 0.0, egui::TextFormat { font_id: serif(15.0, W::Italic), color: p.muted, ..Default::default() });
        } else {
            for (ch, d) in a {
                let (c, bg) = match d {
                    Diff::Same => (p.good, Color32::TRANSPARENT),
                    _ => (p.again, soft(p.again, 0.15)),
                };
                job.append(&ch.to_string(), 0.0, fmt(c, bg));
            }
        }
        job.append("\n", 0.0, fmt(p.text, Color32::TRANSPARENT));
        for (ch, d) in b {
            let (c, bg) = match d {
                Diff::Same => (p.text, Color32::TRANSPARENT),
                _ => (p.hard, soft(p.hard, 0.15)),
            };
            job.append(&ch.to_string(), 0.0, fmt(c, bg));
        }
        ui.label(job);
        if ui.ctx().input(|i| i.key_pressed(Key::Num1) || i.key_pressed(Key::Num3)) {
            ui.data_mut(|d| d.remove::<String>(id));
        }
    }
}

/// Imagem com as máscaras: na frente todas cobertas (a do cartão em destaque);
/// no verso a do cartão é revelada com contorno.
pub fn occlusion_view(ui: &mut Ui, media: &std::path::Path, img: &str, masks: &[cards::Mask], target: usize, revealed: bool) {
    let p = pal();
    let w = ui.available_width();
    let resp = super::richtext::image(ui, &crate::media::uri(media, img), w, 420.0, 8, Sense::hover());
    let r = resp.rect;
    for (i, m) in masks.iter().enumerate() {
        let mr = Rect::from_min_size(pos2(r.left() + m.x * r.width(), r.top() + m.y * r.height()), vec2(m.w * r.width(), m.h * r.height()));
        if i == target {
            if revealed {
                ui.painter().rect_stroke(mr, CornerRadius::same(4), Stroke::new(2.5, p.accent), StrokeKind::Outside);
            } else {
                ui.painter().rect(mr, CornerRadius::same(4), p.accent, Stroke::new(1.0, p.accent_hover), StrokeKind::Inside);
                text(ui.painter(), mr.center(), Align2::CENTER_CENTER, "?", serif(16.0, W::Semi), p.on_accent, mr.width());
            }
        } else {
            ui.painter().rect(mr, CornerRadius::same(4), if p.dark { Color32::from_rgb(0x2a, 0x2d, 0x35) } else { Color32::from_rgb(0xd8, 0xd2, 0xc2) }, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        }
    }
}

fn waiting(app: &mut App, ui: &mut Ui, body: Rect, until: i64) {
    let p = pal();
    let r = Rect::from_center_size(body.center() - vec2(0.0, 40.0), vec2(520.0, 220.0));
    paint_card(ui.painter(), r);
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(32.0)).layout(egui::Layout::top_down(egui::Align::Center)));
    cu.spacing_mut().item_spacing.y = 14.0;
    caps_label(&mut cu, &tr!("review.break"));
    let mins = ((until - srs::now_ms()) as f64 / 60_000.0).ceil().max(1.0);
    para(&mut cu, &tr!("review.learning_returns", n = mins), serif(20.0, W::Regular), p.text);
    para(&mut cu, &tr!("review.break_tip"), serif(14.0, W::Italic), p.muted);
    cu.horizontal(|ui| {
        if button(ui, &tr!("review.show_now"), Btn::Primary, 36.0).clicked() {
            let mut s = app.session.take().unwrap();
            next_card(app, &mut s, true);
            app.session = Some(s);
        }
        if button(ui, &tr!("review.end"), Btn::Secondary, 36.0).clicked() {
            exit(app);
        }
    });
}

fn finished(app: &mut App, ui: &mut Ui, body: Rect) {
    let p = pal();
    let (done, again, time_ms, elapsed) = app.session.as_ref().map(|s| (s.done, s.again, s.time_ms, s.started.elapsed())).unwrap_or_default();
    let r = Rect::from_center_size(body.center() - vec2(0.0, 40.0), vec2(560.0, 300.0));
    paint_card(ui.painter(), r);
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(32.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    cu.spacing_mut().item_spacing.y = 14.0;
    caps_label(&mut cu, &tr!("review.done_title"));
    para(&mut cu, &tr!(if done > 0 { "review.done_good" } else { "review.done_empty" }), serif(22.0, W::Semi), p.text);
    let tomorrow = crate::stats::forecast(&app.db, &crate::stats::Scope(None), &app.clock, 2).ok().and_then(|v| v.get(1).copied()).unwrap_or(0);
    cu.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        for (v, l) in [
            (done.to_string(), tr!("review.stat_cards")),
            (if done > 0 { pct(1.0 - again as f32 / done as f32, 0) } else { "—".into() }, tr!("review.stat_correct")),
            (tr!("time.minutes", n = (time_ms.max(elapsed.as_millis() as i64 / 4) as f64 / 60_000.0).round()), tr!("review.stat_studying")),
            (tomorrow.to_string(), tr!("review.stat_tomorrow")),
        ] {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                label(ui, &v, serif(26.0, W::Semi), p.text);
                label(ui, &l, serif(12.0, W::Regular), p.muted);
            });
        }
    });
    cu.add_space(6.0);
    cu.horizontal(|ui| {
        if button(ui, &tr!("review.back"), Btn::Primary, 36.0).clicked() {
            exit(app);
        }
        if button(ui, &tr!("review.custom_study"), Btn::Secondary, 36.0).clicked() {
            let deck = match app.session.as_ref().map(|s| s.scope.clone()) {
                Some(Scope::Deck(d)) => Some(d),
                _ => None,
            };
            app.session = None;
            app.go(app.back_screen);
            app.modal = Some(super::modals::Modal::custom_study(deck));
        }
    });
    let _ = NoteKind::Basic;
}
