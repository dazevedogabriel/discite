//! Caderno (Pagina.dc.html): árvore de páginas, editor por blocos com
//! marcação viva, barra de formatação e painel com sumário, backlinks e
//! cartões vinculados.
//!
//! Cada bloco de texto é um `TextEdit` próprio. Enter divide o bloco,
//! Backspace no início junta com o anterior, ↑/↓ nas bordas trocam de bloco,
//! Tab recua listas, "/" abre o menu de blocos e `# `, `- `, `1. `, `[] `,
//! `> `, ```` ``` ```` e `---` viram o bloco correspondente.

use super::richtext::{self, Look};
use super::theme::*;
use super::widgets::*;
use super::{App, review};
use crate::{tr, tr_n};
use crate::cards::NoteKind;
use crate::db::{Block, BlockKind, Card, Db, Note, PageMeta};
use crate::rich;
use crate::srs::{self, State};
use eframe::egui::{self, Align2, Color32, CornerRadius, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, text::CCursor, text::CCursorRange, vec2};
use std::time::{Duration, Instant};

pub struct PageEditor {
    pub meta: PageMeta,
    pub blocks: Vec<Block>,
    pub title: String,
    dirty: bool,
    last_change: Instant,
    last_snapshot: Instant,
    unsnapshotted: bool,
    pub focus_title: bool,
    focus: Option<i64>,
    sel: Option<(i64, CCursorRange)>,
    pending: Option<(i64, usize, usize)>,
    slash: Option<(i64, usize)>,
    drag: Option<i64>,
    focus_mode: bool,
    aside: Option<(u64, Aside)>,
    scroll_to: Option<i64>,
    saved_at: Option<Instant>,
    tag_edit: Option<String>,
    /// Pedido de cartões com IA feito neste quadro (aberto depois que o editor volta para o App).
    ai_request: Option<super::ai::Scope>,
}

#[derive(Default)]
struct Aside {
    backlinks: Vec<PageMeta>,
    linked: Vec<(Note, Vec<Card>)>,
}

enum Action {
    Split(usize, usize),
    MergePrev(usize),
    FocusPrev(usize),
    FocusNext(usize),
    InsertAfter(usize, BlockKind),
    Delete(usize),
    Duplicate(usize),
    Move(usize, usize),
    Convert(usize, BlockKind),
    SlashPick(usize, BlockKind),
    Flashcard(usize),
    /// Cartões com IA: `None` usa a seleção (se houver) ou a página; `Some(i)`, o bloco `i`.
    AiCards(Option<usize>),
    OpenLink(String),
    Viewer(String),
}

const SLASH: [BlockKind; 14] = [
    BlockKind::Paragraph,
    BlockKind::H1,
    BlockKind::H2,
    BlockKind::H3,
    BlockKind::Bullet,
    BlockKind::Numbered,
    BlockKind::Check,
    BlockKind::Quote,
    BlockKind::Callout,
    BlockKind::Code,
    BlockKind::Formula,
    BlockKind::Image,
    BlockKind::Table,
    BlockKind::Divider,
];

impl PageEditor {
    pub fn open(db: &Db, meta: PageMeta) -> anyhow::Result<PageEditor> {
        let mut blocks = db.load_blocks(meta.id)?;
        if blocks.is_empty() {
            blocks.push(Block::new(BlockKind::Paragraph, ""));
        }
        Ok(PageEditor {
            title: meta.title.clone(),
            meta,
            blocks,
            dirty: false,
            last_change: Instant::now(),
            last_snapshot: Instant::now(),
            unsnapshotted: false,
            focus_title: false,
            focus: None,
            sel: None,
            pending: None,
            slash: None,
            drag: None,
            focus_mode: false,
            aside: None,
            scroll_to: None,
            saved_at: None,
            tag_edit: None,
            ai_request: None,
        })
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.unsnapshotted = true;
        self.last_change = Instant::now();
    }

    /// Grava agora, se houver mudanças. Devolve se gravou.
    pub fn flush(&mut self, db: &Db) -> bool {
        if !self.dirty {
            return false;
        }
        let title = if self.title.trim().is_empty() { tr!("page.untitled") } else { self.title.trim().to_owned() };
        match db.save_blocks(self.meta.id, &title, &self.blocks) {
            Ok(()) => {
                self.dirty = false;
                self.meta.title = title;
                self.saved_at = Some(Instant::now());
                true
            }
            Err(_) => false,
        }
    }

    pub fn snapshot(&mut self, db: &Db) {
        if self.unsnapshotted {
            let _ = db.add_version(self.meta.id, &self.title, &self.blocks);
            self.unsnapshotted = false;
            self.last_snapshot = Instant::now();
        }
    }

    /// Salvamento automático (500 ms depois da última tecla) e versão a cada 10 min.
    pub fn autosave(&mut self, db: &Db) -> bool {
        let mut saved = false;
        if self.dirty && self.last_change.elapsed() > Duration::from_millis(500) {
            saved = self.flush(db);
        }
        if self.unsnapshotted && self.last_snapshot.elapsed() > Duration::from_secs(600) {
            self.snapshot(db);
        }
        saved
    }

    fn idx(&self, id: i64) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    fn focus_block(&mut self, id: i64, a: usize, b: usize) {
        self.pending = Some((id, a, b));
    }
}

fn block_id(id: i64) -> Id {
    Id::unique(("blk", id))
}

fn look_of(kind: BlockKind) -> Look {
    let p = pal();
    match kind {
        BlockKind::H1 => Look { size: 27.0, semi: true, italic: false, color: p.text, line_height: Some(36.0) },
        BlockKind::H2 => Look { size: 21.0, semi: true, italic: false, color: p.text, line_height: Some(30.0) },
        BlockKind::H3 => Look { size: 17.5, semi: true, italic: false, color: p.text, line_height: Some(26.0) },
        BlockKind::Quote => Look { size: 16.0, semi: false, italic: true, color: p.text2, line_height: Some(26.0) },
        BlockKind::Callout => Look { size: 15.0, semi: false, italic: false, color: p.text2, line_height: Some(24.0) },
        _ => Look { size: 16.0, semi: false, italic: false, color: p.text2, line_height: Some(26.5) },
    }
}

pub fn ui(app: &mut App, ui: &mut Ui, sid: i64) {
    let p = pal();
    let full = ui.max_rect();
    let focus_mode = app.page.as_ref().is_some_and(|e| e.focus_mode);
    let tree_w = if focus_mode { 0.0 } else { 214.0 };
    let aside_w = if focus_mode || full.width() < 1000.0 { 0.0 } else { 248.0 };
    if tree_w > 0.0 {
        let tree = Rect::from_min_size(full.min, vec2(tree_w, full.height()));
        let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(tree));
        page_tree(app, &mut tu, sid, tree);
    }
    let main = Rect::from_min_max(pos2(full.left() + tree_w, full.top()), pos2(full.right() - aside_w, full.bottom()));
    let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(main));
    if app.page.as_ref().is_none_or(|e| e.meta.subject_id != sid) {
        empty_notebook(app, &mut mu, main, sid);
        return;
    }
    editor(app, &mut mu, main, sid);
    if aside_w > 0.0 {
        let aside = Rect::from_min_max(pos2(full.right() - aside_w, full.top()), full.max);
        ui.painter().vline(aside.left(), aside.y_range(), Stroke::new(1.0, p.line));
        let mut au = ui.new_child(egui::UiBuilder::new().max_rect(aside.shrink(18.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        page_aside(app, &mut au);
    }
}

fn empty_notebook(app: &mut App, ui: &mut Ui, rect: Rect, sid: i64) {
    let p = pal();
    let r = Rect::from_center_size(rect.center() - vec2(0.0, 60.0), vec2(440.0, 190.0));
    paint_card(ui.painter(), r);
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(28.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    cu.spacing_mut().item_spacing.y = 12.0;
    caps_label(&mut cu, &tr!("page.empty_title"));
    para(&mut cu, &tr!("page.empty_text"), serif(15.0, W::Regular), p.text2);
    if button_kbd(&mut cu, &tr!("page.new_page"), Some("Ctrl N"), Btn::Primary, 36.0).clicked() {
        app.new_page(sid, None);
    }
}

// ───────────────────────────── árvore ─────────────────────────────

fn page_tree(app: &mut App, ui: &mut Ui, sid: i64, rect: Rect) {
    let p = pal();
    ui.painter().rect_filled(rect, 0, p.window);
    ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, p.line));
    let key = egui::Id::unique(("page_tree_cache", sid));
    let cached: Option<(u64, Vec<PageMeta>)> = ui.data(|d| d.get_temp(key));
    let pages = match cached {
        Some((v, list)) if v == app.data_version => list,
        _ => {
            let list = app.db.list_pages(sid).unwrap_or_default();
            ui.data_mut(|d| d.insert_temp(key, (app.data_version, list.clone())));
            list
        }
    };
    let current = app.page.as_ref().map(|e| (e.meta.id, e.title.clone()));
    let inner = rect.shrink2(vec2(10.0, 16.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 28.0));
    caps(ui.painter(), pos2(head.left() + 8.0, head.center().y), Align2::LEFT_CENTER, &tr!("page.notebook"), p.dim);
    let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let more = icon_button(&mut hu, "⋯", &tr!("page.more_options"), 24.0);
    egui::Popup::menu(&more).frame(popup_frame()).width(220.0).show(|ui| {
        if menu_item(ui, &tr!("page.import_markdown"), false).clicked() {
            import_markdown(app, sid);
        }
    });
    if icon_button(&mut hu, "+", &tr!("page.new_page_hint"), 24.0).clicked() {
        app.new_page(sid, None);
    }
    let foot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 26.0), inner.max);
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, p.line));
    text(ui.painter(), pos2(foot.left() + 8.0, foot.center().y + 4.0), Align2::LEFT_CENTER, &tr_n!("page.count", pages.len()), serif(12.0, W::Regular), p.muted, 120.0);
    kbd(ui.painter(), pos2(foot.right() - 6.0, foot.center().y + 4.0), Align2::RIGHT_CENTER, "Ctrl N");
    let list = Rect::from_min_max(pos2(inner.left(), head.bottom() + 8.0), pos2(inner.right(), foot.top() - 6.0));
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("tree", sid)).auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let roots: Vec<PageMeta> = pages.iter().filter(|x| x.parent_id.is_none_or(|pid| !pages.iter().any(|q| q.id == pid))).cloned().collect();
        for r in roots {
            tree_row(app, ui, &pages, &r, 0, current.as_ref());
        }
    });
}

