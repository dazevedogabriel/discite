//! Banco SQLite (`discite.db`) e todas as consultas.
//!
//! IDs são inteiros derivados do relógio (ms, sempre crescentes), como no
//! Anki: ordenáveis por criação e únicos o bastante para uma futura
//! sincronização. O `revlog` só recebe inserções; "desfazer" marca a linha.

use crate::cards::{self, NoteKind};
use crate::srs::{self, Clock, PresetConfig, Queue, ReviewKind, Sched, State};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

static LAST_ID: AtomicI64 = AtomicI64::new(0);

pub fn new_id() -> i64 {
    let now = srs::now_ms();
    let mut prev = LAST_ID.load(Ordering::Relaxed);
    loop {
        let next = now.max(prev + 1);
        match LAST_ID.compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(p) => prev = p,
        }
    }
}

/// Pasta de dados: `DISCITE_DATA` ou `%APPDATA%\Discite`.
pub fn data_root() -> PathBuf {
    if let Some(p) = std::env::var_os("DISCITE_DATA") {
        return PathBuf::from(p);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Discite")
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS subjects (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  color INTEGER NOT NULL,
  position INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pages (
  id INTEGER PRIMARY KEY,
  subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
  parent_id INTEGER,
  title TEXT NOT NULL,
  position REAL NOT NULL,
  tags TEXT NOT NULL DEFAULT '',
  collapsed INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_pages_subject ON pages(subject_id, deleted_at);

CREATE TABLE IF NOT EXISTS blocks (
  id INTEGER PRIMARY KEY,
  page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  kind TEXT NOT NULL,
  data TEXT NOT NULL,
  plain TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_blocks_page ON blocks(page_id, position);

CREATE TABLE IF NOT EXISTS page_versions (
  id INTEGER PRIMARY KEY,
  page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  title TEXT NOT NULL,
  snapshot TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS page_links (
  from_page INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  to_page INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  PRIMARY KEY (from_page, to_page)
);

CREATE TABLE IF NOT EXISTS media (
  id INTEGER PRIMARY KEY,
  file_name TEXT NOT NULL UNIQUE,
  mime TEXT NOT NULL,
  width INTEGER,
  height INTEGER,
  size INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS presets (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  config TEXT NOT NULL,
  params TEXT NOT NULL DEFAULT '[]',
  last_optimized_at INTEGER,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS decks (
  id INTEGER PRIMARY KEY,
  subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
  parent_id INTEGER,
  preset_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  collapsed INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  fields TEXT NOT NULL,
  sort_field TEXT NOT NULL,
  checksum INTEGER NOT NULL,
  tags TEXT NOT NULL DEFAULT '',
  source_page INTEGER,
  source_block INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notes_checksum ON notes(checksum);
CREATE INDEX IF NOT EXISTS idx_notes_page ON notes(source_page);

CREATE TABLE IF NOT EXISTS cards (
  id INTEGER PRIMARY KEY,
  note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  deck_id INTEGER NOT NULL,
  ord INTEGER NOT NULL,
  state INTEGER NOT NULL DEFAULT 0,
  queue INTEGER NOT NULL DEFAULT 0,
  due INTEGER NOT NULL,
  step INTEGER NOT NULL DEFAULT 0,
  stability REAL,
  difficulty REAL,
  ease REAL NOT NULL DEFAULT 0,
  interval REAL NOT NULL DEFAULT 0,
  reps INTEGER NOT NULL DEFAULT 0,
  lapses INTEGER NOT NULL DEFAULT 0,
  last_review INTEGER,
  flagged INTEGER NOT NULL DEFAULT 0,
  leech INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (note_id, ord)
);
CREATE INDEX IF NOT EXISTS idx_cards_due ON cards(deck_id, queue, state, due);
CREATE INDEX IF NOT EXISTS idx_cards_note ON cards(note_id);

CREATE TABLE IF NOT EXISTS revlog (
  id INTEGER PRIMARY KEY,
  card_id INTEGER NOT NULL,
  at INTEGER NOT NULL,
  rating INTEGER NOT NULL,
  kind INTEGER NOT NULL,
  elapsed_days REAL NOT NULL,
  scheduled_days REAL NOT NULL,
  duration_ms INTEGER NOT NULL,
  state_before INTEGER NOT NULL,
  state_after INTEGER NOT NULL,
  s_before REAL, s_after REAL,
  d_before REAL, d_after REAL,
  retrievability REAL,
  next_due INTEGER NOT NULL,
  algorithm TEXT NOT NULL,
  preset_id INTEGER NOT NULL,
  undone INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_revlog_card ON revlog(card_id, at);
CREATE INDEX IF NOT EXISTS idx_revlog_at ON revlog(at);

CREATE VIRTUAL TABLE IF NOT EXISTS fts_pages USING fts5(page_id UNINDEXED, block_id UNINDEXED, text, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE IF NOT EXISTS fts_notes USING fts5(note_id UNINDEXED, text, tokenize='unicode61 remove_diacritics 2');
"#;

pub struct Db {
    pub conn: Connection,
    pub root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    pub id: i64,
    pub name: String,
    pub color: usize,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageMeta {
    pub id: i64,
    pub subject_id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub position: f64,
    pub tags: String,
    pub collapsed: bool,
    pub updated_at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum BlockKind {
    #[default]
    Paragraph,
    H1,
    H2,
    H3,
    Bullet,
    Numbered,
    Check,
    Quote,
    Code,
    Callout,
    Image,
    Table,
    Divider,
    Formula,
}

impl BlockKind {
    pub fn is_text(self) -> bool {
        !matches!(self, BlockKind::Image | BlockKind::Table | BlockKind::Divider)
    }
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Bullet | BlockKind::Numbered | BlockKind::Check)
    }
    pub fn label(self) -> String {
        crate::tr!(match self {
            BlockKind::Paragraph => "block.paragraph",
            BlockKind::H1 => "block.h1",
            BlockKind::H2 => "block.h2",
            BlockKind::H3 => "block.h3",
            BlockKind::Bullet => "block.bullet",
            BlockKind::Numbered => "block.numbered",
            BlockKind::Check => "block.check",
            BlockKind::Quote => "block.quote",
            BlockKind::Code => "block.code",
            BlockKind::Callout => "block.callout",
            BlockKind::Image => "block.image",
            BlockKind::Table => "block.table",
            BlockKind::Divider => "block.divider",
            BlockKind::Formula => "block.formula",
        })
    }
}

/// Um bloco de página. Guardado inteiro como JSON em `blocks.data`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Block {
    pub id: i64,
    pub kind: BlockKind,
    pub text: String,
    pub indent: u8,
    pub checked: bool,
    /// Imagem: arquivo em `media/`, largura relativa (0–1) e legenda.
    pub media: String,
    pub width: f32,
    pub caption: String,
    /// Tabela: linhas × colunas (a primeira linha é o cabeçalho).
    pub rows: Vec<Vec<String>>,
    /// Código: linguagem; destaque: "dica", "aviso", "info".
    pub lang: String,
}

impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Block {
        Block { id: new_id(), kind, text: text.into(), width: 0.7, ..Default::default() }
    }
    pub fn plain(&self) -> String {
        match self.kind {
            BlockKind::Image => self.caption.clone(),
            BlockKind::Table => self.rows.iter().map(|r| r.join(" ")).collect::<Vec<_>>().join("\n"),
            BlockKind::Code | BlockKind::Formula => self.text.clone(),
            _ => crate::rich::plain(&self.text),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Deck {
    pub id: i64,
    pub subject_id: i64,
    pub parent_id: Option<i64>,
    pub preset_id: i64,
    pub name: String,
    pub position: i64,
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: i64,
    pub name: String,
    pub cfg: PresetConfig,
    pub params: Vec<f32>,
    pub last_optimized_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: i64,
    pub kind: NoteKind,
    pub fields: Vec<String>,
    /// Tags separadas por espaço.
    pub tags: String,
    pub source_page: Option<i64>,
    pub source_block: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Note {
    pub fn new(kind: NoteKind) -> Note {
        Note {
            id: 0,
            kind,
            fields: vec![String::new(); kind.field_names().len()],
            tags: String::new(),
            source_page: None,
            source_block: None,
            created_at: 0,
            updated_at: 0,
        }
    }
    pub fn tag_list(&self) -> Vec<String> {
        self.tags.split_whitespace().map(str::to_owned).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: i64,
    pub note_id: i64,
    pub deck_id: i64,
    pub ord: u32,
    pub s: Sched,
    pub queue: Queue,
    pub flagged: bool,
    pub leech: bool,
    pub position: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rev {
    pub id: i64,
    pub card_id: i64,
    pub at: i64,
    pub rating: u32,
    pub kind: ReviewKind,
    pub elapsed_days: f32,
    pub scheduled_days: f32,
    pub duration_ms: i64,
    pub state_before: State,
    pub state_after: State,
    pub s_before: Option<f32>,
    pub s_after: Option<f32>,
    pub d_before: Option<f32>,
    pub d_after: Option<f32>,
    pub retrievability: Option<f32>,
    pub next_due: i64,
    pub algorithm: String,
    pub preset_id: i64,
}

/// Contagens cruas por baralho (sem limites diários).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub new: u32,
    pub learn: u32,
    pub review: u32,
}

const CARD_COLS: &str = "id, note_id, deck_id, ord, state, queue, due, step, stability, difficulty, ease, interval, reps, lapses, last_review, flagged, leech, position, created_at";

fn card_from_row(r: &Row) -> rusqlite::Result<Card> {
    Ok(Card {
        id: r.get(0)?,
        note_id: r.get(1)?,
        deck_id: r.get(2)?,
        ord: r.get::<_, i64>(3)? as u32,
        s: Sched {
            state: State::from_i64(r.get(4)?),
            due: r.get(6)?,
            step: r.get::<_, i64>(7)? as u32,
            stability: r.get::<_, Option<f64>>(8)?.map(|v| v as f32),
            difficulty: r.get::<_, Option<f64>>(9)?.map(|v| v as f32),
            ease: r.get::<_, f64>(10)? as f32,
            interval: r.get::<_, f64>(11)? as f32,
            reps: r.get::<_, i64>(12)? as u32,
            lapses: r.get::<_, i64>(13)? as u32,
            last_review: r.get(14)?,
        },
        queue: Queue::from_i64(r.get(5)?),
        flagged: r.get::<_, i64>(15)? != 0,
        leech: r.get::<_, i64>(16)? != 0,
        position: r.get(17)?,
        created_at: r.get(18)?,
    })
}

const NOTE_COLS: &str = "id, kind, fields, tags, source_page, source_block, created_at, updated_at";

fn note_from_row(r: &Row) -> rusqlite::Result<Note> {
    let fields: String = r.get(2)?;
    Ok(Note {
        id: r.get(0)?,
        kind: NoteKind::from_key(&r.get::<_, String>(1)?),
        fields: serde_json::from_str(&fields).unwrap_or_default(),
        tags: r.get(3)?,
        source_page: r.get(4)?,
        source_block: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const REV_COLS: &str = "id, card_id, at, rating, kind, elapsed_days, scheduled_days, duration_ms, state_before, state_after, s_before, s_after, d_before, d_after, retrievability, next_due, algorithm, preset_id";

fn rev_from_row(r: &Row) -> rusqlite::Result<Rev> {
    let f = |i: usize| -> rusqlite::Result<Option<f32>> { Ok(r.get::<_, Option<f64>>(i)?.map(|v| v as f32)) };
    Ok(Rev {
        id: r.get(0)?,
        card_id: r.get(1)?,
        at: r.get(2)?,
        rating: r.get::<_, i64>(3)? as u32,
        kind: ReviewKind::from_i64(r.get(4)?),
        elapsed_days: r.get::<_, f64>(5)? as f32,
        scheduled_days: r.get::<_, f64>(6)? as f32,
        duration_ms: r.get(7)?,
        state_before: State::from_i64(r.get(8)?),
        state_after: State::from_i64(r.get(9)?),
        s_before: f(10)?,
        s_after: f(11)?,
        d_before: f(12)?,
        d_after: f(13)?,
        retrievability: f(14)?,
        next_due: r.get(15)?,
        algorithm: r.get(16)?,
        preset_id: r.get(17)?,
    })
}

fn ids_sql(ids: &[i64]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

impl Db {
    pub fn open(root: &Path) -> Result<Db> {
        std::fs::create_dir_all(root).with_context(|| crate::tr!("error.create_folder", path = root.display()))?;
        // Restauração agendada nas Configurações: troca o banco antes de abri-lo.
        let pending = root.join("restore.pending");
        if let Ok(src) = std::fs::read_to_string(&pending) {
            let src = PathBuf::from(src.trim());
            if src.exists() {
                let db = root.join("discite.db");
                let keep = root.join("backups").join(format!("antes-de-restaurar-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                let _ = std::fs::create_dir_all(root.join("backups"));
                let _ = std::fs::copy(&db, keep);
                for ext in ["-wal", "-shm"] {
                    let _ = std::fs::remove_file(root.join(format!("discite.db{ext}")));
                }
                std::fs::copy(&src, &db).with_context(|| crate::tr!("error.restore"))?;
            }
            let _ = std::fs::remove_file(&pending);
        }
        let conn = Connection::open(root.join("discite.db")).with_context(|| crate::tr!("error.open_db"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch("PRAGMA user_version = 1;")?;
        let db = Db { conn, root: root.to_owned() };
        if db.list_presets()?.is_empty() {
            db.conn.execute(
                "INSERT INTO presets (id, name, config, params, updated_at) VALUES (1, ?3, ?1, '[]', ?2)",
                params![serde_json::to_string(&PresetConfig::default())?, srs::now_ms(), crate::tr!("preset.default_name")],
            )?;
        }
        Ok(db)
    }

    pub fn media_dir(&self) -> PathBuf {
        self.root.join("media")
    }

    // ───────────────────────────── meta ─────────────────────────────

    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    // ─────────────────────────── disciplinas ───────────────────────────

    pub fn list_subjects(&self) -> Result<Vec<Subject>> {
        let mut st = self.conn.prepare("SELECT id, name, color, position FROM subjects ORDER BY position, id")?;
        let v = st
            .query_map([], |r| Ok(Subject { id: r.get(0)?, name: r.get(1)?, color: r.get::<_, i64>(2)? as usize, position: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn add_subject(&self, name: &str, color: usize) -> Result<i64> {
        let id = new_id();
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM subjects", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO subjects (id, name, color, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, name, color as i64, pos, srs::now_ms()],
        )?;
        // Toda disciplina nasce com um baralho raiz de mesmo nome.
        self.add_deck(id, None, name)?;
        Ok(id)
    }

    pub fn update_subject(&self, s: &Subject) -> Result<()> {
        self.conn.execute(
            "UPDATE subjects SET name = ?2, color = ?3, position = ?4, updated_at = ?5 WHERE id = ?1",
            params![s.id, s.name, s.color as i64, s.position, srs::now_ms()],
        )?;
        Ok(())
    }

    pub fn delete_subject(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM cards WHERE deck_id IN (SELECT id FROM decks WHERE subject_id = ?1)", [id])?;
        tx.execute("DELETE FROM notes WHERE id NOT IN (SELECT note_id FROM cards)", [])?;
        tx.execute("DELETE FROM fts_notes WHERE note_id NOT IN (SELECT id FROM notes)", [])?;
        tx.execute("DELETE FROM fts_pages WHERE page_id IN (SELECT id FROM pages WHERE subject_id = ?1)", [id])?;
        tx.execute("DELETE FROM decks WHERE subject_id = ?1", [id])?;
        tx.execute("DELETE FROM pages WHERE subject_id = ?1", [id])?;
        tx.execute("DELETE FROM subjects WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // ───────────────────────────── páginas ─────────────────────────────

    fn page_from_row(r: &Row) -> rusqlite::Result<PageMeta> {
        Ok(PageMeta {
            id: r.get(0)?,
            subject_id: r.get(1)?,
            parent_id: r.get(2)?,
            title: r.get(3)?,
            position: r.get(4)?,
            tags: r.get(5)?,
            collapsed: r.get::<_, i64>(6)? != 0,
            updated_at: r.get(7)?,
        })
    }

    pub fn list_pages(&self, subject: i64) -> Result<Vec<PageMeta>> {
        let mut st = self.conn.prepare(
            "SELECT id, subject_id, parent_id, title, position, tags, collapsed, updated_at FROM pages
             WHERE subject_id = ?1 AND deleted_at IS NULL ORDER BY position, id",
        )?;
        let v = st.query_map([subject], Self::page_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn all_pages(&self) -> Result<Vec<PageMeta>> {
        let mut st = self.conn.prepare(
            "SELECT id, subject_id, parent_id, title, position, tags, collapsed, updated_at FROM pages
             WHERE deleted_at IS NULL ORDER BY subject_id, position, id",
        )?;
        let v = st.query_map([], Self::page_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn page(&self, id: i64) -> Result<Option<PageMeta>> {
        Ok(self
            .conn
            .query_row("SELECT id, subject_id, parent_id, title, position, tags, collapsed, updated_at FROM pages WHERE id = ?1 AND deleted_at IS NULL", [id], Self::page_from_row)
            .optional()?)
    }

    pub fn recent_pages(&self, n: usize) -> Result<Vec<PageMeta>> {
        let mut st = self.conn.prepare(
            "SELECT id, subject_id, parent_id, title, position, tags, collapsed, updated_at FROM pages
             WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let v = st.query_map([n as i64], Self::page_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn add_page(&self, subject: i64, parent: Option<i64>, title: &str) -> Result<i64> {
        let id = new_id();
        let pos: f64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM pages WHERE subject_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
            params![subject, parent],
            |r| r.get(0),
        )?;
        let now = srs::now_ms();
        self.conn.execute(
            "INSERT INTO pages (id, subject_id, parent_id, title, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, subject, parent, title, pos, now],
        )?;
        Ok(id)
    }

    pub fn update_page_meta(&self, p: &PageMeta) -> Result<()> {
        self.conn.execute(
            "UPDATE pages SET title = ?2, parent_id = ?3, position = ?4, tags = ?5, collapsed = ?6, subject_id = ?7 WHERE id = ?1",
            params![p.id, p.title, p.parent_id, p.position, p.tags, p.collapsed as i64, p.subject_id],
        )?;
        Ok(())
    }

    pub fn touch_page(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE pages SET updated_at = ?2 WHERE id = ?1", params![id, srs::now_ms()])?;
        Ok(())
    }

    /// Exclusão lógica da página e das subpáginas.
    pub fn delete_page(&self, id: i64) -> Result<()> {
        let mut ids = vec![id];
        let mut i = 0;
        while i < ids.len() {
            let mut st = self.conn.prepare("SELECT id FROM pages WHERE parent_id = ?1 AND deleted_at IS NULL")?;
            let kids: Vec<i64> = st.query_map([ids[i]], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            ids.extend(kids);
            i += 1;
        }
        let now = srs::now_ms();
        let tx = self.conn.unchecked_transaction()?;
        for p in &ids {
            tx.execute("UPDATE pages SET deleted_at = ?2 WHERE id = ?1", params![p, now])?;
            tx.execute("DELETE FROM fts_pages WHERE page_id = ?1", [p])?;
            tx.execute("DELETE FROM page_links WHERE from_page = ?1 OR to_page = ?1", [p])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_blocks(&self, page: i64) -> Result<Vec<Block>> {
        let mut st = self.conn.prepare("SELECT data FROM blocks WHERE page_id = ?1 ORDER BY position")?;
        let v: Vec<String> = st.query_map([page], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())
    }

    /// Grava a página inteira (blocos, índice de busca e links) numa transação.
    pub fn save_blocks(&self, page: i64, title: &str, blocks: &[Block]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM blocks WHERE page_id = ?1", [page])?;
        tx.execute("DELETE FROM fts_pages WHERE page_id = ?1", [page])?;
        tx.execute("DELETE FROM page_links WHERE from_page = ?1", [page])?;
        tx.execute("INSERT INTO fts_pages (page_id, block_id, text) VALUES (?1, 0, ?2)", params![page, title])?;
        let mut links = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            let plain = b.plain();
            tx.execute(
                "INSERT INTO blocks (id, page_id, position, kind, data, plain) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![b.id, page, i as i64, format!("{:?}", b.kind), serde_json::to_string(b)?, plain],
            )?;
            if !plain.trim().is_empty() {
                tx.execute("INSERT INTO fts_pages (page_id, block_id, text) VALUES (?1, ?2, ?3)", params![page, b.id, plain])?;
            }
            if b.kind.is_text() && b.kind != BlockKind::Code {
                links.extend(crate::rich::links(&b.text));
            }
        }
        for l in links {
            let target: Option<i64> = tx
                .query_row("SELECT id FROM pages WHERE title = ?1 COLLATE NOCASE AND deleted_at IS NULL AND id != ?2 LIMIT 1", params![l, page], |r| r.get(0))
                .optional()?;
            if let Some(t) = target {
                tx.execute("INSERT OR IGNORE INTO page_links (from_page, to_page) VALUES (?1, ?2)", params![page, t])?;
            }
        }
        tx.execute("UPDATE pages SET updated_at = ?2, title = ?3 WHERE id = ?1", params![page, srs::now_ms(), title])?;
        tx.commit()?;
        Ok(())
    }

    pub fn backlinks(&self, page: i64) -> Result<Vec<PageMeta>> {
        let mut st = self.conn.prepare(
            "SELECT p.id, p.subject_id, p.parent_id, p.title, p.position, p.tags, p.collapsed, p.updated_at
             FROM page_links l JOIN pages p ON p.id = l.from_page WHERE l.to_page = ?1 AND p.deleted_at IS NULL ORDER BY p.title",
        )?;
        let v = st.query_map([page], Self::page_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn page_by_title(&self, title: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT id FROM pages WHERE title = ?1 COLLATE NOCASE AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1", [title], |r| r.get(0))
            .optional()?)
    }

    pub fn add_version(&self, page: i64, title: &str, blocks: &[Block]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO page_versions (id, page_id, created_at, title, snapshot) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_id(), page, srs::now_ms(), title, serde_json::to_string(blocks)?],
        )?;
        // Guarda as 50 mais recentes por página.
        self.conn.execute(
            "DELETE FROM page_versions WHERE page_id = ?1 AND id NOT IN (SELECT id FROM page_versions WHERE page_id = ?1 ORDER BY created_at DESC LIMIT 50)",
            [page],
        )?;
        Ok(())
    }

    pub fn versions(&self, page: i64) -> Result<Vec<(i64, i64, String, Vec<Block>)>> {
        let mut st = self.conn.prepare("SELECT id, created_at, title, snapshot FROM page_versions WHERE page_id = ?1 ORDER BY created_at DESC")?;
        let v = st
            .query_map([page], |r| {
                let snap: String = r.get(3)?;
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, serde_json::from_str(&snap).unwrap_or_default()))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// Busca de texto completo nas páginas: (página, bloco, trecho).
    pub fn search_pages(&self, query: &str, limit: usize) -> Result<Vec<(i64, i64, String)>> {
        let q = fts_query(query);
        if q.is_empty() {
            return Ok(vec![]);
        }
        let mut st = self.conn.prepare(
            "SELECT page_id, block_id, snippet(fts_pages, 2, '', '', '…', 12) FROM fts_pages WHERE fts_pages MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let v = st.query_map(params![q, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn add_media(&self, s: &crate::media::Stored) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO media (id, file_name, mime, width, height, size, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![new_id(), s.file_name, s.mime, s.width, s.height, s.size as i64, srs::now_ms()],
        )?;
        Ok(())
    }

    /// Arquivos de `media/` que nenhuma página ou nota usa mais.
    pub fn orphan_media(&self) -> Result<Vec<String>> {
        let mut used = std::collections::HashSet::new();
        let mut st = self.conn.prepare("SELECT data FROM blocks WHERE kind = 'Image'")?;
        for d in st.query_map([], |r| r.get::<_, String>(0))? {
            if let Ok(b) = serde_json::from_str::<Block>(&d?) {
                used.insert(b.media);
            }
        }
        let mut st = self.conn.prepare("SELECT fields FROM notes")?;
        for f in st.query_map([], |r| r.get::<_, String>(0))? {
            let f = f?;
            let mut rest = f.as_str();
            while let Some(i) = rest.find("](") {
                let after = &rest[i + 2..];
                if let Some(j) = after.find(')') {
                    used.insert(after[..j].to_owned());
                }
                rest = &rest[i + 2..];
            }
            for name in serde_json::from_str::<Vec<String>>(&f).unwrap_or_default() {
                used.insert(name);
            }
        }
        let mut st = self.conn.prepare("SELECT data FROM blocks")?;
        let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.media_dir()) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !used.contains(&name) && !all.iter().any(|d| d.contains(&name)) {
                    out.push(name);
                }
            }
        }
        Ok(out)
    }

    // ───────────────────────────── baralhos ─────────────────────────────

    pub fn list_decks(&self) -> Result<Vec<Deck>> {
        let mut st = self.conn.prepare("SELECT id, subject_id, parent_id, preset_id, name, position, collapsed FROM decks ORDER BY position, id")?;
        let v = st
            .query_map([], |r| {
                Ok(Deck {
                    id: r.get(0)?,
                    subject_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    preset_id: r.get(3)?,
                    name: r.get(4)?,
                    position: r.get(5)?,
                    collapsed: r.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn add_deck(&self, subject: i64, parent: Option<i64>, name: &str) -> Result<i64> {
        let id = new_id();
        let preset: i64 = match parent {
            Some(p) => self.conn.query_row("SELECT preset_id FROM decks WHERE id = ?1", [p], |r| r.get(0)).unwrap_or(1),
            None => 1,
        };
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM decks", [], |r| r.get(0))?;
        let now = srs::now_ms();
        self.conn.execute(
            "INSERT INTO decks (id, subject_id, parent_id, preset_id, name, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, subject, parent, preset, name, pos, now],
        )?;
        Ok(id)
    }

    pub fn update_deck(&self, d: &Deck) -> Result<()> {
        self.conn.execute(
            "UPDATE decks SET name = ?2, parent_id = ?3, preset_id = ?4, position = ?5, collapsed = ?6, subject_id = ?7, updated_at = ?8 WHERE id = ?1",
            params![d.id, d.name, d.parent_id, d.preset_id, d.position, d.collapsed as i64, d.subject_id, srs::now_ms()],
        )?;
        Ok(())
    }

    /// Apaga o baralho, os sub-baralhos e os cartões deles.
    pub fn delete_deck(&self, id: i64, all: &[Deck]) -> Result<()> {
        let ids = subtree(all, id);
        let list = ids_sql(&ids);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(&format!("DELETE FROM cards WHERE deck_id IN ({list})"), [])?;
        tx.execute("DELETE FROM notes WHERE id NOT IN (SELECT note_id FROM cards)", [])?;
        tx.execute("DELETE FROM fts_notes WHERE note_id NOT IN (SELECT id FROM notes)", [])?;
        tx.execute(&format!("DELETE FROM decks WHERE id IN ({list})"), [])?;
        tx.commit()?;
        Ok(())
    }

    // ───────────────────────────── presets ─────────────────────────────

    pub fn list_presets(&self) -> Result<Vec<Preset>> {
        let mut st = self.conn.prepare("SELECT id, name, config, params, last_optimized_at FROM presets ORDER BY id")?;
        let v = st
            .query_map([], |r| {
                let cfg: String = r.get(2)?;
                let params: String = r.get(3)?;
                Ok(Preset {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    cfg: serde_json::from_str(&cfg).unwrap_or_default(),
                    params: serde_json::from_str(&params).unwrap_or_default(),
                    last_optimized_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn save_preset(&self, p: &Preset) -> Result<()> {
        self.conn.execute(
            "UPDATE presets SET name = ?2, config = ?3, params = ?4, last_optimized_at = ?5, updated_at = ?6 WHERE id = ?1",
            params![p.id, p.name, serde_json::to_string(&p.cfg)?, serde_json::to_string(&p.params)?, p.last_optimized_at, srs::now_ms()],
        )?;
        Ok(())
    }

    pub fn add_preset(&self, name: &str, cfg: &PresetConfig, params_: &[f32]) -> Result<i64> {
        let id = new_id();
        self.conn.execute(
            "INSERT INTO presets (id, name, config, params, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, serde_json::to_string(cfg)?, serde_json::to_string(params_)?, srs::now_ms()],
        )?;
        Ok(id)
    }

    pub fn delete_preset(&self, id: i64) -> Result<()> {
        if id == 1 {
            return Ok(());
        }
        self.conn.execute("UPDATE decks SET preset_id = 1 WHERE preset_id = ?1", [id])?;
        self.conn.execute("DELETE FROM presets WHERE id = ?1", [id])?;
        Ok(())
    }

    // ───────────────────────────── notas ─────────────────────────────

    pub fn note(&self, id: i64) -> Result<Option<Note>> {
        Ok(self.conn.query_row(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = ?1"), [id], note_from_row).optional()?)
    }

    pub fn notes_by_ids(&self, ids: &[i64]) -> Result<HashMap<i64, Note>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(500) {
            let mut st = self.conn.prepare(&format!("SELECT {NOTE_COLS} FROM notes WHERE id IN ({})", ids_sql(chunk)))?;
            for n in st.query_map([], note_from_row)? {
                let n = n?;
                out.insert(n.id, n);
            }
        }
        Ok(out)
    }

    fn write_note_row(&self, n: &Note) -> Result<()> {
        let sort = cards::sort_field(n.kind, &n.fields);
        let tags = normalize_tags(&n.tags);
        self.conn.execute(
            "INSERT INTO notes (id, kind, fields, sort_field, checksum, tags, source_page, source_block, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, fields = excluded.fields, sort_field = excluded.sort_field,
               checksum = excluded.checksum, tags = excluded.tags, source_page = excluded.source_page,
               source_block = excluded.source_block, updated_at = excluded.updated_at",
            params![
                n.id,
                n.kind.key(),
                serde_json::to_string(&n.fields)?,
                sort,
                cards::checksum(&sort),
                tags,
                n.source_page,
                n.source_block,
                n.created_at,
                n.updated_at
            ],
        )?;
        self.conn.execute("DELETE FROM fts_notes WHERE note_id = ?1", [n.id])?;
        let text: String = n
            .fields
            .iter()
            .enumerate()
            .filter(|(i, _)| !(n.kind == NoteKind::Occlusion && *i < 2))
            .map(|(_, f)| crate::rich::plain(&cards::strip_cloze(f)))
            .collect::<Vec<_>>()
            .join("\n");
        self.conn.execute("INSERT INTO fts_notes (note_id, text) VALUES (?1, ?2)", params![n.id, format!("{text}\n{tags}")])?;
        Ok(())
    }

    /// Cria a nota e os cartões dela. Devolve quantos cartões foram gerados.
    pub fn add_note(&self, n: &mut Note, deck: i64) -> Result<usize> {
        let now = srs::now_ms();
        n.id = new_id();
        n.created_at = now;
        n.updated_at = now;
        let tx = self.conn.unchecked_transaction()?;
        self.write_note_row(n)?;
        let made = self.sync_cards(n, deck)?;
        tx.commit()?;
        Ok(made)
    }

    /// Grava a nota editada: cria cartões para lacunas/máscaras novas e apaga os
    /// que deixaram de existir.
    pub fn update_note(&self, n: &mut Note, deck_for_new: i64) -> Result<usize> {
        n.updated_at = srs::now_ms();
        let tx = self.conn.unchecked_transaction()?;
        self.write_note_row(n)?;
        let made = self.sync_cards(n, deck_for_new)?;
        tx.commit()?;
        Ok(made)
    }

    fn sync_cards(&self, n: &Note, deck: i64) -> Result<usize> {
        let want = cards::card_ords(n.kind, &n.fields);
        let existing: Vec<(i64, u32)> = {
            let mut st = self.conn.prepare("SELECT id, ord FROM cards WHERE note_id = ?1")?;
            st.query_map([n.id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u32)))?.collect::<rusqlite::Result<_>>()?
        };
        for (id, ord) in &existing {
            if !want.contains(ord) {
                self.conn.execute("DELETE FROM cards WHERE id = ?1", [id])?;
            }
        }
        let deck = existing
            .first()
            .and_then(|(id, _)| self.conn.query_row("SELECT deck_id FROM cards WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).ok())
            .unwrap_or(deck);
        let mut made = 0;
        let now = srs::now_ms();
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM cards", [], |r| r.get(0))?;
        for ord in want {
            if existing.iter().any(|(_, o)| *o == ord) {
                continue;
            }
            self.conn.execute(
                "INSERT INTO cards (id, note_id, deck_id, ord, due, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![new_id(), n.id, deck, ord as i64, pos + made as i64, pos + made as i64, now],
            )?;
            made += 1;
        }
        Ok(made)
    }

    pub fn delete_notes(&self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for chunk in ids.chunks(500) {
            let l = ids_sql(chunk);
            tx.execute(&format!("DELETE FROM cards WHERE note_id IN ({l})"), [])?;
            tx.execute(&format!("DELETE FROM notes WHERE id IN ({l})"), [])?;
            tx.execute(&format!("DELETE FROM fts_notes WHERE note_id IN ({l})"), [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Nota parecida (mesmo primeiro campo) para o aviso de duplicata.
    pub fn find_duplicate(&self, kind: NoteKind, fields: &[String], exclude: i64) -> Result<Option<(i64, String, String)>> {
        let sort = cards::sort_field(kind, fields);
        if sort.trim().is_empty() {
            return Ok(None);
        }
        let sum = cards::checksum(&sort);
        Ok(self
            .conn
            .query_row(
                "SELECT id, sort_field, kind FROM notes WHERE checksum = ?1 AND id != ?2 LIMIT 1",
                params![sum, exclude],
                |r| Ok((r.get(0)?, r.get(1)?, NoteKind::from_key(&r.get::<_, String>(2)?).label().to_owned())),
            )
            .optional()?)
    }

    pub fn notes_for_page(&self, page: i64) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!("SELECT {NOTE_COLS} FROM notes WHERE source_page = ?1 ORDER BY created_at"))?;
        let v = st.query_map([page], note_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    // ───────────────────────────── cartões ─────────────────────────────

    pub fn card(&self, id: i64) -> Result<Option<Card>> {
        Ok(self.conn.query_row(&format!("SELECT {CARD_COLS} FROM cards WHERE id = ?1"), [id], card_from_row).optional()?)
    }

    pub fn cards_by_ids(&self, ids: &[i64]) -> Result<Vec<Card>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let mut st = self.conn.prepare(&format!("SELECT {CARD_COLS} FROM cards WHERE id IN ({})", ids_sql(chunk)))?;
            out.extend(st.query_map([], card_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(out)
    }

    pub fn note_cards(&self, note: i64) -> Result<Vec<Card>> {
        let mut st = self.conn.prepare(&format!("SELECT {CARD_COLS} FROM cards WHERE note_id = ?1 ORDER BY ord"))?;
        let v = st.query_map([note], card_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn cards_where(&self, filter: &str) -> Result<Vec<Card>> {
        let mut st = self.conn.prepare(&format!("SELECT {CARD_COLS} FROM cards c WHERE {filter}"))?;
        let v = st.query_map([], card_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn save_card(&self, c: &Card) -> Result<()> {
        let s = &c.s;
        self.conn.execute(
            "UPDATE cards SET deck_id = ?2, state = ?3, queue = ?4, due = ?5, step = ?6, stability = ?7, difficulty = ?8, ease = ?9,
               interval = ?10, reps = ?11, lapses = ?12, last_review = ?13, flagged = ?14, leech = ?15, updated_at = ?16 WHERE id = ?1",
            params![
                c.id,
                c.deck_id,
                s.state as i64,
                c.queue as i64,
                s.due,
                s.step as i64,
                s.stability.map(|v| v as f64),
                s.difficulty.map(|v| v as f64),
                s.ease as f64,
                s.interval as f64,
                s.reps as i64,
                s.lapses as i64,
                s.last_review,
                c.flagged as i64,
                c.leech as i64,
                srs::now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn set_queue(&self, ids: &[i64], q: Queue) -> Result<()> {
        for chunk in ids.chunks(500) {
            self.conn.execute(&format!("UPDATE cards SET queue = ?1, updated_at = ?2 WHERE id IN ({})", ids_sql(chunk)), params![q as i64, srs::now_ms()])?;
        }
        Ok(())
    }

    pub fn set_flag(&self, ids: &[i64], on: bool) -> Result<()> {
        for chunk in ids.chunks(500) {
            self.conn.execute(&format!("UPDATE cards SET flagged = ?1 WHERE id IN ({})", ids_sql(chunk)), [on as i64])?;
        }
        Ok(())
    }

    pub fn move_cards(&self, ids: &[i64], deck: i64) -> Result<()> {
        for chunk in ids.chunks(500) {
            self.conn.execute(&format!("UPDATE cards SET deck_id = ?1 WHERE id IN ({})", ids_sql(chunk)), [deck])?;
        }
        Ok(())
    }

    /// Volta os cartões para "novo" (o histórico continua no revlog).
    pub fn reset_cards(&self, ids: &[i64]) -> Result<()> {
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM cards", [], |r| r.get(0))?;
        for chunk in ids.chunks(500) {
            self.conn.execute(
                &format!(
                    "UPDATE cards SET state = 0, queue = 0, due = ?1 + id % 1000, step = 0, stability = NULL, difficulty = NULL, ease = 0,
                     interval = 0, reps = 0, lapses = 0, last_review = NULL, leech = 0 WHERE id IN ({})",
                    ids_sql(chunk)
                ),
                [pos],
            )?;
        }
        Ok(())
    }

    /// Desenterra o que foi enterrado em dias anteriores.
    pub fn unbury(&self) -> Result<()> {
        self.conn.execute("UPDATE cards SET queue = 0 WHERE queue IN (-2, -3)", [])?;
        Ok(())
    }

    pub fn bury_siblings(&self, card: &Card) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT id FROM cards WHERE note_id = ?1 AND id != ?2 AND queue = 0 AND state IN (0, 2)")?;
        let ids: Vec<i64> = st.query_map(params![card.note_id, card.id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        self.set_queue(&ids, Queue::BuriedSibling)?;
        Ok(ids)
    }

    /// Contagens cruas por baralho: novos, em aprendizado vencidos até `learn_until`
    /// e revisões que vencem antes de `day_end`.
    pub fn deck_counts(&self, learn_until: i64, day_end: i64) -> Result<HashMap<i64, Counts>> {
        let mut st = self.conn.prepare(
            "SELECT deck_id,
               SUM(CASE WHEN state = 0 THEN 1 ELSE 0 END),
               SUM(CASE WHEN state IN (1, 3) AND due <= ?1 THEN 1 ELSE 0 END),
               SUM(CASE WHEN state = 2 AND due < ?2 THEN 1 ELSE 0 END)
             FROM cards WHERE queue = 0 GROUP BY deck_id",
        )?;
        let mut out = HashMap::new();
        for r in st.query_map(params![learn_until, day_end], |r| {
            Ok((r.get::<_, i64>(0)?, Counts { new: r.get::<_, i64>(1)? as u32, learn: r.get::<_, i64>(2)? as u32, review: r.get::<_, i64>(3)? as u32 }))
        })? {
            let (d, c) = r?;
            out.insert(d, c);
        }
        Ok(out)
    }

    /// Novos introduzidos e revisões feitas hoje, por baralho.
    pub fn done_today(&self, day_start: i64) -> Result<HashMap<i64, (u32, u32)>> {
        let mut st = self.conn.prepare(
            "SELECT c.deck_id,
               SUM(CASE WHEN r.state_before = 0 THEN 1 ELSE 0 END),
               SUM(CASE WHEN r.state_before = 2 AND r.kind != 3 THEN 1 ELSE 0 END)
             FROM revlog r JOIN cards c ON c.id = r.card_id
             WHERE r.at >= ?1 AND r.undone = 0 GROUP BY c.deck_id",
        )?;
        let mut out = HashMap::new();
        for r in st.query_map([day_start], |r| Ok((r.get::<_, i64>(0)?, (r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32))))? {
            let (d, c) = r?;
            out.insert(d, c);
        }
        Ok(out)
    }

    /// Revisões que vencem em cada dia (para o balanceamento de carga).
    pub fn due_per_day(&self, clock: &Clock) -> Result<HashMap<i64, usize>> {
        let mut st = self.conn.prepare("SELECT due FROM cards WHERE state = 2 AND queue = 0")?;
        let mut out = HashMap::new();
        for d in st.query_map([], |r| r.get::<_, i64>(0))? {
            *out.entry(clock.day_of(d?)).or_insert(0) += 1;
        }
        Ok(out)
    }

    // ───────────────────────────── revlog ─────────────────────────────

    pub fn add_rev(&self, r: &Rev) -> Result<i64> {
        let id = new_id();
        self.conn.execute(
            &format!("INSERT INTO revlog ({REV_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"),
            params![
                id,
                r.card_id,
                r.at,
                r.rating as i64,
                r.kind as i64,
                r.elapsed_days as f64,
                r.scheduled_days as f64,
                r.duration_ms,
                r.state_before as i64,
                r.state_after as i64,
                r.s_before.map(|v| v as f64),
                r.s_after.map(|v| v as f64),
                r.d_before.map(|v| v as f64),
                r.d_after.map(|v| v as f64),
                r.retrievability.map(|v| v as f64),
                r.next_due,
                r.algorithm,
                r.preset_id
            ],
        )?;
        Ok(id)
    }

    pub fn undo_rev(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE revlog SET undone = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn card_history(&self, card: i64) -> Result<Vec<Rev>> {
        let mut st = self.conn.prepare(&format!("SELECT {REV_COLS} FROM revlog WHERE card_id = ?1 AND undone = 0 ORDER BY at"))?;
        let v = st.query_map([card], rev_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// Histórico de todos os cartões dos baralhos dados, agrupado por cartão.
    pub fn histories(&self, deck_ids: &[i64]) -> Result<HashMap<i64, Vec<Rev>>> {
        let mut out: HashMap<i64, Vec<Rev>> = HashMap::new();
        if deck_ids.is_empty() {
            return Ok(out);
        }
        let mut st = self.conn.prepare(&format!(
            "SELECT {} FROM revlog r WHERE r.undone = 0 AND r.kind != 4 AND r.card_id IN (SELECT id FROM cards WHERE deck_id IN ({})) ORDER BY r.card_id, r.at",
            REV_COLS.split(", ").map(|c| format!("r.{c}")).collect::<Vec<_>>().join(", "),
            ids_sql(deck_ids)
        ))?;
        for r in st.query_map([], rev_from_row)? {
            let r = r?;
            out.entry(r.card_id).or_default().push(r);
        }
        Ok(out)
    }

    pub fn revlog_count(&self, deck_ids: &[i64]) -> Result<i64> {
        if deck_ids.is_empty() {
            return Ok(0);
        }
        Ok(self.conn.query_row(
            &format!("SELECT COUNT(*) FROM revlog WHERE undone = 0 AND card_id IN (SELECT id FROM cards WHERE deck_id IN ({}))", ids_sql(deck_ids)),
            [],
            |r| r.get(0),
        )?)
    }

    /// Cópia consistente do banco (`VACUUM INTO`).
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            std::fs::remove_file(dest)?;
        }
        self.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
        Ok(())
    }
}

pub fn normalize_tags(s: &str) -> String {
    let mut v: Vec<String> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim().trim_start_matches('#').to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v.join(" ")
}

/// Termos do usuário viram prefixos FTS5 entre aspas (sem sintaxe solta).
pub fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// O baralho e todos os descendentes.
pub fn subtree(all: &[Deck], root: i64) -> Vec<i64> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        out.extend(all.iter().filter(|d| d.parent_id == Some(cur)).map(|d| d.id));
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> Db {
        let dir = std::env::temp_dir().join(format!("discite-test-{}", new_id()));
        Db::open(&dir).unwrap()
    }

    #[test]
    fn notes_generate_and_sync_cards() {
        let db = temp_db();
        let s = db.add_subject("Biologia", 2).unwrap();
        let deck = db.list_decks().unwrap().into_iter().find(|d| d.subject_id == s).unwrap().id;
        let mut n = Note::new(NoteKind::Cloze);
        n.fields[0] = "A {{c1::mitose}} gera {{c2::duas}} células".into();
        assert_eq!(db.add_note(&mut n, deck).unwrap(), 2);
        n.fields[0] = "A {{c1::mitose}} gera duas células {{c3::diploides}}".into();
        db.update_note(&mut n, deck).unwrap();
        let ords: Vec<u32> = db.note_cards(n.id).unwrap().iter().map(|c| c.ord).collect();
        assert_eq!(ords, vec![0, 2]);
        assert!(db.find_duplicate(NoteKind::Cloze, &n.fields, 0).unwrap().is_some());
    }

    #[test]
    fn pages_blocks_links_and_search() {
        let db = temp_db();
        let s = db.add_subject("Bio", 0).unwrap();
        let a = db.add_page(s, None, "Mitose").unwrap();
        let b = db.add_page(s, None, "Meiose").unwrap();
        let blocks = vec![Block::new(BlockKind::Paragraph, "Compare com [[Meiose]] e a **metáfase**")];
        db.save_blocks(a, "Mitose", &blocks).unwrap();
        assert_eq!(db.load_blocks(a).unwrap(), blocks);
        assert_eq!(db.backlinks(b).unwrap()[0].id, a);
        let hits = db.search_pages("metafase", 10).unwrap();
        assert_eq!(hits[0].0, a);
    }
}
