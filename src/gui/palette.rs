//! Paleta de comandos (Ctrl+K): navegar para páginas, disciplinas e
//! baralhos, buscar no texto das anotações e executar ações.

use super::theme::*;
use super::widgets::*;
use super::{App, Screen, Tab, review};
use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

#[derive(Default)]
pub struct Palette {
    query: String,
    sel: usize,
    focused: bool,
}

#[derive(Clone)]
enum Cmd {
    StudyAll,
    AddNote,
    NewPage,
    NewSubject,
    Go(Screen),
    OpenSubject(i64),
    Notebook(i64),
    Study(i64),
    Page(i64),
    Preset(i64),
    ToggleTheme,
    Backup,
    CustomStudy,
    Browser(String),
}

struct Item {
    label: String,
    hint: String,
    group: &'static str,
    dot: Option<Color32>,
    cmd: Cmd,
}

fn matches(hay: &str, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let h = fold(hay);
    fold(q).split_whitespace().all(|w| h.contains(w))
}

/// Minúsculas sem acento, para "farmaco" achar "Farmacologia".
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

fn items(app: &App, q: &str) -> Vec<Item> {
    let mut out = Vec::new();
    let mut push = |label: String, hint: &str, group: &'static str, dot: Option<Color32>, cmd: Cmd| {
        if matches(&label, q) {
            out.push(Item { label, hint: hint.to_owned(), group, dot, cmd });
        }
    };
    let c = app.total_counts();
    push(format!("Estudar tudo ({} cartões)", c.new + c.learn + c.review), "Espaço", "Ações", None, Cmd::StudyAll);
    push("Adicionar nota".into(), "Ctrl ⇧ A", "Ações", None, Cmd::AddNote);
    push("Nova página".into(), "Ctrl N", "Ações", None, Cmd::NewPage);
    push("Nova disciplina".into(), "", "Ações", None, Cmd::NewSubject);
    push("Estudo personalizado…".into(), "", "Ações", None, Cmd::CustomStudy);
    push("Alternar tema claro/escuro".into(), "", "Ações", None, Cmd::ToggleTheme);
    push("Fazer backup agora".into(), "", "Ações", None, Cmd::Backup);
    push("Ir para o Painel".into(), "01", "Navegar", None, Cmd::Go(Screen::Dashboard));
    push("Ir para Estatísticas".into(), "02", "Navegar", None, Cmd::Go(Screen::Stats));
    push("Ir para Cartões".into(), "03", "Navegar", None, Cmd::Go(Screen::Browser));
    push("Configurações".into(), "", "Navegar", None, Cmd::Go(Screen::Settings));
    for s in &app.subjects {
        let col = Some(subject_color(s.color));
        push(format!("Abrir {}", s.name), "disciplina", "Disciplinas", col, Cmd::OpenSubject(s.id));
        push(format!("Caderno de {}", s.name), "caderno", "Disciplinas", col, Cmd::Notebook(s.id));
    }
    for d in &app.decks {
        let k = app.deck_counts(d.id);
        push(format!("Estudar {}", app.deck_path(d.id)), &format!("{} hoje", k.new + k.learn + k.review), "Baralhos", Some(app.subject_color_of_deck(d.id)), Cmd::Study(d.id));
    }
    for pr in &app.presets {
        push(format!("Opções do preset {}", pr.name), "preset", "Presets", None, Cmd::Preset(pr.id));
    }
    out
}