fn tree_row(app: &mut App, ui: &mut Ui, all: &[PageMeta], pg: &PageMeta, lvl: usize, current: Option<&(i64, String)>) {
    let p = pal();
    let kids: Vec<PageMeta> = all.iter().filter(|x| x.parent_id == Some(pg.id)).cloned().collect();
    let on = current.is_some_and(|c| c.0 == pg.id);
    let title = if on { current.map(|c| c.1.clone()).unwrap_or_default() } else { pg.title.clone() };
    let title = if title.trim().is_empty() { tr!("page.untitled") } else { title };
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if on {
        ui.painter().rect_filled(r, CornerRadius::same(7), p.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(7), p.hover);
    }
    let x = r.left() + 10.0 + lvl as f32 * 18.0;
    if !kids.is_empty() {
        let caret = Rect::from_center_size(pos2(x + 4.0, r.center().y), vec2(16.0, 24.0));
        let cr = ui.interact(caret, ui.scope_id().with(("pcaret", pg.id)), Sense::click());
        text(ui.painter(), caret.center(), Align2::CENTER_CENTER, if pg.collapsed { "▸" } else { "▾" }, serif(10.0, W::Regular), if on { p.accent } else { p.dim }, 16.0);
        if cr.clicked() {
            let mut m = pg.clone();
            m.collapsed = !m.collapsed;
            let _ = app.db.update_page_meta(&m);
            app.data_version += 1;
        }
    }
    let tx = x + 14.0;
    text(ui.painter(), pos2(tx, r.center().y), Align2::LEFT_CENTER, &title, serif(13.0, if on { W::Semi } else { W::Regular }), if on { p.text } else { p.muted }, r.right() - tx - 4.0);
    a11y(&resp, egui::accesskit::Role::TreeItem, &title, Some(on));
    if resp.clicked() && !on {
        app.open_page(pg.id);
    }
    resp.context_menu(|ui| {
        if menu_item(ui, &tr!("page.new_subpage"), false).clicked() {
            app.new_page(pg.subject_id, Some(pg.id));
        }
        let siblings: Vec<PageMeta> = all.iter().filter(|x| x.parent_id == pg.parent_id).cloned().collect();
        let i = siblings.iter().position(|s| s.id == pg.id).unwrap_or(0);
        if i > 0 && menu_item(ui, &tr!("common.move_up"), false).clicked() {
            swap_pages(app, &siblings[i], &siblings[i - 1]);
        }
        if i + 1 < siblings.len() && menu_item(ui, &tr!("common.move_down"), false).clicked() {
            swap_pages(app, &siblings[i], &siblings[i + 1]);
        }
        if i > 0 && menu_item(ui, &tr!("page.indent"), false).clicked() {
            let mut m = pg.clone();
            m.parent_id = Some(siblings[i - 1].id);
            m.position = 1e9;
            let _ = app.db.update_page_meta(&m);
            app.data_version += 1;
        }
        if pg.parent_id.is_some() && menu_item(ui, &tr!("page.outdent"), false).clicked() {
            let parent = all.iter().find(|x| Some(x.id) == pg.parent_id);
            let mut m = pg.clone();
            m.parent_id = parent.and_then(|x| x.parent_id);
            m.position = parent.map(|x| x.position + 0.5).unwrap_or(0.0);
            let _ = app.db.update_page_meta(&m);
            app.data_version += 1;
        }
        ui.menu_button(tr!("page.move_to_subject"), |ui| {
            for s in app.subjects.clone() {
                if s.id != pg.subject_id && menu_item_ex(ui, &s.name, Some(subject_color(s.color)), false, None).clicked() {
                    move_to_subject(app, all, pg, s.id);
                }
            }
        });
        menu_sep(ui);
        if menu_item(ui, &tr!("page.delete"), false).clicked() {
            app.modal = Some(super::modals::Modal::DeletePage(pg.id));
        }
    });
    if !pg.collapsed {
        for k in kids {
            tree_row(app, ui, all, &k, lvl + 1, current);
        }
    }
}

fn swap_pages(app: &mut App, a: &PageMeta, b: &PageMeta) {
    let (mut a, mut b) = (a.clone(), b.clone());
    std::mem::swap(&mut a.position, &mut b.position);
    if a.position == b.position {
        b.position += 0.5;
    }
    let _ = app.db.update_page_meta(&a);
    let _ = app.db.update_page_meta(&b);
    app.data_version += 1;
}

fn move_to_subject(app: &mut App, all: &[PageMeta], pg: &PageMeta, sid: i64) {
    let mut ids = vec![pg.id];
    let mut i = 0;
    while i < ids.len() {
        let cur = ids[i];
        ids.extend(all.iter().filter(|x| x.parent_id == Some(cur)).map(|x| x.id));
        i += 1;
    }
    if let Some(e) = app.page.as_mut() {
        e.flush(&app.db);
    }
    for id in ids {
        if let Some(mut m) = all.iter().find(|x| x.id == id).cloned() {
            m.subject_id = sid;
            if m.id == pg.id {
                m.parent_id = None;
            }
            let _ = app.db.update_page_meta(&m);
        }
    }
    app.page = None;
    app.data_version += 1;
    app.open_page(pg.id);
}

// ───────────────────────────── editor ─────────────────────────────

fn editor(app: &mut App, ui: &mut Ui, rect: Rect, sid: i64) {
    let p = pal();
    let subject = app.subject(sid).map(|s| s.name.clone()).unwrap_or_default();
    let Some(mut ed) = app.page.take() else { return };

    // ── Barra superior ──
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), 48.0));
    ui.painter().hline(top.x_range(), top.bottom() - 0.5, Stroke::new(1.0, p.line));
    let crumb = tr!("page.breadcrumb", subject = subject);
    let r1 = text(ui.painter(), pos2(top.left() + 20.0, top.center().y), Align2::LEFT_CENTER, &crumb, serif(13.0, W::Regular), p.muted, 400.0);
    text(ui.painter(), pos2(r1.right(), top.center().y), Align2::LEFT_CENTER, &ed.title, serif(13.0, W::Regular), p.text2, (top.width() - r1.width() - 330.0).max(60.0));
    let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(top.shrink2(vec2(20.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    tu.spacing_mut().item_spacing.x = 4.0;
    if button(&mut tu, &tr!("page.export"), Btn::Ghost, 30.0).clicked() {
        ed.flush(&app.db);
        export_markdown(app, &ed);
    }
    if button(&mut tu, &tr!("page.history"), Btn::Ghost, 30.0).clicked() {
        ed.flush(&app.db);
        ed.snapshot(&app.db);
        app.modal = Some(super::modals::Modal::Versions(ed.meta.id));
    }
    if button(&mut tu, &tr!(if ed.focus_mode { "page.exit_focus" } else { "page.focus" }), Btn::Ghost, 30.0).clicked() {
        ed.focus_mode = !ed.focus_mode;
    }
    tu.add_space(6.0);
    let (status, col) = if ed.dirty { (tr!("page.editing"), p.hard) } else { (tr!("page.saved"), p.good) };
    let status = status.as_str();
    let (sr, _) = tu.allocate_exact_size(vec2(text_width(tu.painter(), status, serif(12.0, W::Regular)) + 14.0, 20.0), Sense::hover());
    tu.painter().circle_filled(pos2(sr.left() + 3.0, sr.center().y), 3.0, col);
    text(tu.painter(), pos2(sr.left() + 12.0, sr.center().y), Align2::LEFT_CENTER, status, serif(12.0, W::Regular), p.muted, 80.0);

    // ── Barra de formatação ──
    let bar = Rect::from_min_size(pos2(rect.left(), top.bottom()), vec2(rect.width(), 46.0));
    ui.painter().rect_filled(bar, 0, p.card);
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut actions: Vec<Action> = Vec::new();
    toolbar(app, &mut ed, ui, bar, &mut actions);

    // ── Conteúdo ──
    let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
    let col_w = app.cfg.reading_width.clamp(480.0, 900.0).min(body.width() - 100.0);
    let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let mut block_rects: Vec<(i64, Rect)> = Vec::new();
    egui::ScrollArea::vertical().id_salt(("page", ed.meta.id)).auto_shrink([false, false]).show(&mut bu, |ui| {
        ui.add_space(34.0);
        let left_pad = ((ui.available_width() - col_w) / 2.0).max(56.0);
        ui.horizontal(|ui| {
            ui.add_space(left_pad);
            ui.vertical(|ui| {
                ui.set_width(col_w);
                ui.spacing_mut().item_spacing.y = 10.0;
                page_head(app, &mut ed, ui, col_w);
                let n = ed.blocks.len();
                let mut numbering = Vec::with_capacity(n);
                let mut counters = [0u32; 8];
                for b in &ed.blocks {
                    if b.kind == BlockKind::Numbered {
                        let lvl = b.indent.min(7) as usize;
                        counters[lvl] += 1;
                        for c in counters.iter_mut().skip(lvl + 1) {
                            *c = 0;
                        }
                        numbering.push(counters[lvl]);
                    } else {
                        if !b.kind.is_list() {
                            counters = [0; 8];
                        }
                        numbering.push(0);
                    }
                }
                for i in 0..n {
                    let r = block_ui(app, &mut ed, ui, i, col_w, numbering[i], &mut actions);
                    block_rects.push((ed.blocks[i].id, r));
                }
                // Área clicável no fim: cria/foca um parágrafo.
                let (r, resp) = ui.allocate_exact_size(vec2(col_w, 160.0), Sense::click());
                if resp.clicked() {
                    match ed.blocks.last() {
                        Some(b) if b.kind == BlockKind::Paragraph && b.text.is_empty() => {
                            let id = b.id;
                            ed.focus_block(id, 0, 0);
                        }
                        _ => actions.push(Action::InsertAfter(ed.blocks.len() - 1, BlockKind::Paragraph)),
                    }
                }
                let _ = r;
            });
        });
    });

    // Arrastar blocos.
    if let Some(drag_id) = ed.drag {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            let mut target = block_rects.len();
            for (i, (_, r)) in block_rects.iter().enumerate() {
                if pos.y < r.center().y {
                    target = i;
                    break;
                }
            }
            let y = block_rects.get(target).map(|(_, r)| r.top() - 5.0).or_else(|| block_rects.last().map(|(_, r)| r.bottom() + 5.0));
            if let (Some(y), Some((_, r0))) = (y, block_rects.first()) {
                ui.painter().hline(r0.left()..=r0.left() + col_w, y, Stroke::new(2.0, p.accent));
            }
            if ui.ctx().input(|i| i.pointer.any_released()) {
                if let Some(from) = ed.idx(drag_id) {
                    actions.push(Action::Move(from, target));
                }
                ed.drag = None;
            }
        } else {
            ed.drag = None;
        }
    }

    // Imagem colada / solta.
    let focused_idx = ed.focus.and_then(|id| ed.idx(id)).filter(|_| ui.ctx().memory(|m| m.focused().is_some()));
    if let Some(name) = app.pasted_image.take() {
        let at = focused_idx.unwrap_or(ed.blocks.len().saturating_sub(1));
        insert_image_block(&mut ed, at, name);
    }
    for path in std::mem::take(&mut app.dropped) {
        if crate::media::is_image_path(&path) {
            match crate::media::store_file(&app.db.media_dir(), &path) {
                Ok(s) => {
                    let _ = app.db.add_media(&s);
                    let at = focused_idx.unwrap_or(ed.blocks.len().saturating_sub(1));
                    insert_image_block(&mut ed, at, s.file_name);
                }
                Err(e) => app.error(e),
            }
        }
    }

    // Ctrl+Shift+F: flashcard da seleção.
    if ui.ctx().input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
        if let Some(i) = ed.focus.and_then(|id| ed.idx(id)) {
            actions.push(Action::Flashcard(i));
        }
    }
    // Ctrl+Shift+G: cartões com IA da seleção ou da página.
    if ui.ctx().input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G)) {
        actions.push(Action::AiCards(None));
    }

    let nav = apply_actions(app, &mut ed, actions, sid);
    // O foco no bloco novo só vale no quadro seguinte: pede esse quadro já,
    // antes que a próxima tecla chegue e caia no bloco antigo.
    if ed.pending.is_some() || ed.focus_title {
        ui.ctx().request_repaint();
    }
    if ed.autosave(&app.db) {
        app.data_version += 1;
    }
    let ai_request = ed.ai_request.take();
    app.page = Some(ed);
    if let Some(scope) = ai_request {
        super::ai::open(app, scope);
    }
    if let Some(id) = nav {
        app.open_page(id);
    }
}

