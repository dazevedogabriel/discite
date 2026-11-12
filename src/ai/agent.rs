//! Assistente que responde a partir das notas. É o laço de agente mais simples:
//! manda a conversa e as ferramentas para a IA online, executa as ferramentas
//! que ela pedir, devolve o resultado e repete até ela responder sem pedir nada.
//!
//! As ferramentas de leitura usam uma conexão só de leitura. As de escrita só
//! propõem uma `Action`: a interface pede permissão, grava e devolve o
//! resultado. Cartões e questões novos precisam citar um trecho da página de
//! onde saíram, conferido como na geração por IA (`ground.rs`).

use super::actions::{Action, NewCard, NewQuestion};
use super::ground::Index;
use super::online::{self, Api};
use crate::cards;
use crate::db::Db;
use crate::srs::Clock;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// Voltas no laço antes de desistir (cada volta é um pedido à API).
const MAX_STEPS: usize = 12;
/// Teto do texto de uma página entregue ao modelo.
const MAX_PAGE_CHARS: usize = 12_000;
/// Lado maior das imagens mandadas ao modelo.
const MAX_IMAGE_SIDE: u32 = 1600;

// Curto de propósito: as instruções e as ferramentas vão em todo pedido à API.
const SYSTEM: &str = "Study assistant in Discite. Use ONLY the user's notes (and attached images), never outside knowledge; \
if the notes lack something, say so. Search, then read the relevant pages; cite page titles like [Mitochondria]. \
New cards and questions must come from a page you read, each with source_page_id and a quote copied exactly from it \
(or from_image for an attached image). Only the cards/questions tools save anything; never write cards in the reply. \
Cloze syntax: {{c1::hidden}}. Changes wait for the user's approval; if denied, \
don't retry, ask what to change. Delete only when explicitly asked. Never show ids or {{c1::}} markup to the user. \
Always reply in the language of the user's message, concisely; Markdown allowed, no tables.";

/// O que a interface mostra enquanto o assistente trabalha.
#[derive(Debug)]
pub enum Event {
    /// Uma ferramenta de leitura rodou: nome, argumento legível e quantos itens voltaram.
    Tool { name: String, arg: String, count: usize },
    /// Uma mudança proposta: a interface decide (pergunta, aplica ou recusa) e responde.
    Ask { action: Action, reply: Sender<Reply> },
    /// Tokens gastos neste pedido à API.
    Usage(u64),
    /// Resposta final, com as mensagens novas para guardar na conversa.
    Answer { text: String, messages: Vec<Value> },
    Failed(String),
}

/// A decisão sobre uma `Ask`, com o texto que volta ao modelo.
#[derive(Debug)]
pub enum Reply {
    Done(String),
    Refused(String),
}

pub struct Run {
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A página aberta no caderno, quando a pergunta vem do painel lateral.
pub struct Context {
    pub page_id: i64,
    pub title: String,
}

pub struct Request {
    pub api: Api,
    pub root: PathBuf,
    /// Mensagens anteriores da conversa (sem a de sistema).
    pub history: Vec<Value>,
    /// Conteúdo da nova mensagem do usuário (texto, ou partes com imagens `media:arquivo`).
    pub user: Value,
    pub context: Option<Context>,
    pub clock: Clock,
}

/// Começa uma resposta numa thread própria.
pub fn ask(req: Request, repaint: impl Fn() + Send + 'static) -> Run {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        let send = |e: Event| {
            let _ = tx.send(e);
            repaint();
        };
        let result = open_read_only(&req.root).and_then(|db| run(&req, &db, &c, &send));
        match result {
            Ok(_) => {}
            Err(_) if c.load(Ordering::Relaxed) => {}
            Err(e) => {
                crate::diag::error(format!("assistant: {e:#}"));
                send(Event::Failed(format!("{e:#}")));
            }
        }
    });
    Run { events: rx, cancel }
}

