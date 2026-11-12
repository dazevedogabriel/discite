//! Questões objetivas: bancos de questões por disciplina; cada questão tem
//! enunciado (texto rico), imagem opcional, quatro alternativas, a certa e uma
//! explicação.
//!
//! O agendamento é simples, porque a resposta é só certa ou errada: acertou,
//! a questão volta em 3 dias, depois o intervalo cresce 2,5× (até um ano);
//! errou, volta no dia seguinte e o intervalo recomeça.

use crate::db::{Db, new_id};
use crate::srs::{self, Clock};
use anyhow::Result;
use rusqlite::{OptionalExtension, Row, params};

pub const OPTIONS: usize = 4;
const FIRST_INTERVAL: f32 = 3.0;
const GROWTH: f32 = 2.5;
const MAX_INTERVAL: f32 = 365.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Bank {
    pub id: i64,
    pub subject_id: i64,
    pub name: String,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub id: i64,
    pub bank_id: i64,
    pub stem: String,
    /// Arquivo na pasta `media` (vazio = sem imagem).
    pub image: String,
    pub options: [String; OPTIONS],
    /// Índice da alternativa certa.
    pub answer: usize,
    pub explanation: String,
    pub source_page: Option<i64>,
    pub source_block: Option<i64>,
    pub due: i64,
    /// Dias até a próxima vez (0 = errada ou nunca acertada).
    pub interval: f32,
    pub attempts: u32,
    pub correct: u32,
    pub last_ok: Option<bool>,
}

impl Question {
    pub fn new(bank_id: i64) -> Question {
        Question {
            id: 0,
            bank_id,
            stem: String::new(),
            image: String::new(),
            options: Default::default(),
            answer: 0,
            explanation: String::new(),
            source_page: None,
            source_block: None,
            due: 0,
            interval: 0.0,
            attempts: 0,
            correct: 0,
            last_ok: None,
        }
    }

    pub fn is_new(&self) -> bool {
        self.attempts == 0
    }

    /// Enunciado e as quatro alternativas preenchidos.
    pub fn complete(&self) -> bool {
        (!self.stem.trim().is_empty() || !self.image.is_empty()) && self.options.iter().all(|o| !o.trim().is_empty())
    }
}

/// Próxima data e intervalo depois de responder.
pub fn schedule(q: &Question, ok: bool, clock: &Clock, now: i64) -> (i64, f32) {
    let interval = if !ok {
        0.0
    } else if q.interval < 1.0 {
        FIRST_INTERVAL
    } else {
        (q.interval * GROWTH).min(MAX_INTERVAL)
    };
    let days = (interval.round() as i64).max(1);
    (clock.day_start(clock.day_of(now) + days), interval)
}

/// Contagens de um banco (ou de vários somados).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub total: u32,
    pub new: u32,
    /// Já respondidas e vencidas até o fim do dia de estudo.
    pub due: u32,
    /// Respondidas pelo menos uma vez, com a última resposta errada.
    pub wrong: u32,
    pub attempts: u32,
    pub correct: u32,
}

impl Counts {
    pub fn add(&mut self, o: Counts) {
        self.total += o.total;
        self.new += o.new;
        self.due += o.due;
        self.wrong += o.wrong;
        self.attempts += o.attempts;
        self.correct += o.correct;
    }

    pub fn accuracy(&self) -> Option<f32> {
        (self.attempts > 0).then(|| self.correct as f32 / self.attempts as f32)
    }
}

/// Que questões entram num simulado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    All,
    /// Vencidas e novas: a revisão do dia.
    Due,
    /// A última resposta foi errada.
    Wrong,
    New,
}

const COLS: &str = "id, bank_id, stem, image, options, answer, explanation, source_page, source_block, due, interval, attempts, correct, last_ok";