fn page_head(app: &mut App, ed: &mut PageEditor, ui: &mut Ui, w: f32) {
    let p = pal();
    // Tags + "editado há…".
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let tags: Vec<String> = ed.meta.tags.split_whitespace().map(str::to_owned).collect();
        for t in &tags {
            let r = chip(ui, &format!("#{t}"), false, 22.0);
            r.context_menu(|ui| {
                if menu_item(ui, &tr!("page.remove_tag"), false).clicked() {
                    ed.meta.tags = tags.iter().filter(|x| *x != t).cloned().collect::<Vec<_>>().join(" ");
                    let _ = app.db.update_page_meta(&ed.meta);
                    let _ = app.db.touch_page(ed.meta.id);
                }
            });
        }
        match &mut ed.tag_edit {
            Some(s) => {
                let resp = ui.add(egui::TextEdit::singleline(s).font(serif(12.0, W::Regular)).desired_width(120.0).hint_text(tr!("page.new_tag")));
                if !resp.has_focus() && !resp.lost_focus() {
                    resp.request_focus();
                }
                if resp.lost_focus() {
                    let add = s.clone();
                    ed.tag_edit = None;
                    if !add.trim().is_empty() {
                        ed.meta.tags = crate::db::normalize_tags(&format!("{} {}", ed.meta.tags, add));
                        let _ = app.db.update_page_meta(&ed.meta);
                        let _ = app.db.touch_page(ed.meta.id);
                        app.data_version += 1;
                    }
                }
            }
            None => {
                if chip(ui, &tr!("page.add_tag"), false, 22.0).clicked() {
                    ed.tag_edit = Some(String::new());
                }
            }
        }
        label(ui, &tr!("page.edited", when = ago(ed.meta.updated_at)), serif(11.5, W::Italic), p.dim);
    });
    // Título.
    let tid = Id::unique(("page_title", ed.meta.id));
    let resp = ui.add(
        egui::TextEdit::multiline(&mut ed.title)
            .id(tid)
            .frame(egui::Frame::NONE)
            .font(serif(34.0, W::Semi))
            .text_color(p.text)
            .desired_width(w)
            .desired_rows(1)
            .hint_text(egui::RichText::new(tr!("page.untitled")).color(p.dim)),
    );
    if ed.focus_title {
        resp.request_focus();
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), tid) {
            st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(ed.title.chars().count()))));
            st.store(ui.ctx(), tid);
        }
        ed.focus_title = false;
    }
    if resp.has_focus() && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        if let Some(first) = ed.blocks.first() {
            let id = first.id;
            ed.focus_block(id, 0, 0);
        }
    }
    if ed.title.contains('\n') {
        ed.title = ed.title.replace('\n', " ");
    }
    if resp.changed() {
        ed.touch();
    }
}

fn toolbar(app: &mut App, ed: &mut PageEditor, ui: &mut Ui, bar: Rect, actions: &mut Vec<Action>) {
    let p = pal();
    let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    tu.set_clip_rect(bar);
    tu.spacing_mut().item_spacing.x = 2.0;
    // Em larguras menores, Imagem/Tabela/Fórmula vão para o menu "Mais".
    let compact = bar.width() < 900.0;
    // O botão da IA só cabe na barra larga; nas outras, fica no menu Inserir/Mais.
    let ai_button = bar.width() >= 1180.0;
    let fidx = ed.focus.and_then(|id| ed.idx(id));
    let cur_kind = fidx.map(|i| ed.blocks[i].kind).unwrap_or(BlockKind::Paragraph);
    let label_kind = if cur_kind.is_text() { cur_kind.label() } else { BlockKind::Paragraph.label() };
    let kr = button_sized(&mut tu, &format!("{label_kind}  ▾"), None, Btn::Ghost, vec2(if compact { 132.0 } else { 168.0 }, 30.0), true);
    egui::Popup::menu(&kr).frame(popup_frame()).width(220.0).show(|ui| {
        for k in [BlockKind::Paragraph, BlockKind::H1, BlockKind::H2, BlockKind::H3, BlockKind::Bullet, BlockKind::Numbered, BlockKind::Check, BlockKind::Quote, BlockKind::Callout, BlockKind::Code, BlockKind::Formula] {
            if menu_item(ui, &k.label(), k == cur_kind).clicked() {
                if let Some(i) = fidx {
                    actions.push(Action::Convert(i, k));
                }
            }
        }
    });
    sep(&mut tu);
    for (g, m, key) in [("B", "**", "page.bold"), ("I", "*", "page.italic"), ("U", "__", "page.underline"), ("S", "~~", "page.strike"), ("</>", "`", "page.inline_code"), ("▮", "==", "page.highlight"), ("[[ ]]", "[[", "page.link")] {
        if tool_btn(&mut tu, g, &tr!(key)).clicked() {
            wrap_focused(ed, m);
        }
    }
    sep(&mut tu);
    for (g, k) in [("•≡", BlockKind::Bullet), ("1≡", BlockKind::Numbered), ("☑", BlockKind::Check), ("“", BlockKind::Quote)] {
        let on = cur_kind == k && fidx.is_some();
        let r = tool_btn_on(&mut tu, g, &k.label(), on);
        if r.clicked() {
            if let Some(i) = fidx {
                actions.push(Action::Convert(i, if on { BlockKind::Paragraph } else { k }));
            }
        }
    }
    sep(&mut tu);
    let at = fidx.unwrap_or(ed.blocks.len().saturating_sub(1));
    if !compact {
        if button(&mut tu, &tr!("page.image"), Btn::Ghost, 30.0).clicked() {
            actions.push(Action::InsertAfter(at, BlockKind::Image));
        }
        if button(&mut tu, &tr!("page.table"), Btn::Ghost, 30.0).clicked() {
            actions.push(Action::InsertAfter(at, BlockKind::Table));
        }
        if button(&mut tu, &tr!("page.formula"), Btn::Ghost, 30.0).clicked() {
            actions.push(Action::InsertAfter(at, BlockKind::Formula));
        }
    }
    let more = button_kbd(&mut tu, &tr!(if compact { "page.insert" } else { "page.more" }), Some("/"), Btn::Ghost, 30.0);
    let extra: &[BlockKind] = if compact { &[BlockKind::Image, BlockKind::Table, BlockKind::Formula] } else { &[] };
    egui::Popup::menu(&more).frame(popup_frame()).width(220.0).show(|ui| {
        for k in extra.iter().copied().chain([BlockKind::Callout, BlockKind::Code, BlockKind::Divider, BlockKind::H1, BlockKind::H2, BlockKind::H3]) {
            if menu_item(ui, &k.label(), false).clicked() {
                actions.push(Action::InsertAfter(at, k));
            }
        }
        if !ai_button {
            menu_sep(ui);
            if menu_item_ex(ui, &tr!("ai.button"), None, false, Some(&keys("Ctrl ⇧ G"))).clicked() {
                actions.push(Action::AiCards(None));
            }
        }
    });
    tu.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if button_kbd(ui, &tr!("page.create_flashcard"), if compact { None } else { Some("Ctrl ⇧ F") }, Btn::Primary, 30.0).clicked() {
            actions.push(Action::Flashcard(at));
        }
        if ai_button && button(ui, &tr!("ai.button"), Btn::Secondary, 30.0).on_hover_text(keys(&tr!("ai.button_tip"))).clicked() {
            actions.push(Action::AiCards(None));
        }
    });
    let _ = (app, p);
}

fn sep(ui: &mut Ui) {
    let p = pal();
    let (r, _) = ui.allocate_exact_size(vec2(13.0, 20.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, p.border));
}

fn tool_btn(ui: &mut Ui, glyph: &str, label: &str) -> egui::Response {
    tool_btn_on(ui, glyph, label, false)
}