fn run(app: &mut App, cmd: Cmd) {
    match cmd {
        Cmd::StudyAll => review::start(app, review::Scope::All),
        Cmd::AddNote => app.add_note(),
        Cmd::NewPage => match app.current_subject() {
            Some(s) => app.new_page(s, None),
            None => app.toast("Crie uma disciplina primeiro"),
        },
        Cmd::NewSubject => app.modal = Some(super::modals::Modal::Subject { id: None, name: String::new(), color: app.subjects.len() % 8 }),
        Cmd::Go(s) => app.go(s),
        Cmd::OpenSubject(s) => app.go(Screen::Subject(s, Tab::Decks)),
        Cmd::Notebook(s) => super::subject::open_notebook(app, s),
        Cmd::Study(d) => review::start(app, review::Scope::Deck(d)),
        Cmd::Page(p) => app.open_page(p),
        Cmd::Preset(p) => {
            app.preset_view = None;
            app.go(Screen::Preset(p));
        }
        Cmd::ToggleTheme => {
            app.cfg.dark = !app.cfg.dark;
            app.save_settings();
        }
        Cmd::Backup => {
            app.maybe_backup(true);
            app.toast("Backup feito");
        }
        Cmd::CustomStudy => app.modal = Some(super::modals::Modal::custom_study(None)),
        Cmd::Browser(q) => {
            app.browser.query = q;
            app.browser.dirty = true;
            app.go(Screen::Browser);
        }
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut pal_state) = app.palette.take() else { return };
    let p = pal();
    let q = pal_state.query.trim().to_owned();
    let mut list = items(app, &q);
    // Páginas por título e por conteúdo.
    let pages = app.db.all_pages().unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for pg in pages.iter().filter(|pg| matches(&pg.title, &q)).take(if q.is_empty() { 6 } else { 12 }) {
        seen.insert(pg.id);
        let s = app.subject(pg.subject_id);
        list.push(Item { label: pg.title.clone(), hint: s.map(|s| s.name.clone()).unwrap_or_default(), group: "Páginas", dot: s.map(|s| subject_color(s.color)), cmd: Cmd::Page(pg.id) });
    }
    if q.chars().count() >= 3 {
        for (pid, _, snippet) in app.db.search_pages(&q, 12).unwrap_or_default() {
            if !seen.insert(pid) {
                continue;
            }
            let Some(pg) = pages.iter().find(|x| x.id == pid) else { continue };
            list.push(Item { label: pg.title.clone(), hint: snippet.replace('\n', " "), group: "No texto das páginas", dot: app.subject(pg.subject_id).map(|s| subject_color(s.color)), cmd: Cmd::Page(pid) });
        }
        list.push(Item { label: format!("Buscar “{q}” nos cartões"), hint: "Cartões".into(), group: "Cartões", dot: None, cmd: Cmd::Browser(q.clone()) });
    }
    if pal_state.sel >= list.len() {
        pal_state.sel = list.len().saturating_sub(1);
    }
    let mut close = false;
    let mut chosen: Option<Cmd> = None;
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close = true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
        pal_state.sel = (pal_state.sel + 1).min(list.len().saturating_sub(1));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
        pal_state.sel = pal_state.sel.saturating_sub(1);
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        chosen = list.get(pal_state.sel).map(|i| i.cmd.clone());
    }
    let screen = ctx.content_rect();
    let w = 640.0f32.min(screen.width() - 80.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("palette_scrim")));
    painter.rect_filled(screen, 0, p.scrim);
    let area = egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x - w / 2.0, screen.top() + 90.0)).show(ctx, |ui| {
        egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border_control)).corner_radius(14).shadow(egui::Shadow { offset: [0, 24], blur: 64, spread: 0, color: Color32::from_black_alpha(if p.dark { 140 } else { 46 }) }).show(ui, |ui| {
            ui.set_width(w);
            let (r, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
            text(ui.painter(), pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, "⌕", serif(17.0, W::Regular), p.muted, 20.0);
            let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.left() + 44.0, r.top()), pos2(r.right() - 70.0, r.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let resp = cu.add(egui::TextEdit::singleline(&mut pal_state.query).frame(egui::Frame::NONE).font(serif(16.0, W::Regular)).hint_text(egui::RichText::new("Buscar páginas, baralhos, ações…").italics().color(p.dim)).desired_width(w - 120.0));
            if !pal_state.focused {
                resp.request_focus();
                pal_state.focused = true;
            }
            if resp.changed() {
                pal_state.sel = 0;
            }
            kbd(ui.painter(), pos2(r.right() - 18.0, r.center().y), Align2::RIGHT_CENTER, "Esc");
            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, p.line));
            egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                let mut last_group = "";
                for (i, it) in list.iter().enumerate() {
                    if it.group != last_group {
                        last_group = it.group;
                        let (gr, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                        caps(ui.painter(), pos2(gr.left() + 20.0, gr.center().y + 3.0), Align2::LEFT_CENTER, it.group, p.dim);
                    }
                    let (rr, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
                    let row = rr.shrink2(vec2(8.0, 1.0));
                    if i == pal_state.sel {
                        ui.painter().rect_filled(row, CornerRadius::same(8), p.accent_soft);
                        ui.scroll_to_rect(rr, None);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(8), p.hover);
                    }
                    let mut x = row.left() + 12.0;
                    if let Some(c) = it.dot {
                        ui.painter().circle_filled(pos2(x + 4.0, row.center().y), 4.0, c);
                        x += 16.0;
                    }
                    let hint_w = if it.hint.is_empty() { 0.0 } else { 220.0 };
                    text(ui.painter(), pos2(x, row.center().y), Align2::LEFT_CENTER, &it.label, serif(14.0, if i == pal_state.sel { W::Semi } else { W::Regular }), p.text, row.right() - x - hint_w - 12.0);
                    if !it.hint.is_empty() {
                        text(ui.painter(), pos2(row.right() - 12.0, row.center().y), Align2::RIGHT_CENTER, &it.hint, serif(12.0, W::Regular), p.muted, hint_w);
                    }
                    a11y(&resp, egui::accesskit::Role::MenuItem, &it.label, Some(i == pal_state.sel));
                    if resp.clicked() {
                        chosen = Some(it.cmd.clone());
                    }
                }
                if list.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 60.0), Sense::hover());
                    text(ui.painter(), r.center(), Align2::CENTER_CENTER, "Nada encontrado", serif(13.0, W::Italic), p.muted, w);
                }
                ui.add_space(6.0);
            });
        });
    });
    if area.response.clicked_elsewhere() && chosen.is_none() {
        close = true;
    }
    let _ = StrokeKind::Inside;
    if let Some(cmd) = chosen {
        run(app, cmd);
        if matches!(app.screen, Screen::Settings) || app.cfg.dark != super::theme::pal().dark {
            apply(ctx, app.cfg.dark);
        }
        return;
    }
    if !close {
        app.palette = Some(pal_state);
    }
}