fn from_row(r: &Row) -> rusqlite::Result<Question> {
    let opts: Vec<String> = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default();
    let mut options: [String; OPTIONS] = Default::default();
    for (slot, o) in options.iter_mut().zip(opts) {
        *slot = o;
    }
    Ok(Question {
        id: r.get(0)?,
        bank_id: r.get(1)?,
        stem: r.get(2)?,
        image: r.get(3)?,
        options,
        answer: (r.get::<_, i64>(5)?.max(0) as usize).min(OPTIONS - 1),
        explanation: r.get(6)?,
        source_page: r.get(7)?,
        source_block: r.get(8)?,
        due: r.get(9)?,
        interval: r.get::<_, f64>(10)? as f32,
        attempts: r.get::<_, i64>(11)? as u32,
        correct: r.get::<_, i64>(12)? as u32,
        last_ok: r.get::<_, Option<i64>>(13)?.map(|v| v != 0),
    })
}

fn ids_sql(ids: &[i64]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

impl Db {
    // ───────────────────────────── bancos ─────────────────────────────

    pub fn list_banks(&self, subject: i64) -> Result<Vec<Bank>> {
        let mut st = self.conn.prepare("SELECT id, subject_id, name, position FROM qbanks WHERE subject_id = ?1 ORDER BY position, id")?;
        let v = st.query_map([subject], |r| Ok(Bank { id: r.get(0)?, subject_id: r.get(1)?, name: r.get(2)?, position: r.get(3)? }))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn bank(&self, id: i64) -> Result<Option<Bank>> {
        Ok(self
            .conn
            .query_row("SELECT id, subject_id, name, position FROM qbanks WHERE id = ?1", [id], |r| Ok(Bank { id: r.get(0)?, subject_id: r.get(1)?, name: r.get(2)?, position: r.get(3)? }))
            .optional()?)
    }

    pub fn add_bank(&self, subject: i64, name: &str) -> Result<i64> {
        let id = new_id();
        let pos: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM qbanks WHERE subject_id = ?1", [subject], |r| r.get(0))?;
        self.conn.execute("INSERT INTO qbanks (id, subject_id, name, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)", params![id, subject, name, pos, srs::now_ms()])?;
        Ok(id)
    }

    pub fn rename_bank(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE qbanks SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name, srs::now_ms()])?;
        Ok(())
    }

    /// Apaga o banco com as questões e o histórico delas.
    pub fn delete_bank(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM qlog WHERE question_id IN (SELECT id FROM questions WHERE bank_id = ?1)", [id])?;
        tx.execute("DELETE FROM questions WHERE bank_id = ?1", [id])?;
        tx.execute("DELETE FROM qbanks WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// O primeiro banco da disciplina; cria um com o nome dela se não houver.
    pub fn first_bank(&self, subject: i64, name: &str) -> Result<i64> {
        match self.list_banks(subject)?.first() {
            Some(b) => Ok(b.id),
            None => self.add_bank(subject, name),
        }
    }

    /// Contagens por banco da disciplina.
    pub fn bank_counts(&self, subject: i64, day_end: i64) -> Result<std::collections::HashMap<i64, Counts>> {
        let mut st = self.conn.prepare(
            "SELECT q.bank_id, COUNT(*),
                    SUM(q.attempts = 0),
                    SUM(q.attempts > 0 AND q.due < ?2),
                    SUM(q.last_ok = 0),
                    SUM(q.attempts), SUM(q.correct)
             FROM questions q JOIN qbanks b ON b.id = q.bank_id
             WHERE b.subject_id = ?1 GROUP BY q.bank_id",
        )?;
        let mut out = std::collections::HashMap::new();
        let rows = st.query_map(params![subject, day_end], |r| {
            let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) as u32);
            Ok((r.get::<_, i64>(0)?, Counts { total: n(1)?, new: n(2)?, due: n(3)?, wrong: n(4)?, attempts: n(5)?, correct: n(6)? }))
        })?;
        for row in rows {
            let (id, c) = row?;
            out.insert(id, c);
        }
        Ok(out)
    }

    /// Acertos e respostas desde `since` (ms) nos bancos da disciplina.
    pub fn quiz_accuracy_since(&self, subject: i64, since: i64) -> Result<(u32, u32)> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(l.ok), 0) FROM qlog l
             JOIN questions q ON q.id = l.question_id JOIN qbanks b ON b.id = q.bank_id
             WHERE b.subject_id = ?1 AND l.at >= ?2",
            params![subject, since],
            |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32)),
        )?)
    }

    // ───────────────────────────── questões ─────────────────────────────

    pub fn list_questions(&self, bank: i64) -> Result<Vec<Question>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM questions WHERE bank_id = ?1 ORDER BY id"))?;
        let v = st.query_map([bank], from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn question(&self, id: i64) -> Result<Option<Question>> {
        Ok(self.conn.query_row(&format!("SELECT {COLS} FROM questions WHERE id = ?1"), [id], from_row).optional()?)
    }

    /// Questões dos bancos dados, filtradas por `pick`.
    pub fn pick_questions(&self, banks: &[i64], pick: Pick, day_end: i64) -> Result<Vec<Question>> {
        if banks.is_empty() {
            return Ok(vec![]);
        }
        let filter = match pick {
            Pick::All => String::new(),
            Pick::Due => format!(" AND (attempts = 0 OR due < {day_end})"),
            Pick::Wrong => " AND last_ok = 0".into(),
            Pick::New => " AND attempts = 0".into(),
        };
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM questions WHERE bank_id IN ({}){filter} ORDER BY due, id", ids_sql(banks)))?;
        let v = st.query_map([], from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// Grava a questão (cria se `id` for 0) e devolve o id.
    pub fn save_question(&self, q: &mut Question) -> Result<i64> {
        let opts = serde_json::to_string(&q.options)?;
        let now = srs::now_ms();
        if q.id == 0 {
            q.id = new_id();
            self.conn.execute(
                "INSERT INTO questions (id, bank_id, stem, image, options, answer, explanation, source_page, source_block, due, interval, attempts, correct, last_ok, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0, 0, NULL, ?11, ?11)",
                params![q.id, q.bank_id, q.stem, q.image, opts, q.answer as i64, q.explanation, q.source_page, q.source_block, q.due, now],
            )?;
        } else {
            self.conn.execute(
                "UPDATE questions SET bank_id = ?2, stem = ?3, image = ?4, options = ?5, answer = ?6, explanation = ?7, updated_at = ?8 WHERE id = ?1",
                params![q.id, q.bank_id, q.stem, q.image, opts, q.answer as i64, q.explanation, now],
            )?;
        }
        Ok(q.id)
    }

    /// Questões que têm todas as palavras no enunciado, nas alternativas ou na explicação.
    pub fn search_questions(&self, query: &str, limit: usize) -> Result<Vec<Question>> {
        let words: Vec<String> = query.split_whitespace().map(|w| format!("%{}%", w.replace(['%', '_'], ""))).collect();
        let cond = if words.is_empty() {
            "1".to_owned()
        } else {
            (1..=words.len()).map(|i| format!("(stem || ' ' || options || ' ' || explanation) LIKE ?{i}")).collect::<Vec<_>>().join(" AND ")
        };
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM questions WHERE {cond} ORDER BY id DESC LIMIT {limit}"))?;
        let v = st.query_map(rusqlite::params_from_iter(words.iter()), from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// Desfaz uma exclusão (o histórico de respostas não volta; os totais, sim).
    pub fn restore_question(&self, q: &Question) -> Result<()> {
        let now = srs::now_ms();
        self.conn.execute(
            "INSERT OR IGNORE INTO questions (id, bank_id, stem, image, options, answer, explanation, source_page, source_block, due, interval, attempts, correct, last_ok, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            params![q.id, q.bank_id, q.stem, q.image, serde_json::to_string(&q.options)?, q.answer as i64, q.explanation, q.source_page, q.source_block, q.due, q.interval as f64, q.attempts as i64, q.correct as i64, q.last_ok, now],
        )?;
        Ok(())
    }

    pub fn delete_questions(&self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(&format!("DELETE FROM qlog WHERE question_id IN ({})", ids_sql(ids)), [])?;
        tx.execute(&format!("DELETE FROM questions WHERE id IN ({})", ids_sql(ids)), [])?;
        tx.commit()?;
        Ok(())
    }

    /// Registra uma resposta e reagenda a questão.
    pub fn answer_question(&self, q: &Question, chosen: usize, duration_ms: i64, clock: &Clock) -> Result<Question> {
        let ok = chosen == q.answer;
        let now = srs::now_ms();
        let (due, interval) = schedule(q, ok, clock, now);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT INTO qlog (id, question_id, at, chosen, ok, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![new_id(), q.id, now, chosen as i64, ok, duration_ms])?;
        tx.execute(
            "UPDATE questions SET due = ?2, interval = ?3, attempts = attempts + 1, correct = correct + ?4, last_ok = ?4 WHERE id = ?1",
            params![q.id, due, interval as f64, ok as i64],
        )?;
        tx.commit()?;
        let mut out = q.clone();
        out.due = due;
        out.interval = interval;
        out.attempts += 1;
        out.correct += ok as u32;
        out.last_ok = Some(ok);
        Ok(out)
    }

    /// Enunciados das questões que já saíram de uma página (para a IA não repetir).
    pub fn question_stems_for_page(&self, page: i64) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT stem FROM questions WHERE source_page = ?1")?;
        let v = st.query_map([page], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v.iter().map(|s| crate::rich::plain(s)).collect())
    }

    /// Textos das questões, para achar as imagens que ainda estão em uso.
    pub fn question_media(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT image || ' ' || stem || ' ' || options || ' ' || explanation FROM questions")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn subject_question_count(&self, subject: i64) -> Result<u32> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM questions q JOIN qbanks b ON b.id = q.bank_id WHERE b.subject_id = ?1", [subject], |r| r.get::<_, i64>(0))? as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("discite-quiz-{}", new_id()));
        (Db::open(&dir).unwrap(), dir)
    }

    #[test]
    fn interval_grows_on_right_and_resets_on_wrong() {
        let clock = Clock { rollover_hour: 4 };
        let now = srs::now_ms();
        let mut q = Question::new(1);
        let (d1, i1) = schedule(&q, true, &clock, now);
        assert_eq!(i1, FIRST_INTERVAL);
        assert_eq!(clock.day_of(d1), clock.day_of(now) + 3);
        q.interval = i1;
        let (_, i2) = schedule(&q, true, &clock, now);
        assert_eq!(i2, 7.5);
        let (d3, i3) = schedule(&q, false, &clock, now);
        assert_eq!(i3, 0.0);
        assert_eq!(clock.day_of(d3), clock.day_of(now) + 1);
    }

    #[test]
    fn save_answer_count_and_pick() {
        let (db, dir) = db();
        let sid = db.add_subject("Oftalmo", 0).unwrap();
        let bank = db.first_bank(sid, "Oftalmo").unwrap();
        assert_eq!(db.first_bank(sid, "x").unwrap(), bank);
        let mut q = Question::new(bank);
        q.stem = "Onde fica a **fóvea**?".into();
        q.options = ["Mácula".into(), "Íris".into(), "Córnea".into(), "Cristalino".into()];
        db.save_question(&mut q).unwrap();
        let mut q2 = q.clone();
        q2.id = 0;
        q2.stem = "Outra".into();
        db.save_question(&mut q2).unwrap();
        let clock = Clock { rollover_hour: 4 };
        let day_end = clock.today_end();
        let c = db.bank_counts(sid, day_end).unwrap()[&bank];
        assert_eq!((c.total, c.new, c.due), (2, 2, 0));
        let q = db.answer_question(&q, 2, 5000, &clock).unwrap();
        assert_eq!(q.last_ok, Some(false));
        let c = db.bank_counts(sid, day_end).unwrap()[&bank];
        assert_eq!((c.new, c.wrong, c.attempts, c.correct), (1, 1, 1, 0));
        // Errada volta amanhã: não está vencida hoje, mas entra em "erradas".
        assert_eq!(db.pick_questions(&[bank], Pick::Due, day_end).unwrap().len(), 1);
        assert_eq!(db.pick_questions(&[bank], Pick::Wrong, day_end).unwrap()[0].id, q.id);
        assert_eq!(db.question(q.id).unwrap().unwrap().options[0], "Mácula");
        assert_eq!(db.quiz_accuracy_since(sid, 0).unwrap(), (1, 0));
        db.delete_bank(bank).unwrap();
        assert_eq!(db.subject_question_count(sid).unwrap(), 0);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