fn tool_btn_on(ui: &mut Ui, glyph: &str, label: &str, on: bool) -> egui::Response {
    let p = pal();
    let w = (text_width(ui.painter(), glyph, serif(13.0, W::Regular)) + 16.0).max(30.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    if on {
        ui.painter().rect_filled(r, CornerRadius::same(8), p.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(8), p.hover);
    }
    let font = match glyph {
        "B" => serif(14.0, W::Bold),
        "I" => serif(14.0, W::Italic),
        "</>" | "[[ ]]" => mono(11.0),
        _ => serif(14.0, W::Regular),
    };
    let color = if glyph == "▮" { p.hard } else { p.text2 };
    let tr = text(ui.painter(), r.center(), Align2::CENTER_CENTER, glyph, font, color, w);
    match glyph {
        "U" => {
            ui.painter().hline(tr.x_range(), tr.bottom() - 1.0, Stroke::new(1.0, p.text2));
        }
        "S" => {
            ui.painter().hline(tr.x_range(), tr.center().y, Stroke::new(1.0, p.text2));
        }
        _ => {}
    }
    a11y(&resp, egui::accesskit::Role::Button, label, Some(on));
    resp.on_hover_text(keys(label))
}

fn wrap_focused(ed: &mut PageEditor, marker: &str) {
    let Some((id, range)) = ed.sel else { return };
    let Some(i) = ed.idx(id) else { return };
    let b = &mut ed.blocks[i];
    if !b.kind.is_text() || matches!(b.kind, BlockKind::Code | BlockKind::Formula) {
        return;
    }
    let (a, c) = (range.primary.index.0.min(range.secondary.index.0), range.primary.index.0.max(range.secondary.index.0));
    let (ba, bc) = (rich::byte_of_char(&b.text, a), rich::byte_of_char(&b.text, c));
    let (s, na, nc) = if marker == "[[" {
        let mut s = b.text.clone();
        s.insert_str(bc, "]]");
        s.insert_str(ba, "[[");
        (s, ba + 2, bc + 2)
    } else {
        rich::toggle_wrap(&b.text, ba, bc, marker)
    };
    let (ca, cc) = (rich::char_of_byte(&s, na), rich::char_of_byte(&s, nc));
    b.text = s;
    ed.focus_block(id, ca, cc);
    ed.touch();
}

fn insert_image_block(ed: &mut PageEditor, at: usize, name: String) {
    let mut b = Block::new(BlockKind::Image, "");
    b.media = name;
    b.width = 0.75;
    let pos = (at + 1).min(ed.blocks.len());
    ed.blocks.insert(pos, b);
    // Se o bloco atual era um parágrafo vazio, a imagem toma o lugar dele.
    if at < ed.blocks.len() && ed.blocks[at].kind == BlockKind::Paragraph && ed.blocks[at].text.is_empty() && ed.blocks.len() > 1 {
        ed.blocks.remove(at);
    }
    ed.touch();
}

/// Desenha um bloco; devolve o retângulo que ocupou.
fn block_ui(app: &mut App, ed: &mut PageEditor, ui: &mut Ui, i: usize, w: f32, number: u32, actions: &mut Vec<Action>) -> Rect {
    let p = pal();
    let ctx = ui.ctx().clone();
    let id = ed.blocks[i].id;
    let kind = ed.blocks[i].kind;
    let eid = block_id(id);
    let has_focus = ctx.memory(|m| m.has_focus(eid));
    let top = ui.cursor().top();

    // Teclas tratadas antes do TextEdit.
    if has_focus && kind.is_text() {
        handle_keys(ed, &ctx, i, eid, actions);
    }

    let mut changed = false;
    let mut text_resp: Option<egui::text_edit::TextEditOutput> = None;
    let indent = ed.blocks[i].indent as f32 * 22.0;
    match kind {
        BlockKind::Divider => {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
            ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border_strong));
            if resp.clicked() {
                ed.focus = Some(id);
            }
        }
        BlockKind::Image => image_block(app, ed, ui, i, w, actions),
        BlockKind::Table => {
            changed |= table_block(ed, ui, i, w);
        }
        BlockKind::Code | BlockKind::Formula => {
            let is_code = kind == BlockKind::Code;
            egui::Frame::NONE.fill(p.input).stroke(Stroke::new(1.0, p.border)).corner_radius(8).inner_margin(egui::Margin { left: 14, right: 14, top: if is_code { 10 } else { 8 }, bottom: 10 }).show(ui, |ui| {
                ui.set_width(w - 28.0);
                let (r, _) = ui.allocate_exact_size(vec2(w - 28.0, 14.0), Sense::hover());
                caps(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &if is_code { if ed.blocks[i].lang.is_empty() { tr!("page.code") } else { ed.blocks[i].lang.clone() } } else { tr!("page.formula_label") }, if is_code { p.dim } else { p.accent });
                let mut lay = richtext::mono_layouter(13.0, p.text);
                let out = egui::TextEdit::multiline(&mut ed.blocks[i].text).id(eid).frame(egui::Frame::NONE).desired_width(w - 28.0).desired_rows(1).code_editor().layouter(&mut lay).show(ui);
                changed |= out.response.response.changed();
                text_resp = Some(out);
            });
        }
        _ => {
            let look = look_of(kind);
            let marker_w = if kind.is_list() { 24.0 } else { 0.0 };
            let inner_w = w - indent - marker_w;
            let frame = match kind {
                BlockKind::Callout => egui::Frame::NONE.fill(p.accent_tint).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 12 }),
                BlockKind::Quote => egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 0, top: 2, bottom: 2 }),
                _ => egui::Frame::NONE,
            };
            let space_before = match kind {
                BlockKind::H1 => 10.0,
                BlockKind::H2 => 8.0,
                BlockKind::H3 => 4.0,
                _ => 0.0,
            };
            if space_before > 0.0 && i > 0 {
                ui.add_space(space_before);
            }
            let fr = frame.show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if indent > 0.0 {
                        ui.add_space(indent);
                    }
                    let ew = match kind {
                        BlockKind::Callout => {
                            let (r, _) = ui.allocate_exact_size(vec2(52.0, 22.0), Sense::hover());
                            caps(ui.painter(), pos2(r.left(), r.top() + 13.0), Align2::LEFT_CENTER, &tr!("page.tip"), p.accent);
                            inner_w - 28.0 - 52.0
                        }
                        BlockKind::Quote => inner_w - 18.0,
                        _ => inner_w,
                    };
                    if kind.is_list() {
                        let (r, resp) = ui.allocate_exact_size(vec2(marker_w, look.line_height.unwrap_or(26.0)), Sense::click());
                        match kind {
                            BlockKind::Bullet => {
                                let glyph = ["•", "◦", "▪"][(ed.blocks[i].indent as usize) % 3];
                                text(ui.painter(), pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER, glyph, serif(16.0, W::Regular), p.text2, 20.0);
                            }
                            BlockKind::Numbered => {
                                text(ui.painter(), pos2(r.right() - 7.0, r.center().y), Align2::RIGHT_CENTER, &format!("{number}."), serif(16.0, W::Regular), p.text2, 24.0);
                            }
                            _ => {
                                let b = Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(15.0, 15.0));
                                paint_check(ui.painter(), b, ed.blocks[i].checked);
                                if resp.clicked() {
                                    ed.blocks[i].checked = !ed.blocks[i].checked;
                                    changed = true;
                                }
                            }
                        }
                    }
                    let checked = kind == BlockKind::Check && ed.blocks[i].checked;
                    let look = if checked { Look { color: p.muted, ..look } } else { look };
                    let mut lay_edit = richtext::layouter(look);
                    let mut lay_quiet = richtext::layouter_quiet(look);
                    let lay: &mut dyn FnMut(&Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> = if has_focus { &mut lay_edit } else { &mut lay_quiet };
                    let hint = if has_focus && ed.blocks[i].text.is_empty() && kind == BlockKind::Paragraph {
                        tr!("page.slash_hint")
                    } else if ed.blocks[i].text.is_empty() && kind != BlockKind::Paragraph {
                        kind.label()
                    } else {
                        String::new()
                    };
                    let out = egui::TextEdit::multiline(&mut ed.blocks[i].text)
                        .id(eid)
                        .frame(egui::Frame::NONE)
                        .desired_width(ew)
                        .desired_rows(1)
                        .layouter(lay)
                        .hint_text(egui::RichText::new(hint).color(p.dim).italics().size(look.size))
                        .show(ui);
                    changed |= out.response.response.changed();
                    text_resp = Some(out);
                });
            });
            if kind == BlockKind::Quote {
                let r = fr.response.rect;
                ui.painter().vline(r.left() + 2.0, r.y_range().shrink(2.0), Stroke::new(3.0, p.border_strong));
            }
        }
    }

    if let Some(out) = &text_resp {
        let resp = &out.response.response;
        if resp.has_focus() {
            ed.focus = Some(id);
            if let Some(r) = out.cursor_range {
                ed.sel = Some((id, r));
            }
        }
        // Ctrl+clique num [[link]].
        if resp.clicked() && ctx.input(|i| i.modifiers.command) {
            if let Some(c) = out.cursor_range.map(|r| r.primary.index.0) {
                if let Some(target) = link_at(&ed.blocks[i].text, c) {
                    actions.push(Action::OpenLink(target));
                }
            }
        }
        if let Some((pid, a, b)) = ed.pending {
            if pid == id {
                let mut st = out.state.clone();
                st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                st.store(&ctx, eid);
                resp.request_focus();
                ed.pending = None;
                ed.focus = Some(id);
            }
        }
    } else if let Some((pid, _, _)) = ed.pending {
        if pid == id {
            ed.pending = None;
            ed.focus = Some(id);
        }
    }

    if changed {
        ed.touch();
        markdown_shortcuts(ed, i);
        // Menu "/".
        let b = &ed.blocks[i];
        if b.kind == BlockKind::Paragraph && b.text.starts_with('/') && !b.text.contains('\n') && b.text.len() < 24 {
            if ed.slash.is_none_or(|(sid, _)| sid != id) {
                ed.slash = Some((id, 0));
            }
        } else if ed.slash.is_some_and(|(sid, _)| sid == id) {
            ed.slash = None;
        }
    }

    let rect = Rect::from_min_max(pos2(ui.min_rect().left(), top), pos2(ui.min_rect().left() + w, ui.cursor().top() - ui.spacing().item_spacing.y));
    if ed.scroll_to == Some(id) {
        ui.scroll_to_rect(rect, Some(egui::Align::TOP));
        ed.scroll_to = None;
    }

    // Menu "/" deste bloco.
    if let Some((sid, sel)) = ed.slash {
        if sid == id {
            slash_menu(ed, ui, i, rect, sel, actions);
        }
    }

    gutter(ed, ui, i, rect, actions);
    rect
}