fn open_read_only(root: &Path) -> Result<Db> {
    let conn = rusqlite::Connection::open_with_flags(root.join("discite.db"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(Db { conn, root: root.to_owned() })
}

fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}

/// Cinco ferramentas: cada uma custa tokens em todo pedido, então as de
/// cartões e questões juntam buscar/criar/editar/apagar num campo `op`/`kind`.
fn tools() -> Value {
    let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
    let (int, txt, flag) = (json!({"type": "integer"}), json!({"type": "string"}), json!({"type": "boolean"}));
    let ints = json!({"type": "array", "items": {"type": "integer"}});
    let op = json!({"enum": ["create", "edit", "delete"]});
    let card = obj(json!({"type": {"enum": ["basic", "cloze"]}, "front": txt, "back": txt, "source_page_id": int, "quote": txt, "from_image": flag}), &["type", "front"]);
    let question = obj(json!({"stem": txt, "options": {"type": "array", "items": txt}, "answer": int, "explanation": txt, "source_page_id": int, "quote": txt, "from_image": flag}), &["stem", "options", "answer"]);
    json!([
        tool("search_notes", "Search notes. 1-3 keywords, all must match; retry with synonyms or the other language.", obj(json!({"query": txt}), &["query"])),
        tool("read_page", "Full text of a page.", obj(json!({"page_id": int}), &["page_id"])),
        tool("find", "decks: list decks and question banks. cards: query with words, \"phrase\", deck:X, tag:X, is:due|new|review|leech, lapses>N, added:N, -x. questions: words. Retry in the other language.",
            obj(json!({"kind": {"enum": ["decks", "cards", "questions"]}, "query": txt}), &["kind"])),
        tool("cards", "Propose changes. create: deck_id+cards. edit: note_id+front/back (omitted stay). delete: note_ids.",
            obj(json!({"op": op, "deck_id": int, "cards": {"type": "array", "items": card}, "note_id": int, "front": txt, "back": txt, "note_ids": ints}), &["op"])),
        tool("questions", "Propose changes. create: bank_id+questions (4 options, answer 0-3). edit: question_id+fields (omitted stay). delete: question_ids.",
            obj(json!({"op": op, "bank_id": int, "questions": {"type": "array", "items": question}, "question_id": int, "stem": txt, "options": {"type": "array", "items": txt}, "answer": int, "explanation": txt, "question_ids": ints}), &["op"])),
    ])
}

/// O nome interno de cada operação (o mesmo que a interface mostra).
fn internal_name(name: &str, args: &Value) -> String {
    let field = |k: &str| args[k].as_str().unwrap_or_default().to_owned();
    match (name, field("kind").as_str(), field("op").as_str()) {
        ("find", "decks", _) => "list_decks_and_banks",
        ("find", "questions", _) => "search_questions",
        ("find", _, _) => "search_cards",
        ("cards", _, "create") => "create_cards",
        ("cards", _, "edit") => "edit_card",
        ("cards", _, "delete") => "delete_cards",
        ("questions", _, "create") => "create_questions",
        ("questions", _, "edit") => "edit_question",
        ("questions", _, "delete") => "delete_questions",
        _ => name,
    }
    .to_owned()
}

/// Para as próximas perguntas, os resultados longos de ferramentas viram um
/// começo: o modelo pode chamar a ferramenta de novo se precisar.
fn trim_tool_results(messages: &mut [Value]) {
    for m in messages.iter_mut().filter(|m| m["role"] == "tool") {
        let Some(c) = m["content"].as_str() else { continue };
        if c.len() > 400 {
            let cut = (0..=300).rev().find(|&i| c.is_char_boundary(i)).unwrap_or(0);
            m["content"] = json!(format!("{}… (trimmed; call the tool again if needed)", &c[..cut]));
        }
    }
}

/// Número que alguns modelos mandam como texto.
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_owned()
}

fn page_text(db: &Db, id: i64) -> Option<(String, super::prompt::Source)> {
    let meta = db.page(id).ok().flatten()?;
    let blocks = db.load_blocks(id).ok()?;
    Some((meta.title.clone(), super::prompt::source(&meta.title, &blocks)))
}

/// Alternativa sem a letra que alguns modelos põem na frente ("B) texto").
fn option(v: &Value) -> String {
    let s = str_of(v);
    let mut c = s.chars();
    match (c.next(), c.next(), c.next()) {
        (Some('A'..='D' | 'a'..='d'), Some(')' | '.'), Some(' ')) => s[3..].trim().to_owned(),
        _ => s,
    }
}

/// O que uma ferramenta devolve: só texto (leitura) ou uma mudança a aprovar.
enum Outcome {
    Read { text: String, arg: String, count: usize },
    Propose { action: Action, notes: String },
    Error(String),
}