fn handle_keys(ed: &mut PageEditor, ctx: &egui::Context, i: usize, eid: Id, actions: &mut Vec<Action>) {
    let b = &ed.blocks[i];
    let kind = b.kind;
    let len = b.text.chars().count();
    let range = egui::TextEdit::load_state(ctx, eid).and_then(|s| s.cursor.char_range());
    let (cur, collapsed) = range.map(|r| (r.primary.index.0, r.primary.index == r.secondary.index)).unwrap_or((len, true));
    let key = |m: Modifiers, k: Key| ctx.input_mut(|inp| inp.consume_key(m, k));

    if let Some((sid, sel)) = ed.slash {
        if sid == b.id {
            let filter = b.text.trim_start_matches('/').to_lowercase();
            let opts: Vec<BlockKind> = SLASH.iter().copied().filter(|k| k.label().to_lowercase().contains(&filter)).collect();
            if key(Modifiers::NONE, Key::ArrowDown) {
                ed.slash = Some((sid, (sel + 1) % opts.len().max(1)));
                return;
            }
            if key(Modifiers::NONE, Key::ArrowUp) {
                ed.slash = Some((sid, (sel + opts.len().max(1) - 1) % opts.len().max(1)));
                return;
            }
            if key(Modifiers::NONE, Key::Escape) {
                ed.slash = None;
                return;
            }
            if key(Modifiers::NONE, Key::Enter) {
                if let Some(k) = opts.get(sel.min(opts.len().saturating_sub(1))) {
                    actions.push(Action::SlashPick(i, *k));
                }
                ed.slash = None;
                return;
            }
        }
    }

    for (k, m) in [(Key::B, "**"), (Key::I, "*"), (Key::U, "__"), (Key::E, "`")] {
        if !matches!(kind, BlockKind::Code | BlockKind::Formula) && key(Modifiers::COMMAND, k) {
            wrap_focused(ed, m);
        }
    }
    if !matches!(kind, BlockKind::Code | BlockKind::Formula) && key(Modifiers::COMMAND | Modifiers::SHIFT, Key::H) {
        wrap_focused(ed, "==");
    }
    let b = &ed.blocks[i];
    if kind != BlockKind::Code && kind != BlockKind::Formula && key(Modifiers::NONE, Key::Enter) {
        if b.kind.is_list() && b.text.is_empty() {
            // Enter numa lista vazia sai da lista (ou recua).
            if b.indent > 0 {
                ed.blocks[i].indent -= 1;
            } else {
                actions.push(Action::Convert(i, BlockKind::Paragraph));
            }
            ed.touch();
        } else {
            actions.push(Action::Split(i, cur));
        }
        return;
    }
    if matches!(kind, BlockKind::Code | BlockKind::Formula) && key(Modifiers::COMMAND, Key::Enter) {
        actions.push(Action::InsertAfter(i, BlockKind::Paragraph));
        return;
    }
    if collapsed && cur == 0 && key(Modifiers::NONE, Key::Backspace) {
        if b.kind.is_list() && b.indent > 0 {
            ed.blocks[i].indent -= 1;
            ed.touch();
        } else if b.kind != BlockKind::Paragraph {
            actions.push(Action::Convert(i, BlockKind::Paragraph));
        } else {
            actions.push(Action::MergePrev(i));
        }
        return;
    }
    if collapsed && cur == 0 && i > 0 && key(Modifiers::NONE, Key::ArrowUp) {
        actions.push(Action::FocusPrev(i));
        return;
    }
    if collapsed && cur >= len && key(Modifiers::NONE, Key::ArrowDown) {
        actions.push(Action::FocusNext(i));
        return;
    }
    if b.kind.is_list() {
        if key(Modifiers::NONE, Key::Tab) {
            ed.blocks[i].indent = (ed.blocks[i].indent + 1).min(5);
            ed.touch();
        } else if key(Modifiers::SHIFT, Key::Tab) {
            ed.blocks[i].indent = ed.blocks[i].indent.saturating_sub(1);
            ed.touch();
        }
    }
}

fn markdown_shortcuts(ed: &mut PageEditor, i: usize) {
    let b = &mut ed.blocks[i];
    if b.kind != BlockKind::Paragraph {
        return;
    }
    let rules: [(&str, BlockKind); 10] = [
        ("# ", BlockKind::H1),
        ("## ", BlockKind::H2),
        ("### ", BlockKind::H3),
        ("- ", BlockKind::Bullet),
        ("* ", BlockKind::Bullet),
        ("1. ", BlockKind::Numbered),
        ("[] ", BlockKind::Check),
        ("[ ] ", BlockKind::Check),
        ("> ", BlockKind::Quote),
        ("```", BlockKind::Code),
    ];
    for (prefix, kind) in rules {
        if b.text.starts_with(prefix) {
            b.text = b.text[prefix.len()..].to_owned();
            b.kind = kind;
            let id = b.id;
            ed.focus_block(id, 0, 0);
            return;
        }
    }
    if b.text == "---" {
        b.text.clear();
        b.kind = BlockKind::Divider;
        let nb = Block::new(BlockKind::Paragraph, "");
        let nid = nb.id;
        ed.blocks.insert(i + 1, nb);
        ed.focus_block(nid, 0, 0);
    }
}

fn link_at(text: &str, char_idx: usize) -> Option<String> {
    let mut pos = 0;
    for r in rich::parse(text) {
        let n = r.text.chars().count();
        if r.style.link && !r.marker && (pos..=pos + n).contains(&char_idx) {
            return Some(r.text);
        }
        pos += n;
    }
    None
}