fn deck_name(db: &Db, id: i64) -> Option<String> {
    let decks = db.list_decks().ok()?;
    decks.iter().any(|d| d.id == id).then(|| crate::search::deck_path(&decks, id))
}

/// Confere o item com a página citada. Devolve (página, bloco) ou o motivo da recusa.
fn ground(db: &Db, args: &Value, answer: &str, pages: &mut HashMap<i64, Option<(Index, Vec<i64>)>>) -> Result<(i64, Option<i64>), String> {
    let Some(page) = int(&args["source_page_id"]) else { return Err("missing source_page_id".into()) };
    let entry = pages.entry(page).or_insert_with(|| page_text(db, page).map(|(_, s)| (Index::new(&s.text), s.ids)));
    let Some((index, ids)) = entry else { return Err(format!("page_id={page} does not exist")) };
    let (ok, line, missing) = index.check(&str_of(&args["quote"]), answer);
    if !ok {
        return Err(if line.is_none() {
            format!("the quote was not found in page_id={page}; copy it word for word")
        } else {
            format!("these words are not in the page: {}", missing.join(", "))
        });
    }
    Ok((page, line.and_then(|n| ids.get(n - 1).copied())))
}

fn call_tool(db: &Db, clock: Clock, name: &str, args: &Value, has_image: bool) -> Outcome {
    let mut pages = HashMap::new();
    match name {
        "search_notes" => {
            let q = str_of(&args["query"]);
            let hits = db.search_pages(&q, 40).unwrap_or_default();
            let mut seen: Vec<i64> = vec![];
            let mut out = String::new();
            for (page, _, snippet) in hits {
                if seen.contains(&page) || seen.len() >= 8 {
                    continue;
                }
                let Ok(Some(meta)) = db.page(page) else { continue };
                seen.push(page);
                out.push_str(&format!("page_id={page} | {} | {}\n", meta.title.trim(), snippet.replace('\n', " ")));
            }
            if seen.is_empty() {
                out = "No page matches. Try fewer or different keywords.".into();
            }
            Outcome::Read { text: out, arg: q, count: seen.len() }
        }
        "read_page" => {
            let id = int(&args["page_id"]).unwrap_or(0);
            match page_text(db, id) {
                Some((title, s)) => {
                    let mut text = s.text;
                    if text.len() > MAX_PAGE_CHARS {
                        let cut = (0..=MAX_PAGE_CHARS).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
                        text.truncate(cut);
                        text.push_str("\n(…page truncated)");
                    }
                    Outcome::Read { text, arg: title, count: s.ids.len() }
                }
                None => Outcome::Read { text: format!("No page with page_id={id}."), arg: id.to_string(), count: 0 },
            }
        }
        "list_decks_and_banks" => {
            let (subjects, decks) = (db.list_subjects().unwrap_or_default(), db.list_decks().unwrap_or_default());
            let mut out = String::new();
            for s in &subjects {
                out.push_str(&format!("Subject \"{}\"\n", s.name));
                for d in decks.iter().filter(|d| d.subject_id == s.id) {
                    out.push_str(&format!("  deck_id={} | {}\n", d.id, crate::search::deck_path(&decks, d.id)));
                }
                for b in db.list_banks(s.id).unwrap_or_default() {
                    out.push_str(&format!("  bank_id={} | {}\n", b.id, b.name));
                }
            }
            Outcome::Read { text: out, arg: String::new(), count: decks.len() }
        }
        "search_cards" => {
            let q = str_of(&args["query"]);
            let (subjects, decks) = (db.list_subjects().unwrap_or_default(), db.list_decks().unwrap_or_default());
            let parsed = crate::search::parse(&q, &decks, &subjects, &clock, crate::srs::now_ms());
            if let Some(e) = parsed.errors.first() {
                return Outcome::Read { text: format!("Query error: {e}"), arg: q, count: 0 };
            }
            // shortcut: `prop:r` (filtrado depois, no navegador) é ignorado aqui.
            let sql = format!("SELECT c.id FROM cards c JOIN notes n ON n.id = c.note_id WHERE {} ORDER BY c.id DESC", parsed.sql);
            let ids: Vec<i64> = db.conn.prepare(&sql).and_then(|mut st| st.query_map([], |r| r.get(0))?.collect()).unwrap_or_default();
            let cards = db.cards_by_ids(&ids).unwrap_or_default();
            let mut by_note: Vec<i64> = vec![];
            for c in &cards {
                if !by_note.contains(&c.note_id) {
                    by_note.push(c.note_id);
                }
            }
            let total = by_note.len();
            by_note.truncate(25);
            let notes = db.notes_by_ids(&by_note).unwrap_or_default();
            let mut out = String::new();
            for id in &by_note {
                let Some(n) = notes.get(id) else { continue };
                let c = cards.iter().find(|c| c.note_id == *id).unwrap();
                let fields: Vec<String> = n.fields.iter().filter(|f| !f.trim().is_empty()).map(|f| f.replace('\n', " ")).collect();
                out.push_str(&format!(
                    "note_id={id} | {} | {} | {} | state={:?} reps={} lapses={}\n",
                    crate::search::deck_path(&decks, c.deck_id),
                    n.kind.key(),
                    fields.join(" || "),
                    c.s.state,
                    c.s.reps,
                    c.s.lapses
                ));
            }
            if total > by_note.len() {
                out.push_str(&format!("({} more not shown; refine the query)\n", total - by_note.len()));
            }
            if total == 0 {
                out = "No cards match.".into();
            }
            Outcome::Read { text: out, arg: q, count: total }
        }
        "search_questions" => {
            let q = str_of(&args["query"]);
            let found = db.search_questions(&q, 20).unwrap_or_default();
            let mut out = String::new();
            for x in &found {
                let bank = db.bank(x.bank_id).ok().flatten().map(|b| b.name).unwrap_or_default();
                let opts: Vec<String> = x.options.iter().enumerate().map(|(i, o)| format!("{}) {o}", (b'A' + i as u8) as char)).collect();
                out.push_str(&format!("question_id={} | bank \"{bank}\" | {} | {} | answer={} | right {}/{}\n", x.id, x.stem.replace('\n', " "), opts.join(" "), x.answer, x.correct, x.attempts));
            }
            if found.is_empty() {
                out = "No questions match.".into();
            }
            Outcome::Read { text: out, arg: q, count: found.len() }
        }
        "create_cards" => {
            let Some(deck_id) = int(&args["deck_id"]) else { return Outcome::Error("missing deck_id".into()) };
            let Some(deck) = deck_name(db, deck_id) else { return Outcome::Error(format!("deck_id={deck_id} does not exist; call list_decks_and_banks")) };
            let mut cards = vec![];
            let mut notes = String::new();
            for (i, c) in args["cards"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                let cloze = str_of(&c["type"]) == "cloze";
                let (front, back) = (str_of(&c["front"]), str_of(&c["back"]));
                let problem = if cloze && cards::cloze_numbers(&front).is_empty() {
                    Some("a cloze card needs {{c1::…}} in front".to_owned())
                } else if !cloze && (front.is_empty() || back.is_empty()) {
                    Some("a basic card needs front and back".to_owned())
                } else {
                    None
                };
                if let Some(p) = problem {
                    notes.push_str(&format!("Card {} rejected: {p}.\n", i + 1));
                    continue;
                }
                let from_image = has_image && c["from_image"].as_bool().unwrap_or(false);
                let answer = if cloze { cards::strip_cloze(&front) } else { back.clone() };
                let (page, block) = if from_image {
                    (None, None)
                } else {
                    match ground(db, c, &answer, &mut pages) {
                        Ok((p, b)) => (Some(p), b),
                        Err(e) => {
                            notes.push_str(&format!("Card {} rejected: {e}.\n", i + 1));
                            continue;
                        }
                    }
                };
                cards.push(NewCard { cloze, front, back, page, block, from_image });
            }
            if cards.is_empty() {
                return Outcome::Error(format!("No valid cards.\n{notes}"));
            }
            Outcome::Propose { action: Action::CreateCards { deck_id, deck, cards }, notes }
        }
        "edit_card" => {
            let Some(id) = int(&args["note_id"]) else { return Outcome::Error("missing note_id".into()) };
            let Ok(Some(n)) = db.note(id) else { return Outcome::Error(format!("note_id={id} does not exist")) };
            let before = [n.fields.first().cloned().unwrap_or_default(), n.fields.get(1).cloned().unwrap_or_default()];
            let pick = |k: &str, old: &String| args.get(k).and_then(Value::as_str).map(|s| s.trim().to_owned()).unwrap_or_else(|| old.clone());
            let after = [pick("front", &before[0]), pick("back", &before[1])];
            if after == before {
                return Outcome::Error("nothing changes".into());
            }
            Outcome::Propose { action: Action::EditCard { note_id: id, cloze: n.kind == cards::NoteKind::Cloze, before, after }, notes: String::new() }
        }
        "delete_cards" => {
            let ids: Vec<i64> = args["note_ids"].as_array().map(|a| a.iter().filter_map(int).collect()).unwrap_or_default();
            let notes = db.notes_by_ids(&ids).unwrap_or_default();
            let ids: Vec<i64> = ids.into_iter().filter(|i| notes.contains_key(i)).collect();
            if ids.is_empty() {
                return Outcome::Error("none of these note_ids exist".into());
            }
            let previews = ids.iter().map(|i| crate::rich::plain(&cards::strip_cloze(&cards::sort_field(notes[i].kind, &notes[i].fields)))).collect();
            Outcome::Propose { action: Action::DeleteCards { note_ids: ids, previews }, notes: String::new() }
        }
        "create_questions" => {
            let Some(bank_id) = int(&args["bank_id"]) else { return Outcome::Error("missing bank_id".into()) };
            let Ok(Some(bank)) = db.bank(bank_id) else { return Outcome::Error(format!("bank_id={bank_id} does not exist; call list_decks_and_banks")) };
            let mut questions = vec![];
            let mut notes = String::new();
            for (i, q) in args["questions"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                let opts: Vec<String> = q["options"].as_array().map(|a| a.iter().map(option).collect()).unwrap_or_default();
                let answer = int(&q["answer"]).unwrap_or(-1);
                if str_of(&q["stem"]).is_empty() || opts.len() != 4 || opts.iter().any(String::is_empty) || !(0..4).contains(&answer) {
                    notes.push_str(&format!("Question {} rejected: it needs a stem, 4 options and answer 0-3.\n", i + 1));
                    continue;
                }
                let from_image = has_image && q["from_image"].as_bool().unwrap_or(false);
                let (page, block) = if from_image {
                    (None, None)
                } else {
                    match ground(db, q, &opts[answer as usize], &mut pages) {
                        Ok((p, b)) => (Some(p), b),
                        Err(e) => {
                            notes.push_str(&format!("Question {} rejected: {e}.\n", i + 1));
                            continue;
                        }
                    }
                };
                let options = [opts[0].clone(), opts[1].clone(), opts[2].clone(), opts[3].clone()];
                questions.push(NewQuestion { stem: str_of(&q["stem"]), options, answer: answer as usize, explanation: str_of(&q["explanation"]), page, block, from_image });
            }
            if questions.is_empty() {
                return Outcome::Error(format!("No valid questions.\n{notes}"));
            }
            Outcome::Propose { action: Action::CreateQuestions { bank_id, bank: bank.name, questions }, notes }
        }
        "edit_question" => {
            let Some(id) = int(&args["question_id"]) else { return Outcome::Error("missing question_id".into()) };
            let Ok(Some(q)) = db.question(id) else { return Outcome::Error(format!("question_id={id} does not exist")) };
            let before = NewQuestion::of(&q);
            let mut after = before.clone();
            if let Some(s) = args.get("stem").and_then(Value::as_str) {
                after.stem = s.trim().to_owned();
            }
            if let Some(o) = args.get("options").and_then(Value::as_array).filter(|o| o.len() == 4) {
                for (slot, v) in after.options.iter_mut().zip(o) {
                    *slot = option(v);
                }
            }
            if let Some(a) = args.get("answer").and_then(int).filter(|a| (0..4).contains(a)) {
                after.answer = a as usize;
            }
            if let Some(s) = args.get("explanation").and_then(Value::as_str) {
                after.explanation = s.trim().to_owned();
            }
            if after == before {
                return Outcome::Error("nothing changes".into());
            }
            Outcome::Propose { action: Action::EditQuestion { id, before, after }, notes: String::new() }
        }
        "delete_questions" => {
            let qs: Vec<_> = args["question_ids"].as_array().map(|a| a.iter().filter_map(int).filter_map(|i| db.question(i).ok().flatten()).collect()).unwrap_or_default();
            if qs.is_empty() {
                return Outcome::Error("none of these question_ids exist".into());
            }
            Outcome::Propose { action: Action::DeleteQuestions { ids: qs.iter().map(|q| q.id).collect(), previews: qs.iter().map(|q| crate::rich::plain(&q.stem)).collect() }, notes: String::new() }
        }
        _ => Outcome::Error(format!("unknown tool {name}")),
    }
}