fn slash_menu(ed: &mut PageEditor, ui: &mut Ui, i: usize, rect: Rect, sel: usize, actions: &mut Vec<Action>) {
    let filter = ed.blocks[i].text.trim_start_matches('/').to_lowercase();
    let opts: Vec<BlockKind> = SLASH.iter().copied().filter(|k| k.label().to_lowercase().contains(&filter)).collect();
    if opts.is_empty() {
        return;
    }
    let sel = sel.min(opts.len() - 1);
    egui::Area::new(Id::unique(("slash", ed.blocks[i].id))).order(egui::Order::Foreground).fixed_pos(pos2(rect.left(), rect.top() + 30.0)).show(ui.ctx(), |ui| {
        popup_frame().show(ui, |ui| {
            ui.set_width(240.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            caps_label(ui, &tr!("page.blocks"));
            ui.add_space(4.0);
            for (j, k) in opts.iter().enumerate() {
                if menu_item(ui, &k.label(), j == sel).clicked() {
                    actions.push(Action::SlashPick(i, *k));
                    ed.slash = None;
                }
            }
        });
    });
}

/// "+" e "⋮⋮" à esquerda do bloco sob o mouse.
fn gutter(ed: &mut PageEditor, ui: &mut Ui, i: usize, rect: Rect, actions: &mut Vec<Action>) {
    let p = pal();
    let hover_zone = Rect::from_min_max(rect.min - vec2(56.0, 0.0), rect.max);
    let handle_id = Id::unique(("handle", ed.blocks[i].id));
    // Com o menu aberto a alça continua existindo mesmo com o mouse fora do
    // bloco; sem ela o menu, que nasce da alça, fecharia no meio do caminho.
    let menu_open = egui::Popup::is_id_open(ui.ctx(), handle_id.with("popup"));
    let hovered = ui.rect_contains_pointer(hover_zone) || ed.drag == Some(ed.blocks[i].id) || menu_open;
    if !hovered {
        return;
    }
    let y = rect.top() + 13.0;
    let plus = Rect::from_center_size(pos2(rect.left() - 40.0, y), vec2(20.0, 22.0));
    let handle = Rect::from_center_size(pos2(rect.left() - 20.0, y), vec2(18.0, 22.0));
    let pr = ui.interact(plus, Id::unique(("plus", ed.blocks[i].id)), Sense::click());
    let hr = ui.interact(handle, handle_id, Sense::click_and_drag());
    text(ui.painter(), plus.center(), Align2::CENTER_CENTER, "+", serif(15.0, W::Regular), if pr.hovered() { p.text } else { p.dim }, 20.0);
    grip_icon(ui.painter(), handle.center(), if hr.hovered() { p.text } else { p.dim });
    a11y(&pr, egui::accesskit::Role::Button, &tr!("page.insert_below"), None);
    a11y(&hr, egui::accesskit::Role::Button, &tr!("page.drag_or_menu"), None);
    if pr.on_hover_text(tr!("page.insert_below")).clicked() {
        actions.push(Action::InsertAfter(i, BlockKind::Paragraph));
        actions.push(Action::SlashPick(usize::MAX, BlockKind::Paragraph));
    }
    if hr.drag_started() {
        ed.drag = Some(ed.blocks[i].id);
    }
    let hr = hr.on_hover_cursor(egui::CursorIcon::Grab);
    egui::Popup::menu(&hr).frame(popup_frame()).width(230.0).show(|ui| {
        ui.menu_button(tr!("page.turn_into"), |ui| {
            for k in [BlockKind::Paragraph, BlockKind::H1, BlockKind::H2, BlockKind::H3, BlockKind::Bullet, BlockKind::Numbered, BlockKind::Check, BlockKind::Quote, BlockKind::Callout, BlockKind::Code, BlockKind::Formula] {
                if menu_item(ui, &k.label(), ed.blocks[i].kind == k).clicked() {
                    actions.push(Action::Convert(i, k));
                }
            }
        });
        if menu_item(ui, &tr!("page.flashcard_from_block"), false).clicked() {
            actions.push(Action::Flashcard(i));
        }
        if menu_item(ui, &tr!("ai.from_this_block"), false).clicked() {
            actions.push(Action::AiCards(Some(i)));
        }
        if menu_item(ui, &tr!("page.duplicate"), false).clicked() {
            actions.push(Action::Duplicate(i));
        }
        if i > 0 && menu_item(ui, &tr!("common.move_up"), false).clicked() {
            actions.push(Action::Move(i, i - 1));
        }
        if i + 1 < ed.blocks.len() && menu_item(ui, &tr!("common.move_down"), false).clicked() {
            actions.push(Action::Move(i, i + 2));
        }
        menu_sep(ui);
        if menu_item(ui, &tr!("page.delete_block"), false).clicked() {
            actions.push(Action::Delete(i));
        }
    });
}

fn image_block(app: &mut App, ed: &mut PageEditor, ui: &mut Ui, i: usize, w: f32, actions: &mut Vec<Action>) {
    let p = pal();
    let b = ed.blocks[i].clone();
    let media = app.db.media_dir();
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let max_w = (w * b.width.clamp(0.25, 1.0)).max(80.0);
        let src = crate::media::uri(&media, &b.media);
        // Sem arquivo, arquivo sumido ou que não decodifica: um espaço reservado
        // com as ações (antes não havia como remover um bloco assim).
        let problem = if b.media.is_empty() {
            Some(tr!("page.image_pick"))
        } else if !media.join(&b.media).exists() {
            Some(tr!("page.image_missing"))
        } else if egui::Image::new(src.clone()).load_for_size(ui.ctx(), vec2(max_w, 520.0)).is_err() {
            Some(tr!("page.image_broken"))
        } else {
            None
        };
        if let Some(msg) = problem {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 128.0), Sense::click());
            ui.painter().rect(r, CornerRadius::same(10), p.card, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
            text(ui.painter(), r.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, &msg, serif(13.0, W::Italic), p.muted, w - 20.0);
            let bar = Rect::from_center_size(r.center() + vec2(0.0, 22.0), vec2(220.0, 30.0));
            let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
            bu.spacing_mut().item_spacing.x = 8.0;
            if button(&mut bu, &tr!("page.choose_image"), Btn::Secondary, 28.0).clicked() {
                pick_image(app, ed, i);
            }
            if button(&mut bu, &tr!("page.remove"), Btn::Danger, 28.0).clicked() {
                actions.push(Action::Delete(i));
            }
            if resp.clicked() && b.media.is_empty() {
                pick_image(app, ed, i);
            }
            return;
        }
        let img = richtext::image(ui, &src, max_w, 520.0, 10, Sense::click());
        if img.clicked() {
            actions.push(Action::Viewer(b.media.clone()));
        }
        let img = img.on_hover_cursor(egui::CursorIcon::ZoomIn);
        // Barra flutuante: tamanho e remover.
        let zone = img.rect.expand(4.0);
        if ui.rect_contains_pointer(zone) {
            let bar = Rect::from_min_size(pos2(img.rect.right() - 196.0, img.rect.top() + 8.0), vec2(188.0, 28.0));
            ui.painter().rect(bar, CornerRadius::same(8), p.card, Stroke::new(1.0, p.border_control), StrokeKind::Inside);
            for (j, (lab, wv)) in [(tr!("page.size_s"), 0.4f32), (tr!("page.size_m"), 0.6), (tr!("page.size_l"), 0.8), ("100%".to_owned(), 1.0)].into_iter().enumerate() {
                let lab = lab.as_str();
                let r = Rect::from_min_size(pos2(bar.left() + 4.0 + j as f32 * 36.0, bar.top() + 3.0), vec2(34.0, 22.0));
                let resp = ui.interact(r, Id::unique(("imgw", b.id, j)), Sense::click());
                let on = (b.width - wv).abs() < 0.05;
                if on || resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), if on { p.accent_soft } else { p.hover });
                }
                text(ui.painter(), r.center(), Align2::CENTER_CENTER, lab, serif(11.5, W::Regular), if on { p.accent } else { p.text2 }, 34.0);
                if resp.clicked() {
                    ed.blocks[i].width = wv;
                    ed.touch();
                }
            }
            let r = Rect::from_min_size(pos2(bar.right() - 38.0, bar.top() + 3.0), vec2(34.0, 22.0));
            let resp = ui.interact(r, Id::unique(("imgdel", b.id)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), soft(p.again, 0.15));
            }
            close_icon(ui.painter(), r.center(), 12.0, p.again);
            if resp.on_hover_text(tr!("page.remove_image")).clicked() {
                actions.push(Action::Delete(i));
            }
        }
        let mut cap = ed.blocks[i].caption.clone();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut cap)
                .id(block_id(b.id))
                .frame(egui::Frame::NONE)
                .font(serif(13.0, W::Italic))
                .text_color(p.muted)
                .align(egui::Align2::CENTER_TOP)
                .desired_width(w)
                .hint_text(egui::RichText::new(tr!("page.caption")).italics().color(p.dim)),
        );
        if resp.changed() {
            ed.blocks[i].caption = cap;
            ed.touch();
        }
        if resp.has_focus() {
            ed.focus = Some(b.id);
            // Backspace/Delete na legenda vazia apaga a imagem, como num parágrafo vazio.
            if ed.blocks[i].caption.is_empty() && ui.ctx().input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Backspace) || inp.consume_key(Modifiers::NONE, Key::Delete)) {
                actions.push(Action::Delete(i));
            }
        }
    });
}

fn pick_image(app: &mut App, ed: &mut PageEditor, i: usize) {
    if let Some(path) = rfd::FileDialog::new().add_filter(tr!("note.images_filter"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"]).pick_file() {
        match crate::media::store_file(&app.db.media_dir(), &path) {
            Ok(s) => {
                let _ = app.db.add_media(&s);
                ed.blocks[i].media = s.file_name;
                if ed.blocks[i].width <= 0.0 {
                    ed.blocks[i].width = 0.75;
                }
                ed.touch();
            }
            Err(e) => app.error(e),
        }
    }
}

fn table_block(ed: &mut PageEditor, ui: &mut Ui, i: usize, w: f32) -> bool {
    let p = pal();
    let mut changed = false;
    let id = ed.blocks[i].id;
    if ed.blocks[i].rows.is_empty() {
        ed.blocks[i].rows = vec![vec![String::new(); 3]; 3];
    }
    let cols = ed.blocks[i].rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    for r in ed.blocks[i].rows.iter_mut() {
        r.resize(cols, String::new());
    }
    let cw = w / cols as f32;
    let resp = egui::Frame::NONE.stroke(Stroke::new(1.0, p.border)).corner_radius(8).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let nrows = ed.blocks[i].rows.len();
        for ri in 0..nrows {
            let (rr, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
            if ri == 0 {
                ui.painter().rect_filled(rr, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.chip);
            }
            if ri + 1 < nrows {
                ui.painter().hline(rr.x_range(), rr.bottom(), Stroke::new(1.0, p.line_row));
            }
            for ci in 0..cols {
                let cell = Rect::from_min_size(pos2(rr.left() + ci as f32 * cw, rr.top()), vec2(cw, 34.0));
                if ci > 0 {
                    ui.painter().vline(cell.left(), cell.y_range(), Stroke::new(1.0, p.line_row));
                }
                let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(cell.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let r = cu.add(
                    egui::TextEdit::singleline(&mut ed.blocks[i].rows[ri][ci])
                        .id(Id::unique(("cell", id, ri, ci)))
                        .frame(egui::Frame::NONE)
                        .font(serif(14.0, if ri == 0 { W::Semi } else { W::Regular }))
                        .text_color(if ri == 0 { p.text } else { p.text2 })
                        .desired_width(cw - 20.0),
                );
                changed |= r.changed();
                if r.has_focus() {
                    ed.focus = Some(id);
                }
            }
        }
    });
    if ui.rect_contains_pointer(resp.response.rect.expand2(vec2(0.0, 30.0))) || ed.focus == Some(id) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if button(ui, &tr!("page.add_row"), Btn::Ghost, 24.0).clicked() {
                ed.blocks[i].rows.push(vec![String::new(); cols]);
                changed = true;
            }
            if button(ui, &tr!("page.add_col"), Btn::Ghost, 24.0).clicked() {
                for r in ed.blocks[i].rows.iter_mut() {
                    r.push(String::new());
                }
                changed = true;
            }
            if ed.blocks[i].rows.len() > 1 && button(ui, &tr!("page.del_row"), Btn::Ghost, 24.0).clicked() {
                ed.blocks[i].rows.pop();
                changed = true;
            }
            if cols > 1 && button(ui, &tr!("page.del_col"), Btn::Ghost, 24.0).clicked() {
                for r in ed.blocks[i].rows.iter_mut() {
                    r.pop();
                }
                changed = true;
            }
        });
    }
    changed
}