/// Espera a decisão da interface, sem travar o cancelamento.
fn wait(rx: &Receiver<Reply>, cancel: &AtomicBool) -> Result<Reply> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(r) => return Ok(r),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("cancelled"),
        }
    }
}

/// Base64 padrão, para mandar imagens no pedido.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A imagem `media/arquivo` como data URL JPEG, reduzida se for grande.
fn image_url(media: &Path, name: &str) -> Result<String> {
    let mut img = image::open(media.join(name))?;
    if img.width().max(img.height()) > MAX_IMAGE_SIDE {
        img = img.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(format!("data:image/jpeg;base64,{}", base64(buf.get_ref())))
}

/// Troca as referências `media:arquivo` das mensagens pelas imagens em si.
fn with_images(messages: &[Value], media: &Path, cache: &mut HashMap<String, String>) -> Result<Vec<Value>> {
    let mut out = messages.to_vec();
    for m in &mut out {
        let Some(parts) = m["content"].as_array_mut() else { continue };
        for p in parts {
            if let Some(name) = p.pointer("/image_url/url").and_then(Value::as_str).and_then(|u| u.strip_prefix("media:")).map(str::to_owned) {
                if !cache.contains_key(&name) {
                    cache.insert(name.clone(), image_url(media, &name)?);
                }
                p["image_url"]["url"] = json!(cache[&name]);
            }
        }
    }
    Ok(out)
}

fn has_image(messages: &[Value]) -> bool {
    messages.iter().any(|m| m["content"].as_array().is_some_and(|a| a.iter().any(|p| p["type"] == "image_url")))
}

fn run(req: &Request, db: &Db, cancel: &AtomicBool, send: &dyn Fn(Event)) -> Result<()> {
    let mut system = SYSTEM.to_owned();
    if let Some(c) = &req.context {
        system.push_str(&format!("\nThe user is looking at the page \"{}\" (page_id={}). Questions like \"this page\" refer to it; read it first.", c.title.trim(), c.page_id));
    }
    let mut messages = vec![json!({"role": "system", "content": system})];
    messages.extend(req.history.iter().cloned());
    let start = messages.len();
    messages.push(json!({"role": "user", "content": req.user}));
    let images = has_image(&messages);
    let mut cache = HashMap::new();
    let mut nudged = false;
    let mut created = std::collections::HashSet::new();
    for _ in 0..MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let body = json!({"model": req.api.model.trim(), "messages": with_images(&messages, &db.media_dir(), &mut cache)?, "tools": tools(), "temperature": 0.3});
        let resp = online::post_json(&req.api, "chat/completions", &body)?;
        if let Some(n) = resp.pointer("/usage/total_tokens").and_then(Value::as_u64) {
            send(Event::Usage(n));
            crate::diag::info(format!("assistant usage: {}", resp["usage"]));
        }
        let msg = &resp["choices"][0]["message"];
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        let text = msg["content"].as_str().unwrap_or_default().trim().to_owned();
        if calls.is_empty() {
            if text.is_empty() {
                bail!("the model answered with no text");
            }
            messages.push(json!({"role": "assistant", "content": text}));
            // Modelos fracos escrevem os cartões na resposta em vez de chamar a ferramenta: cobra uma vez.
            if !nudged && text.contains("{{c") {
                nudged = true;
                messages.push(json!({"role": "user", "content": "Nothing was saved: cards written in the reply are not created. Call the cards tool with them now (deck_id, and source_page_id + quote for each)."}));
                continue;
            }
            let mut new = messages.split_off(start);
            trim_tool_results(&mut new);
            send(Event::Answer { text, messages: new });
            return Ok(());
        }
        // As chamadas voltam como vieram: o Gemini precisa da assinatura que vem nelas.
        messages.push(json!({"role": "assistant", "content": text, "tool_calls": calls}));
        for call in &calls {
            let name = call["function"]["name"].as_str().unwrap_or_default();
            let args: Value = call["function"]["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or_else(|| call["function"]["arguments"].clone());
            let name = internal_name(name, &args);
            let creates = name.starts_with("create_");
            // Modelos fracos reenviam o lote reescrito até "acertar" a quantidade e duplicam cartões:
            // um lote aprovado por pedido; o usuário pede mais se quiser.
            let outcome = if creates && created.contains(&name) {
                Outcome::Error("already created in this request; don't create more. Tell the user what was saved and which items were rejected.".into())
            } else {
                call_tool(db, req.clock, &name, &args, images)
            };
            let result = match outcome {
                Outcome::Read { text, arg, count } => {
                    send(Event::Tool { name, arg, count });
                    text
                }
                Outcome::Error(e) => format!("Error: {e}"),
                Outcome::Propose { action, notes } => {
                    let (tx, rx) = mpsc::channel();
                    send(Event::Ask { action, reply: tx });
                    match wait(&rx, cancel)? {
                        Reply::Done(s) => {
                            if creates {
                                created.insert(name.clone());
                            }
                            format!("{s}\n{notes}")
                        }
                        Reply::Refused(s) => s,
                    }
                }
            };
            messages.push(json!({"role": "tool", "tool_call_id": call["id"], "content": result}));
        }
    }
    bail!("the assistant took too many steps without answering; try a more specific question")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Block, BlockKind};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// Servidor de mentira: responde cada pedido com a próxima resposta da lista.
    fn fake_api(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = vec![];
            for reply in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                seen.push(serde_json::from_slice(&body).unwrap());
                let out = reply.to_string();
                write!(&stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).unwrap();
            }
            seen
        });
        (url, handle)
    }

    fn call(id: &str, name: &str, args: Value) -> Value {
        json!({"choices": [{"message": {"content": null, "tool_calls": [{"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}}]})
    }

    fn answer(text: &str) -> Value {
        json!({"choices": [{"message": {"content": text}}], "usage": {"total_tokens": 42}})
    }

    /// Banco com uma página "Mitocôndria" e o baralho da disciplina.
    fn notes() -> (PathBuf, i64, i64) {
        let dir = std::env::temp_dir().join(format!("discite-agent-{}", crate::db::new_id()));
        let db = Db::open(&dir).unwrap();
        let s = db.add_subject("Biologia", 0).unwrap();
        let page = db.add_page(s, None, "Mitocôndria").unwrap();
        db.save_blocks(page, "Mitocôndria", &[Block::new(BlockKind::Paragraph, "A mitocôndria produz ATP pela respiração celular.")]).unwrap();
        let deck = db.list_decks().unwrap().into_iter().find(|d| d.subject_id == s).unwrap().id;
        (dir, page, deck)
    }

    fn request(url: String, dir: &Path, user: &str) -> Request {
        Request { api: Api { url, model: "m".into(), ..Default::default() }, root: dir.to_owned(), history: vec![], user: json!(user), context: None, clock: Clock { rollover_hour: 4 } }
    }

    #[test]
    fn searches_reads_and_answers() {
        let (dir, page, _) = notes();
        let (url, server) = fake_api(vec![
            call("a", "search_notes", json!({"query": "mitocôndria"})),
            call("b", "read_page", json!({"page_id": page.to_string()})),
            answer("Produz ATP [Mitocôndria]."),
        ]);
        let events = std::sync::Mutex::new(vec![]);
        run(&request(url, &dir, "O que a mitocôndria faz?"), &open_read_only(&dir).unwrap(), &AtomicBool::new(false), &|e| events.lock().unwrap().push(e)).unwrap();
        let seen = server.join().unwrap();

        let events = events.into_inner().unwrap();
        assert!(matches!(&events[0], Event::Tool { name, count: 1, .. } if name == "search_notes"));
        assert!(matches!(&events[1], Event::Tool { name, arg, count: 1 } if name == "read_page" && arg == "Mitocôndria"));
        assert!(matches!(events[2], Event::Usage(42)));
        let Event::Answer { text, messages } = &events[3] else { panic!("{events:?}") };
        assert_eq!(text, "Produz ATP [Mitocôndria].");
        // Pergunta, 2 × (chamada + resultado), resposta.
        assert_eq!(messages.len(), 6);
        let last = seen[2]["messages"].as_array().unwrap();
        assert!(last[last.len() - 1]["content"].as_str().unwrap().contains("A mitocôndria produz ATP"));
        assert!(last[last.len() - 3]["content"].as_str().unwrap().contains(&format!("page_id={page}")));
        assert_eq!(seen[0]["tools"].as_array().unwrap().len(), 5);
    }

    /// Cartão com trecho certo vai para aprovação; o inventado é recusado; a
    /// resposta da interface volta ao modelo.
    #[test]
    fn new_cards_are_checked_against_the_page_and_wait_for_approval() {
        let (dir, page, deck) = notes();
        let cards = json!([
            {"type": "basic", "front": "O que a mitocôndria produz?", "back": "ATP", "source_page_id": page, "quote": "A mitocôndria produz ATP"},
            {"type": "basic", "front": "Quem descobriu a mitocôndria?", "back": "Kölliker em 1857", "source_page_id": page, "quote": "descoberta por Kölliker"},
        ]);
        let (url, server) = fake_api(vec![call("a", "cards", json!({"op": "create", "deck_id": deck, "cards": cards})), answer("Pronto.")]);
        let db = open_read_only(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let req = request(url, &dir, "Crie cartões");
        let worker = std::thread::spawn(move || run(&req, &db, &AtomicBool::new(false), &|e| tx.send(e).unwrap()));
        let Event::Ask { action, reply } = rx.recv().unwrap() else { panic!() };
        let Action::CreateCards { deck_id, cards, .. } = &action else { panic!("{action:?}") };
        assert_eq!(*deck_id, deck);
        assert_eq!(cards.len(), 1);
        assert_eq!((cards[0].back.as_str(), cards[0].page), ("ATP", Some(page)));
        assert!(cards[0].block.is_some());
        reply.send(Reply::Done("Created 1 notes.".into())).unwrap();
        worker.join().unwrap().unwrap();
        let seen = server.join().unwrap();
        let msgs = seen[1]["messages"].as_array().unwrap();
        let result = msgs.last().unwrap()["content"].as_str().unwrap();
        assert!(result.contains("Created 1 notes.") && result.contains("Card 2 rejected"), "{result}");
    }

    /// Cartões escritos na resposta não contam: o modelo é cobrado a chamar a ferramenta.
    #[test]
    fn cards_written_as_text_are_sent_back_to_the_tool() {
        let (dir, page, deck) = notes();
        let cards = json!([{"type": "cloze", "front": "A mitocôndria produz {{c1::ATP}}.", "source_page_id": page, "quote": "A mitocôndria produz ATP"}]);
        let (url, server) = fake_api(vec![
            answer("1. A mitocôndria produz {{c1::ATP}}."),
            call("a", "cards", json!({"op": "create", "deck_id": deck, "cards": cards})),
            call("b", "cards", json!({"op": "create", "deck_id": deck, "cards": cards})),
            answer("Pronto."),
        ]);
        let db = open_read_only(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let req = request(url, &dir, "Crie cartões");
        let worker = std::thread::spawn(move || run(&req, &db, &AtomicBool::new(false), &|e| tx.send(e).unwrap()));
        let ask = rx.iter().find_map(|e| match e {
            Event::Ask { action, reply } => Some((action, reply)),
            _ => None,
        });
        let (action, reply) = ask.unwrap();
        assert!(matches!(&action, Action::CreateCards { cards, .. } if cards.len() == 1));
        reply.send(Reply::Done("Created 1 notes.".into())).unwrap();
        let text = rx.iter().find_map(|e| if let Event::Answer { text, .. } = e { Some(text) } else { None });
        assert_eq!(text.as_deref(), Some("Pronto."));
        worker.join().unwrap().unwrap();
        let seen = server.join().unwrap();
        assert!(seen[1]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().starts_with("Nothing was saved"));
        // O segundo lote no mesmo pedido é recusado sem perguntar ao usuário.
        assert!(seen[3]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("already created"));
    }

    #[test]
    fn option_letters_are_dropped() {
        assert_eq!(option(&json!("B) Membrana plasmática")), "Membrana plasmática");
        assert_eq!(option(&json!("d. 42")), "42");
        assert_eq!(option(&json!("ATP")), "ATP");
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn read_only_connection_cannot_write() {
        let (dir, _, _) = notes();
        assert!(open_read_only(&dir).unwrap().add_subject("x", 0).is_err());
    }
}