fn apply_actions(app: &mut App, ed: &mut PageEditor, actions: Vec<Action>, sid: i64) -> Option<i64> {
    let mut open_slash_on_new = false;
    for a in actions {
        match a {
            Action::Split(i, cur) => {
                let b = &ed.blocks[i];
                let at = rich::byte_of_char(&b.text, cur);
                let rest = b.text[at..].to_owned();
                // Lista continua lista; título vira parágrafo.
                let kind = if b.kind.is_list() { b.kind } else { BlockKind::Paragraph };
                let indent = if b.kind.is_list() { b.indent } else { 0 };
                ed.blocks[i].text.truncate(at);
                let mut nb = Block::new(kind, rest);
                nb.indent = indent;
                let nid = nb.id;
                ed.blocks.insert(i + 1, nb);
                ed.focus_block(nid, 0, 0);
                ed.touch();
            }
            Action::MergePrev(i) => {
                if i == 0 {
                    continue;
                }
                let prev = (0..i).rev().find(|j| ed.blocks[*j].kind.is_text());
                let cur = ed.blocks.remove(i);
                match prev {
                    Some(j) if !matches!(ed.blocks[j].kind, BlockKind::Code | BlockKind::Formula) => {
                        let pos = ed.blocks[j].text.chars().count();
                        ed.blocks[j].text.push_str(&cur.text);
                        let id = ed.blocks[j].id;
                        ed.focus_block(id, pos, pos);
                    }
                    _ => {
                        // Bloco anterior não é texto (imagem, tabela): só apaga se vazio.
                        if !cur.text.is_empty() {
                            ed.blocks.insert(i, cur);
                        } else if let Some(prev) = ed.blocks.get(i - 1) {
                            ed.focus = Some(prev.id);
                        }
                    }
                }
                ed.touch();
            }
            Action::FocusPrev(i) => {
                if let Some(j) = (0..i).rev().find(|j| ed.blocks[*j].kind.is_text()) {
                    let n = ed.blocks[j].text.chars().count();
                    let id = ed.blocks[j].id;
                    ed.focus_block(id, n, n);
                } else {
                    ed.focus_title = false;
                }
            }
            Action::FocusNext(i) => {
                if let Some(j) = (i + 1..ed.blocks.len()).find(|j| ed.blocks[*j].kind.is_text()) {
                    let id = ed.blocks[j].id;
                    ed.focus_block(id, 0, 0);
                }
            }
            Action::InsertAfter(i, kind) => {
                let mut nb = Block::new(kind, "");
                if kind == BlockKind::Table {
                    nb.rows = vec![vec![String::new(); 3]; 3];
                }
                let nid = nb.id;
                let pos = (i + 1).min(ed.blocks.len());
                ed.blocks.insert(pos, nb);
                if kind == BlockKind::Image {
                    pick_image(app, ed, pos);
                } else if kind == BlockKind::Divider {
                    let p2 = Block::new(BlockKind::Paragraph, "");
                    let p2id = p2.id;
                    ed.blocks.insert(pos + 1, p2);
                    ed.focus_block(p2id, 0, 0);
                } else {
                    ed.focus_block(nid, 0, 0);
                }
                ed.touch();
                open_slash_on_new = false;
            }
            Action::SlashPick(i, kind) => {
                if i == usize::MAX {
                    open_slash_on_new = true;
                    continue;
                }
                if i < ed.blocks.len() {
                    ed.blocks[i].text.clear();
                    convert(ed, i, kind);
                    match kind {
                        BlockKind::Image => pick_image(app, ed, i),
                        BlockKind::Divider => {
                            let nb = Block::new(BlockKind::Paragraph, "");
                            let nid = nb.id;
                            ed.blocks.insert(i + 1, nb);
                            ed.focus_block(nid, 0, 0);
                        }
                        _ => {
                            let id = ed.blocks[i].id;
                            ed.focus_block(id, 0, 0);
                        }
                    }
                    ed.slash = None;
                    ed.touch();
                }
            }
            Action::Convert(i, kind) => {
                if i < ed.blocks.len() {
                    convert(ed, i, kind);
                    let id = ed.blocks[i].id;
                    let n = ed.blocks[i].text.chars().count();
                    if kind.is_text() {
                        ed.focus_block(id, n, n);
                    }
                    ed.touch();
                }
            }
            Action::Delete(i) => {
                if i < ed.blocks.len() {
                    ed.blocks.remove(i);
                    if ed.blocks.is_empty() {
                        ed.blocks.push(Block::new(BlockKind::Paragraph, ""));
                    }
                    ed.touch();
                }
            }
            Action::Duplicate(i) => {
                let mut b = ed.blocks[i].clone();
                b.id = crate::db::new_id();
                ed.blocks.insert(i + 1, b);
                ed.touch();
            }
            Action::Move(from, to) => {
                if from < ed.blocks.len() {
                    let b = ed.blocks.remove(from);
                    let to = if to > from { to - 1 } else { to }.min(ed.blocks.len());
                    ed.blocks.insert(to, b);
                    ed.touch();
                }
            }
            Action::Flashcard(i) => {
                ed.flush(&app.db);
                let Some(b) = ed.blocks.get(i) else { continue };
                let selected = match ed.sel {
                    Some((id, r)) if id == b.id && r.primary.index != r.secondary.index => {
                        let (a, c) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
                        let t = &b.text;
                        Some(t[rich::byte_of_char(t, a)..rich::byte_of_char(t, c)].to_owned())
                    }
                    _ => None,
                };
                let text = selected.unwrap_or_else(|| match b.kind {
                    BlockKind::Image => rich::image_markup(&b.media),
                    BlockKind::Table => b.plain(),
                    _ => b.text.clone(),
                });
                let deck = app.default_deck().filter(|d| app.deck(*d).is_some_and(|d| d.subject_id == sid)).or_else(|| app.root_decks(sid).first().map(|d| d.id));
                match deck {
                    Some(d) => {
                        app.note_editor = Some(super::note_editor::NoteEditor::new_note(d, Some((ed.meta.id, Some(b.id), text, ed.title.clone()))));
                    }
                    None => app.toast(tr!("toast.create_deck_first")),
                }
            }
            Action::AiCards(at) => {
                let scope = match at {
                    Some(i) => ed.blocks.get(i).map(|b| super::ai::Scope::Block(b.id)),
                    // Seleção dentro de um bloco de texto, se houver.
                    None => match ed.sel {
                        Some((id, r)) if r.primary.index != r.secondary.index => ed.idx(id).map(|i| {
                            let t = &ed.blocks[i].text;
                            let (a, c) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
                            super::ai::Scope::Selection(id, t[rich::byte_of_char(t, a)..rich::byte_of_char(t, c)].to_owned())
                        }),
                        _ => None,
                    },
                };
                ed.ai_request = Some(scope.unwrap_or(super::ai::Scope::Page));
            }
            Action::OpenLink(target) => {
                ed.flush(&app.db);
                match app.db.page_by_title(&target) {
                    Ok(Some(pid)) => return Some(pid),
                    _ => {
                        if let Ok(id) = app.db.add_page(sid, None, &target) {
                            app.data_version += 1;
                            app.toast(tr!("toast.page_created", title = target));
                            return Some(id);
                        }
                    }
                }
            }
            Action::Viewer(name) => {
                app.modal = Some(super::modals::Modal::Viewer(name, 1.0));
            }
        }
    }
    if open_slash_on_new {
        if let Some((id, _, _)) = ed.pending {
            if let Some(i) = ed.idx(id) {
                ed.blocks[i].text = "/".into();
                ed.slash = Some((id, 0));
                ed.pending = Some((id, 1, 1));
            }
        }
    }
    None
}

fn convert(ed: &mut PageEditor, i: usize, kind: BlockKind) {
    let b = &mut ed.blocks[i];
    if kind == BlockKind::Table && b.rows.is_empty() {
        b.rows = vec![vec![String::new(); 3]; 3];
    }
    if !kind.is_list() {
        b.indent = 0;
    }
    b.kind = kind;
}

// ───────────────────────────── painel direito ─────────────────────────────

fn page_aside(app: &mut App, ui: &mut Ui) {
    let p = pal();
    let Some(ed) = app.page.as_ref() else { return };
    let pid = ed.meta.id;
    let version = app.data_version;
    let need = ed.aside.as_ref().is_none_or(|(v, _)| *v != version);
    if need {
        let backlinks = app.db.backlinks(pid).unwrap_or_default();
        let linked = app.db.notes_for_page(pid).unwrap_or_default().into_iter().map(|n| (n.clone(), app.db.note_cards(n.id).unwrap_or_default())).collect();
        if let Some(ed) = app.page.as_mut() {
            ed.aside = Some((version, Aside { backlinks, linked }));
        }
    }
    let Some(ed) = app.page.as_ref() else { return };
    let headings: Vec<(i64, BlockKind, String)> = ed.blocks.iter().filter(|b| matches!(b.kind, BlockKind::H1 | BlockKind::H2 | BlockKind::H3)).map(|b| (b.id, b.kind, rich::plain(&b.text))).collect();
    let title = ed.title.clone();
    let (backlinks, linked) = ed.aside.as_ref().map(|(_, a)| (a.backlinks.clone(), a.linked.clone())).unwrap_or_default();
    ui.spacing_mut().item_spacing.y = 6.0;
    egui::ScrollArea::vertical().id_salt("aside").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        caps_label(ui, &tr!("page.outline"));
        ui.add_space(2.0);
        if link_row(ui, &title, 0.0, p.text).clicked() {
            app.page.as_mut().unwrap().scroll_to = app.page.as_ref().unwrap().blocks.first().map(|b| b.id);
        }
        for (id, k, t) in &headings {
            let indent = match k {
                BlockKind::H1 => 12.0,
                BlockKind::H2 => 12.0,
                _ => 24.0,
            };
            if link_row(ui, t, indent, p.muted).clicked() {
                app.page.as_mut().unwrap().scroll_to = Some(*id);
            }
        }
        ui.add_space(16.0);
        caps_label(ui, &tr!("page.backlinks", n = backlinks.len()));
        ui.add_space(2.0);
        if backlinks.is_empty() {
            label(ui, &tr!("page.no_backlinks"), serif(12.0, W::Italic), p.dim);
        }
        for b in &backlinks {
            if link_row(ui, &b.title, 0.0, p.text2).clicked() {
                app.open_page(b.id);
                return;
            }
        }
        ui.add_space(16.0);
        let all_cards: Vec<Card> = linked.iter().flat_map(|(_, c)| c.clone()).collect();
        caps_label(ui, &tr!("page.linked_cards", n = all_cards.len()));
        ui.add_space(2.0);
        if linked.is_empty() {
            label(ui, &tr!("page.linked_hint"), serif(12.0, W::Italic), p.dim);
        }
        let now = srs::now_ms();
        for (note, cards) in linked.iter().take(12) {
            for c in cards.iter().take(3) {
                let resp = egui::Frame::NONE.fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let face = crate::cards::render(note.kind, &note.fields, c.ord);
                    let q = rich::plain(&face.question);
                    let q = if q.trim().is_empty() { note.kind.label() } else { q };
                    para(ui, &q.chars().take(110).collect::<String>(), serif(13.0, W::Regular), p.text);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                    let (st, col) = match c.s.state {
                        State::New => (c.s.state.label(), p.easy),
                        State::Learning | State::Relearning => (c.s.state.label(), p.hard),
                        State::Review => match srs::retrievability(&c.s, app.decay_of_deck(c.deck_id), &app.clock, now) {
                            Some(r) => (tr!("page.review_r", pct = pct(r, 0)), p.good),
                            None => (c.s.state.label(), p.good),
                        },
                    };
                    text(ui.painter(), r.left_center(), Align2::LEFT_CENTER, &st, serif(11.5, W::Regular), col, r.width() * 0.6);
                    let due = if c.s.state == State::New { "—".to_owned() } else if c.s.due <= now { tr!("page.due_now") } else { tr!("page.due_in", interval = srs::interval_label(c.s.due - now)) };
                    text(ui.painter(), r.right_center(), Align2::RIGHT_CENTER, &due, serif(11.5, W::Regular), p.muted, r.width() * 0.4);
                });
                let resp = ui.interact(resp.response.rect, Id::unique(("linked", c.id)), Sense::click());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.note_editor = Some(super::note_editor::NoteEditor::edit(note.clone(), c.deck_id));
                }
            }
        }
        ui.add_space(2.0);
        let w = ui.available_width();
        if !all_cards.is_empty() && button_sized(ui, &tr_n!("page.study_these", all_cards.len()), None, Btn::Secondary, vec2(w, 32.0), true).clicked() {
            let ids: Vec<i64> = all_cards.iter().map(|c| c.id).collect();
            review::start(app, review::Scope::Cards(ids, tr!("page.cards_of", title = title)));
        }
        if button_sized(ui, &tr!("ai.button"), None, Btn::Ghost, vec2(w, 32.0), true).on_hover_text(keys(&tr!("ai.button_tip"))).clicked() {
            super::ai::open(app, super::ai::Scope::Page);
        }
    });
}

fn link_row(ui: &mut Ui, s: &str, indent: f32, color: Color32) -> egui::Response {
    let p = pal();
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
    let c = if resp.hovered() { p.accent } else { color };
    text(ui.painter(), pos2(r.left() + indent, r.center().y), Align2::LEFT_CENTER, &if s.trim().is_empty() { tr!("page.untitled") } else { s.to_owned() }, serif(13.0, W::Regular), c, r.width() - indent);
    a11y(&resp, egui::accesskit::Role::Link, s, None);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ───────────────────────────── Markdown ─────────────────────────────

pub fn to_markdown(title: &str, blocks: &[Block], media_prefix: &str) -> String {
    let mut out = format!("# {title}\n\n");
    let mut n = 0;
    for b in blocks {
        let ind = "    ".repeat(b.indent as usize);
        if b.kind != BlockKind::Numbered {
            n = 0;
        }
        match b.kind {
            BlockKind::Paragraph => out.push_str(&format!("{}\n\n", b.text)),
            BlockKind::H1 => out.push_str(&format!("## {}\n\n", b.text)),
            BlockKind::H2 => out.push_str(&format!("### {}\n\n", b.text)),
            BlockKind::H3 => out.push_str(&format!("#### {}\n\n", b.text)),
            BlockKind::Bullet => out.push_str(&format!("{ind}- {}\n", b.text)),
            BlockKind::Numbered => {
                n += 1;
                out.push_str(&format!("{ind}{n}. {}\n", b.text))
            }
            BlockKind::Check => out.push_str(&format!("{ind}- [{}] {}\n", if b.checked { "x" } else { " " }, b.text)),
            BlockKind::Quote => out.push_str(&format!("> {}\n\n", b.text.replace('\n', "\n> "))),
            BlockKind::Callout => out.push_str(&format!("> **{}** {}\n\n", tr!("page.md_tip"), b.text.replace('\n', "\n> "))),
            BlockKind::Code => out.push_str(&format!("```{}\n{}\n```\n\n", b.lang, b.text)),
            BlockKind::Formula => out.push_str(&format!("$$\n{}\n$$\n\n", b.text)),
            BlockKind::Divider => out.push_str("---\n\n"),
            BlockKind::Image => out.push_str(&format!("![{}]({media_prefix}{})\n\n", b.caption, b.media)),
            BlockKind::Table => {
                for (i, r) in b.rows.iter().enumerate() {
                    out.push_str(&format!("| {} |\n", r.join(" | ")));
                    if i == 0 {
                        out.push_str(&format!("|{}\n", " --- |".repeat(r.len())));
                    }
                }
                out.push('\n');
            }
        }
    }
    out.replace("\n\n\n", "\n\n")
}

pub fn from_markdown(md: &str, media_dir: &std::path::Path, base: &std::path::Path) -> (Option<String>, Vec<Block>) {
    let mut title = None;
    let mut blocks = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        let indent = (line.len() - line.trim_start().len()) / 2;
        let s = t.trim_start();
        if s.is_empty() {
            continue;
        }
        if let Some(h) = s.strip_prefix("# ") {
            if title.is_none() && blocks.is_empty() {
                title = Some(h.to_owned());
            } else {
                blocks.push(Block::new(BlockKind::H1, h));
            }
        } else if let Some(h) = s.strip_prefix("## ") {
            blocks.push(Block::new(BlockKind::H1, h));
        } else if let Some(h) = s.strip_prefix("### ") {
            blocks.push(Block::new(BlockKind::H2, h));
        } else if let Some(h) = s.strip_prefix("#### ") {
            blocks.push(Block::new(BlockKind::H3, h));
        } else if s.starts_with("```") {
            let lang = s.trim_start_matches('`').to_owned();
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l.to_owned());
            }
            let mut b = Block::new(BlockKind::Code, code.join("\n"));
            b.lang = lang;
            blocks.push(b);
        } else if s == "$$" {
            let mut f = Vec::new();
            for l in lines.by_ref() {
                if l.trim() == "$$" {
                    break;
                }
                f.push(l.to_owned());
            }
            blocks.push(Block::new(BlockKind::Formula, f.join("\n")));
        } else if s == "---" || s == "***" {
            blocks.push(Block::new(BlockKind::Divider, ""));
        } else if let Some(rest) = s.strip_prefix("- [x] ").or_else(|| s.strip_prefix("- [X] ")) {
            let mut b = Block::new(BlockKind::Check, rest);
            b.checked = true;
            b.indent = indent.min(5) as u8;
            blocks.push(b);
        } else if let Some(rest) = s.strip_prefix("- [ ] ") {
            let mut b = Block::new(BlockKind::Check, rest);
            b.indent = indent.min(5) as u8;
            blocks.push(b);
        } else if let Some(rest) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")) {
            let mut b = Block::new(BlockKind::Bullet, rest);
            b.indent = indent.min(5) as u8;
            blocks.push(b);
        } else if s.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
            let mut b = Block::new(BlockKind::Numbered, s.split_once(". ").unwrap().1);
            b.indent = indent.min(5) as u8;
            blocks.push(b);
        } else if let Some(q) = s.strip_prefix("> ") {
            blocks.push(Block::new(BlockKind::Quote, q));
        } else if s.starts_with('|') {
            let mut rows = vec![s.trim_matches('|').split('|').map(|c| c.trim().to_owned()).collect::<Vec<_>>()];
            while lines.peek().is_some_and(|l| l.trim_start().starts_with('|')) {
                let l = lines.next().unwrap().trim().trim_matches('|').to_owned();
                if l.chars().all(|c| matches!(c, '-' | ':' | '|' | ' ')) {
                    continue;
                }
                rows.push(l.split('|').map(|c| c.trim().to_owned()).collect());
            }
            let mut b = Block::new(BlockKind::Table, "");
            b.rows = rows;
            blocks.push(b);
        } else if let Some(name) = rich::image_line(s) {
            let caption = s.strip_prefix("![").and_then(|r| r.split_once("](")).map(|(c, _)| c.to_owned()).unwrap_or_default();
            let path = base.join(&name);
            let mut b = Block::new(BlockKind::Image, "");
            if let Ok(st) = crate::media::store_file(media_dir, &path) {
                b.media = st.file_name;
            }
            b.caption = caption;
            b.width = 0.75;
            blocks.push(b);
        } else {
            blocks.push(Block::new(BlockKind::Paragraph, s));
        }
    }
    (title, blocks)
}

fn export_markdown(app: &mut App, ed: &PageEditor) {
    let safe: String = ed.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.md", safe.trim())).add_filter("Markdown", &["md"]).save_file() else { return };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "page".into());
    let media_folder = format!("{stem}{}", tr!("page.md_images_suffix"));
    let has_images = ed.blocks.iter().any(|b| b.kind == BlockKind::Image && !b.media.is_empty());
    let md = to_markdown(&ed.title, &ed.blocks, &format!("{media_folder}/"));
    let r = (|| -> anyhow::Result<()> {
        std::fs::write(&path, md)?;
        if has_images {
            let dir = path.with_file_name(&media_folder);
            std::fs::create_dir_all(&dir)?;
            for b in ed.blocks.iter().filter(|b| b.kind == BlockKind::Image && !b.media.is_empty()) {
                let _ = std::fs::copy(app.db.media_dir().join(&b.media), dir.join(&b.media));
            }
        }
        Ok(())
    })();
    match r {
        Ok(()) => app.toast(tr!("toast.exported", path = path.display())),
        Err(e) => app.error(e),
    }
}

fn import_markdown(app: &mut App, sid: i64) {
    let Some(paths) = rfd::FileDialog::new().add_filter("Markdown", &["md", "markdown", "txt"]).pick_files() else { return };
    let mut last = None;
    for path in paths {
        let Ok(md) = std::fs::read_to_string(&path) else { continue };
        let base = path.parent().map(|p| p.to_owned()).unwrap_or_default();
        let (title, blocks) = from_markdown(&md, &app.db.media_dir(), &base);
        let title = title.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| tr!("page.imported_title")));
        if let Ok(id) = app.db.add_page(sid, None, &title) {
            let _ = app.db.save_blocks(id, &title, &blocks);
            last = Some(id);
        }
    }
    if let Some(id) = last {
        app.data_version += 1;
        app.open_page(id);
        app.toast(tr!("toast.markdown_imported"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_roundtrip() {
        let blocks = vec![
            Block::new(BlockKind::H1, "Fases"),
            Block::new(BlockKind::Numbered, "**Prófase** — condensa"),
            Block::new(BlockKind::Numbered, "Metáfase"),
            Block::new(BlockKind::Quote, "citação"),
            Block::new(BlockKind::Divider, ""),
        ];
        let md = to_markdown("Mitose", &blocks, "");
        let (title, back) = from_markdown(&md, std::path::Path::new("."), std::path::Path::new("."));
        assert_eq!(title.as_deref(), Some("Mitose"));
        let kinds: Vec<BlockKind> = back.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, vec![BlockKind::H1, BlockKind::Numbered, BlockKind::Numbered, BlockKind::Quote, BlockKind::Divider]);
        assert_eq!(back[1].text, "**Prófase** — condensa");
    }

    #[test]
    fn link_lookup() {
        assert_eq!(link_at("ver [[Meiose]] já", 6).as_deref(), Some("Meiose"));
        assert_eq!(link_at("ver [[Meiose]] já", 1), None);
    }
}

#[allow(dead_code)]
fn _kind_used(_: NoteKind) {}
