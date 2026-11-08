//! O que vai para o modelo (instruções + notas numeradas por bloco) e o que
//! volta: uma gramática que só aceita o JSON dos cartões (ou das questões) e
//! um leitor que entrega cada um assim que o objeto dele fecha.

use super::ground::Index;
use crate::cards::{self, NoteKind};
use crate::db::{Block, BlockKind};
use serde::Deserialize;

/// Que tipo de cartão pedir.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, Deserialize)]
pub enum Style {
    /// O modelo escolhe: pergunta e resposta para fatos, lacuna para termos.
    #[default]
    Auto,
    Basic,
    Cloze,
}

/// Quão difíceis devem ser os cartões e as questões.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, Deserialize)]
pub enum Level {
    Easy,
    #[default]
    Medium,
    Hard,
}

/// Teto de caracteres das notas mandadas ao modelo (~4 mil tokens), para
/// caber no contexto junto com as instruções e a resposta.
const MAX_NOTES_CHARS: usize = 14_000;

/// As notas como o modelo as vê: `[n] texto`, um bloco por linha. `ids[n - 1]`
/// é o bloco de origem do número `n`.
pub struct Source {
    pub text: String,
    pub ids: Vec<i64>,
    /// Ficou algo de fora por causa do tamanho.
    pub truncated: bool,
}

pub fn source(title: &str, blocks: &[Block]) -> Source {
    let mut text = format!("Page: {}\n", title.trim());
    let mut ids = vec![];
    let mut truncated = false;
    for b in blocks {
        let line = match b.kind {
            BlockKind::Divider => continue,
            BlockKind::Image if b.caption.trim().is_empty() => continue,
            BlockKind::Image => format!("(image) {}", b.caption.trim()),
            BlockKind::H1 => format!("# {}", b.plain().trim()),
            BlockKind::H2 => format!("## {}", b.plain().trim()),
            BlockKind::H3 => format!("### {}", b.plain().trim()),
            BlockKind::Bullet | BlockKind::Check => format!("{}- {}", "  ".repeat(b.indent as usize), b.plain().trim()),
            BlockKind::Numbered => format!("{}1. {}", "  ".repeat(b.indent as usize), b.plain().trim()),
            _ => b.plain().trim().to_owned(),
        };
        if line.trim_start_matches(['#', '-', ' ', '1', '.']).trim().is_empty() {
            continue;
        }
        let line = format!("[{}] {}\n", ids.len() + 1, line.replace('\n', " "));
        if text.len() + line.len() > MAX_NOTES_CHARS {
            truncated = true;
            break;
        }
        text.push_str(&line);
        ids.push(b.id);
    }
    Source { text, ids, truncated }
}

// Sem frase de exemplo: o modelo pequeno copiava o exemplo como se fosse um
// cartão ("The mitochondria produce…" numa página sobre a Revolução Francesa).
// O trecho de prova ("quote") vem depois do cartão: pedido antes, os modelos
// pequenos só copiavam a frase da nota como pergunta e como resposta.
const SYSTEM: &str = r#"You write spaced-repetition flashcards from a student's notes.
Rules:
- Use only what is written in the notes. Never add facts, explanations, causes, examples or numbers that are not in the notes, even if you know them. Never reverse or change what the notes say.
- One fact per card. A card must make sense on its own. Never repeat a fact.
- "basic": "front" is a specific question about one fact; "back" is the short answer: a few words, at most one short sentence. Never copy the whole passage into "back".
- "cloze": "text" is one sentence from the notes with the key term wrapped in {{c1::…}}, left in its original place.
- "quote": the exact words of the notes that back the card, copied as they are (one sentence or less).
- "block" is the number in brackets of the note line the quote comes from.
- Skip anything already covered by the existing cards.
- Prefer the most important ideas: definitions, causes, functions, differences, numbers.
Answer with JSON only."#;

// Questões: o modelo dá a certa e três erradas separadas, e o app sorteia a
// posição; pedindo as quatro alternativas, os modelos pequenos punham a certa
// quase sempre na primeira.
const SYSTEM_QUESTIONS: &str = r#"You write multiple-choice exam questions from a student's notes.
Rules:
- Each question tests one fact that is written in the notes. The correct answer must come from the notes. Never add facts that are not in the notes, and never reverse or change what the notes say.
- "question": a clear question (or an incomplete statement) that makes sense on its own and does not give the answer away.
- "correct": the correct option, short (a few words, at most one sentence). Options only give the answer: never repeat the question in them.
- "wrong": three options that are clearly wrong according to the notes but plausible to someone who did not study: other terms, numbers or ideas of the same kind and about the same length as the correct one. Never "all of the above" or "none of the above". Never a second correct option. No letters (A, B…) before the options.
- "explanation": one short sentence saying why the correct option is right, using the notes.
- "quote": the exact words of the notes that back the correct answer, copied as they are (one sentence or less).
- "block" is the number in brackets of the note line the quote comes from.
- One fact per question. Never repeat a fact. Skip anything already covered by the existing questions.
- Prefer the most important ideas: definitions, causes, functions, differences, numbers.
Answer with JSON only."#;

pub struct Request<'a> {
    pub source: &'a Source,
    /// Perguntas/lacunas dos cartões que a página já tem.
    pub existing: &'a [String],
    pub count: u32,
    pub style: Style,
    /// Trecho escolhido pelo usuário dentro das notas (seleção ou bloco).
    pub focus: Option<&'a str>,
    pub level: Level,
    /// Instruções livres do usuário ("foque em doses").
    pub guide: &'a str,
}

/// Dificuldade e instruções do usuário, iguais para cartões e questões.
fn extras(r: &Request) -> String {
    let mut s = match r.level {
        Level::Easy => " Keep it easy: ask about the main ideas and definitions, in direct wording.".to_owned(),
        Level::Medium => String::new(),
        Level::Hard => " Make it demanding: ask about details, exceptions, numbers, comparisons and how ideas relate, still using only facts from the notes; wrong options must be close to the right one.".to_owned(),
    };
    // O texto vem do usuário: vai entre aspas e não pode passar por cima das regras.
    let guide: String = r.guide.trim().chars().take(400).collect();
    if !guide.is_empty() {
        s.push_str(&format!("\nThe student also asked: \"{}\". Follow this when choosing what to ask, but keep every rule above (only facts from the notes, the same JSON).", guide.replace('"', "'")));
    }
    s
}

/// O pedido sem o formato de um modelo: as regras (sistema) e as notas com o
/// que gerar (usuário). Cada motor monta o texto final do seu jeito.
#[derive(Clone, Debug)]
pub struct Chat {
    pub system: &'static str,
    pub user: String,
}

/// Formato ChatML (o dos modelos Qwen), com o raciocínio ("thinking")
/// desligado: a resposta começa direto no JSON.
pub fn chatml(c: &Chat) -> String {
    format!("<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n", c.system, c.user)
}

/// O JSON esperado, por escrito: na IA online não há gramática que o garanta.
pub fn json_shape(questions: bool, style: Style) -> &'static str {
    match (questions, style) {
        (true, _) => r#"Answer with JSON exactly in this shape: {"questions": [{"question": "...", "correct": "...", "wrong": ["...", "...", "..."], "explanation": "...", "quote": "...", "block": 1}]}"#,
        (false, Style::Basic) => r#"Answer with JSON exactly in this shape: {"cards": [{"type": "basic", "front": "...", "back": "...", "quote": "...", "block": 1}]}"#,
        (false, Style::Cloze) => r#"Answer with JSON exactly in this shape: {"cards": [{"type": "cloze", "text": "... {{c1::...}} ...", "quote": "...", "block": 1}]}"#,
        (false, Style::Auto) => r#"Answer with JSON exactly in this shape: {"cards": [{"type": "basic", "front": "...", "back": "...", "quote": "...", "block": 1}, {"type": "cloze", "text": "... {{c1::...}} ...", "quote": "...", "block": 2}]}"#,
    }
}

/// O pedido de cartões.
pub fn prompt(r: &Request) -> Chat {
    let mut user = format!("Notes:\n{}\n", r.source.text);
    if let Some(f) = r.focus {
        user.push_str(&format!("\nOnly make cards about this part of the notes:\n\"{}\"\n", f.trim()));
    }
    if !r.existing.is_empty() {
        user.push_str("\nExisting cards:\n");
        for e in r.existing.iter().take(60) {
            user.push_str(&format!("- {}\n", e.replace('\n', " ")));
        }
    }
    let kind = match r.style {
        Style::Auto => "Mix \"basic\" and \"cloze\" cards, whichever fits each fact best.",
        Style::Basic => "Use only \"basic\" cards.",
        Style::Cloze => "Use only \"cloze\" cards.",
    };
    user.push_str(&format!("\nWrite {} new flashcards; write fewer only if the notes run out of distinct facts. {kind}{}", r.count, extras(r)));
    // Dito com todas as letras: o modelo pequeno respondia em inglês a notas em português.
    // Os cartões saem no idioma do app; o trecho de prova, no das notas.
    let lang = card_language();
    user.push_str(&format!(" Write every card in {lang}."));
    if let Some(notes) = super::ground::language(&r.source.text).filter(|n| !lang.ends_with(n)) {
        user.push_str(&format!(" The notes are in {notes}: translate the facts into {lang}, but copy the quote exactly as it is in the notes."));
    }
    Chat { system: SYSTEM, user }
}

/// Prompt das questões objetivas (mesmas notas, foco e idioma dos cartões).
pub fn prompt_questions(r: &Request) -> Chat {
    let mut user = format!("Notes:\n{}\n", r.source.text);
    if let Some(f) = r.focus {
        user.push_str(&format!("\nOnly write questions about this part of the notes:\n\"{}\"\n", f.trim()));
    }
    if !r.existing.is_empty() {
        user.push_str("\nExisting questions:\n");
        for e in r.existing.iter().take(60) {
            user.push_str(&format!("- {}\n", e.replace('\n', " ")));
        }
    }
    user.push_str(&format!("\nWrite {} new multiple-choice questions; write fewer only if the notes run out of distinct facts.{}", r.count, extras(r)));
    let lang = card_language();
    user.push_str(&format!(" Write every question, option and explanation in {lang}."));
    if let Some(notes) = super::ground::language(&r.source.text).filter(|n| !lang.ends_with(n)) {
        user.push_str(&format!(" The notes are in {notes}: translate the facts into {lang}, but copy the quote exactly as it is in the notes."));
    }
    Chat { system: SYSTEM_QUESTIONS, user }
}

/// Gramática GBNF das questões: `{"questions": [...]}` com no máximo `max`.
pub fn grammar_questions(max: u32) -> String {
    let more = max.saturating_sub(1);
    format!(
        r#"root ::= "{{" ws "\"questions\":" ws "[" ws ( qn ( ws "," ws qn ){{0,{more}}} )? ws "]" ws "}}"
qn ::= "{{" ws "\"question\":" ws q ws "," ws "\"correct\":" ws o ws "," ws "\"wrong\":" ws "[" ws o ws "," ws o ws "," ws o ws "]" ws "," ws "\"explanation\":" ws s ws "," ws "\"quote\":" ws s ws "," ws "\"block\":" ws int ws "}}"
q ::= "\"" ch{{1,400}} "\""
o ::= "\"" ch{{1,200}} "\""
s ::= "\"" ch{{1,400}} "\""
ch ::= [^"\\\x7F\x00-\x1F] | "\\" ( ["\\/bfnrt] | "u" [0-9a-fA-F]{{4}} )
int ::= [0-9]{{1,4}}
ws ::= [ \t\n]{{0,8}}
"#
    )
}

/// Idioma dos cartões: o escolhido no app.
pub fn card_language() -> &'static str {
    // Português do Brasil, com todas as letras: só "Portuguese" pode sair de Portugal.
    if crate::i18n::current().starts_with("pt") { "Brazilian Portuguese" } else { "English" }
}

/// Gramática GBNF que só aceita `{"cards": [...]}` com no máximo `max` cartões.
pub fn grammar(style: Style, max: u32) -> String {
    let card = match style {
        Style::Auto => "basic | cloze",
        Style::Basic => "basic",
        Style::Cloze => "cloze",
    };
    let more = max.saturating_sub(1).max(0);
    format!(
        r#"root ::= "{{" ws "\"cards\":" ws "[" ws ( card ( ws "," ws card ){{0,{more}}} )? ws "]" ws "}}"
card ::= {card}
basic ::= "{{" ws "\"type\":" ws "\"basic\"" ws "," ws "\"front\":" ws q ws "," ws "\"back\":" ws a ws "," ws "\"quote\":" ws s ws "," ws "\"block\":" ws int ws "}}"
cloze ::= "{{" ws "\"type\":" ws "\"cloze\"" ws "," ws "\"text\":" ws s ws "," ws "\"quote\":" ws s ws "," ws "\"block\":" ws int ws "}}"
q ::= "\"" ch{{1,300}} "\""
a ::= "\"" ch{{1,400}} "\""
s ::= "\"" ch{{1,400}} "\""
ch ::= [^"\\\x7F\x00-\x1F] | "\\" ( ["\\/bfnrt] | "u" [0-9a-fA-F]{{4}} )
int ::= [0-9]{{1,4}}
ws ::= [ \t\n]{{0,8}}
"#
    )
}

/// Um cartão sugerido, já no formato de nota do Discite.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub kind: NoteKind,
    /// Básico: frente e verso. Cloze: o texto com as lacunas em `front`.
    pub front: String,
    pub back: String,
    /// Bloco de origem (id), quando o número devolvido existe.
    pub block: Option<i64>,
    /// Trecho copiado das notas que comprova o cartão.
    pub quote: String,
    /// O trecho existe nas notas e a resposta só usa palavras delas.
    pub grounded: bool,
    /// Palavras da resposta que não aparecem nas notas.
    pub missing: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    front: String,
    #[serde(default)]
    back: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    block: usize,
    #[serde(default)]
    quote: String,
}

impl Suggestion {
    /// `translated`: as notas estão em outro idioma que o dos cartões.
    fn from_raw(r: Raw, ids: &[i64], notes: &Index, translated: bool) -> Option<Suggestion> {
        let quote = unlist(&r.quote);
        let (kind, front, back) = if r.kind == "cloze" {
            let text = fix_cloze(&unlist(&r.text));
            // Uma lacuna que cobre a frase toda não pergunta nada ("{{c1::Fases da mitose}}").
            if cards::cloze_numbers(&text).is_empty() || cards::strip_cloze(&text).split_whitespace().count() < 4 {
                return None;
            }
            (NoteKind::Cloze, text, String::new())
        } else {
            let (front, back) = (unlist(&r.front), unlist(&r.back));
            // Pergunta que é a própria resposta (o modelo copiou a frase nos dois)
            // ou uma afirmação longa no lugar da pergunta não serve.
            let statement = !front.contains('?') && front.split_whitespace().count() >= 8;
            if front.is_empty() || back.is_empty() || similar(&front, &back) || statement {
                return None;
            }
            (NoteKind::Basic, front, back)
        };
        // Lacuna: o texto inteiro tem de vir da nota; básico: a resposta.
        let answer = if kind == NoteKind::Cloze { cards::strip_cloze(&front) } else { back.clone() };
        let (grounded, line, missing) = notes.check(&quote, &answer);
        // Traduzido, as palavras da resposta não estão nas notas: vale só o trecho de prova.
        let (grounded, missing) = if translated { (line.is_some(), vec![]) } else { (grounded, missing) };
        // O bloco de origem é onde o trecho foi achado (o número que o modelo
        // dá às vezes erra por um).
        let n = line.unwrap_or(r.block);
        let block = n.checked_sub(1).and_then(|i| ids.get(i)).copied();
        Some(Suggestion { kind, front, back, block, quote, grounded, missing })
    }

    /// Os campos da nota (Básico: frente, verso, extra; Cloze: texto, extra).
    pub fn fields(&self) -> Vec<String> {
        match self.kind {
            NoteKind::Cloze => vec![self.front.clone(), String::new()],
            _ => vec![self.front.clone(), self.back.clone(), String::new()],
        }
    }

    /// O texto que identifica o cartão (para comparar com os que já existem).
    pub fn key(&self) -> String {
        cards::sort_field(self.kind, &self.fields())
    }
}

/// Uma questão objetiva sugerida.
#[derive(Clone, Debug, PartialEq)]
pub struct QSuggestion {
    pub stem: String,
    pub options: [String; 4],
    /// Posição da certa (sorteada).
    pub answer: usize,
    pub explanation: String,
    pub block: Option<i64>,
    pub quote: String,
    pub grounded: bool,
    pub missing: Vec<String>,
}

#[derive(Deserialize)]
struct RawQ {
    #[serde(default)]
    question: String,
    #[serde(default)]
    correct: String,
    #[serde(default)]
    wrong: Vec<String>,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    block: usize,
    #[serde(default)]
    quote: String,
}

/// "Todas as anteriores", "None of the above"…: não servem como alternativa.
fn catch_all(o: &str) -> bool {
    let f = super::ground::fold(o);
    ["todas as anteriores", "nenhuma das anteriores", "todas as alternativas", "nenhuma das alternativas", "all of the above", "none of the above"].iter().any(|p| f.contains(p))
}

impl QSuggestion {
    fn from_raw(r: RawQ, ids: &[i64], notes: &Index, translated: bool, slot: usize) -> Option<QSuggestion> {
        let stem = unlist(&r.question);
        let correct = unlist(&r.correct);
        let wrong: Vec<String> = r.wrong.iter().map(|w| unlist(w)).collect();
        if stem.is_empty() || correct.is_empty() || wrong.len() != 3 {
            return None;
        }
        // Quatro alternativas diferentes, sem "todas as anteriores".
        let mut all = vec![correct.clone()];
        all.extend(wrong.iter().cloned());
        let folded: Vec<String> = all.iter().map(|o| super::ground::fold(o).split_whitespace().collect::<Vec<_>>().join(" ")).collect();
        for (i, f) in folded.iter().enumerate() {
            if f.is_empty() || folded[..i].contains(f) || catch_all(&all[i]) {
                return None;
            }
        }
        let quote = unlist(&r.quote);
        let (grounded, line, missing) = notes.check(&quote, &correct);
        let (grounded, missing) = if translated { (line.is_some(), vec![]) } else { (grounded, missing) };
        let n = line.unwrap_or(r.block);
        let block = n.checked_sub(1).and_then(|i| ids.get(i)).copied();
        // A certa vai para `slot`; as erradas ocupam as outras posições na ordem.
        let answer = slot % 4;
        let mut options: [String; 4] = Default::default();
        let mut rest = wrong.into_iter();
        for (i, o) in options.iter_mut().enumerate() {
            *o = if i == answer { correct.clone() } else { rest.next().unwrap_or_default() };
        }
        Some(QSuggestion { stem, options, answer, explanation: r.explanation.trim().to_owned(), block, quote, grounded, missing })
    }

    /// O texto que identifica a questão (para comparar com as que já existem).
    pub fn key(&self) -> String {
        crate::rich::plain(&self.stem)
    }
}

/// O que o leitor entrega.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Card(Suggestion),
    Question(QSuggestion),
}

/// Tira o marcador de lista ou título que o modelo às vezes copia das notas
/// ("1. Prófase — …", "- Cones: …", "### …").
fn unlist(s: &str) -> String {
    let t = s.trim();
    // O número da linha das notas ("[5] …"), que o modelo às vezes copia junto.
    let t = match t.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest.trim_start(),
        _ => t,
    };
    let t = t.trim_start_matches('#').trim_start();
    let t = t.strip_prefix("- ").unwrap_or(t);
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    let t = if digits > 0 && t[digits..].starts_with(". ") { &t[digits + 2..] } else { t };
    t.trim().to_owned()
}

/// Palavras (minúsculas, sem pontuação) de um texto, para comparar cartões.
fn words(s: &str) -> std::collections::BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 2).map(str::to_lowercase).collect()
}

/// `a` repete `b`? Mesmas palavras em pelo menos 75% (Jaccard), o bastante
/// para pegar "Qual a função dos bastonetes?" × "Qual é a função dos bastonetes?".
pub fn similar(a: &str, b: &str) -> bool {
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() || wb.is_empty() {
        return false;
    }
    let inter = wa.intersection(&wb).count() as f32;
    let union = wa.union(&wb).count() as f32;
    inter / union >= 0.75
}

/// Modelos pequenos às vezes põem a lacuna no fim ("… a fóvea concentra
/// cones. {{c1::fóvea}}") em vez de envolver o termo. Se o termo aparece antes
/// na frase, a lacuna vai para o lugar dele.
pub fn fix_cloze(text: &str) -> String {
    let spans = cards::parse_clozes(text);
    let Some(last) = spans.last() else { return text.to_owned() };
    if spans.len() != 1 || !text[last.end..].trim().is_empty() {
        return text.to_owned();
    }
    let before = text[..last.start].trim_end();
    let answer = last.answer.as_str();
    if answer.is_empty() {
        return text.to_owned();
    }
    let lower = before.to_lowercase();
    let Some(at) = lower.find(&answer.to_lowercase()) else { return text.to_owned() };
    // Posição em bytes no texto original (to_lowercase pode mudar tamanhos; só
    // aceitamos quando os tamanhos batem).
    if lower.len() != before.len() || !before.is_char_boundary(at) || !before.is_char_boundary(at + answer.len()) {
        return text.to_owned();
    }
    let original = &before[at..at + answer.len()];
    format!("{}{{{{c{}::{}}}}}{}", &before[..at], last.num, original, &before[at + answer.len()..])
}

/// Lê o JSON enquanto ele é gerado e devolve cada cartão completo, já
/// conferido contra as notas.
pub struct Stream {
    buf: String,
    pos: usize,
    depth: u32,
    in_str: bool,
    esc: bool,
    start: Option<usize>,
    ids: Vec<i64>,
    notes: Index,
    translated: bool,
    /// Lê `{"questions": …}` em vez de `{"cards": …}`.
    questions: bool,
    /// Questões já entregues (para sortear a posição da certa).
    made: usize,
}

impl Stream {
    /// `notes` é o texto mandado ao modelo (`Source::text`); `ids`, os blocos.
    pub fn new(ids: Vec<i64>, notes: &str, questions: bool) -> Stream {
        let mut s = Stream::for_language(ids, notes, card_language());
        s.questions = questions;
        s
    }

    /// Com o idioma dos cartões dado (`lang`), sem depender do idioma do app.
    fn for_language(ids: Vec<i64>, notes: &str, lang: &str) -> Stream {
        let translated = super::ground::language(notes).is_some_and(|l| !lang.ends_with(l));
        Stream { buf: String::new(), pos: 0, depth: 0, in_str: false, esc: false, start: None, ids, notes: Index::new(notes), translated, questions: false, made: 0 }
    }

    pub fn feed(&mut self, piece: &str) -> Vec<Out> {
        self.buf.push_str(piece);
        let mut out = vec![];
        let bytes = self.buf.as_bytes();
        while self.pos < bytes.len() {
            let c = bytes[self.pos];
            if self.in_str {
                if self.esc {
                    self.esc = false;
                } else if c == b'\\' {
                    self.esc = true;
                } else if c == b'"' {
                    self.in_str = false;
                }
            } else {
                match c {
                    b'"' => self.in_str = true,
                    b'{' => {
                        self.depth += 1;
                        // Profundidade 2: um cartão dentro de {"cards": [ … ]}.
                        if self.depth == 2 {
                            self.start = Some(self.pos);
                        }
                    }
                    b'}' => {
                        if self.depth == 2 {
                            if let Some(s) = self.start.take() {
                                let obj = &self.buf[s..=self.pos];
                                if self.questions {
                                    if let Ok(raw) = serde_json::from_str::<RawQ>(obj) {
                                        // Posição da certa: sorteada, sem repetir a anterior.
                                        let slot = (self.made + 1 + rand::random::<u32>() as usize % 3) % 4;
                                        if let Some(q) = QSuggestion::from_raw(raw, &self.ids, &self.notes, self.translated, slot) {
                                            self.made = slot;
                                            out.push(Out::Question(q));
                                        }
                                    }
                                } else if let Ok(raw) = serde_json::from_str::<Raw>(obj) {
                                    out.extend(Suggestion::from_raw(raw, &self.ids, &self.notes, self.translated).map(Out::Card));
                                }
                            }
                        }
                        self.depth = self.depth.saturating_sub(1);
                    }
                    _ => {}
                }
            }
            self.pos += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_trailing_cloze_into_the_sentence() {
        assert_eq!(fix_cloze("A fóvea concentra cones. {{c1::fóvea}}"), "A {{c1::fóvea}} concentra cones.");
        assert_eq!(fix_cloze("Os {{c1::cones}} veem cores."), "Os {{c1::cones}} veem cores.");
        // Termo que não aparece antes: fica como veio.
        assert_eq!(fix_cloze("Camada sensorial do olho. {{c1::retina}}"), "Camada sensorial do olho. {{c1::retina}}");
    }

    #[test]
    fn unlist_drops_markers_copied_from_the_notes() {
        assert_eq!(unlist("[5] 1. Anaphase — sister chromatids separate."), "Anaphase — sister chromatids separate.");
        assert_eq!(unlist("- Cones: visão de cores"), "Cones: visão de cores");
        assert_eq!(unlist("### Fases"), "Fases");
        // Colchetes que não são número de linha ficam.
        assert_eq!(unlist("[ATP] é a moeda energética"), "[ATP] é a moeda energética");
    }

    #[test]
    fn similar_catches_rewordings_only() {
        assert!(similar("Qual a função dos bastonetes?", "Qual é a função dos bastonetes?"));
        assert!(!similar("Qual a função dos bastonetes?", "Qual a função dos cones?"));
    }

    #[test]
    fn stream_yields_each_card_checked_against_the_notes() {
        let notes = "Page: Retina\n[1] A retina é sensorial.\n[2] A fóvea é a região \"central\" da mácula.\n";
        let mut s = Stream::for_language(vec![10, 20], notes, "Portuguese");
        // O primeiro diz bloco 1, mas o trecho está no 2; o terceiro inventa um número.
        let json = r#"{"cards": [{"type": "basic", "block": 1, "quote": "A fóvea é a região \"central\" da mácula", "front": "O que é a {fóvea}?", "back": "Região \"central\" da mácula"},
            {"type": "cloze", "block": 1, "quote": "A retina é sensorial", "text": "A {{c1::retina}} é sensorial."},
            {"type": "basic", "block": 1, "quote": "A retina tem 120 milhões de bastonetes", "front": "Quantos bastonetes?", "back": "120 milhões"}]}"#;
        let mut got = vec![];
        for ch in json.chars() {
            got.extend(s.feed(&ch.to_string()).into_iter().filter_map(|o| match o {
                Out::Card(c) => Some(c),
                Out::Question(_) => None,
            }));
        }
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].kind, NoteKind::Basic);
        assert_eq!(got[0].back, "Região \"central\" da mácula");
        assert_eq!(got[0].block, Some(20));
        assert!(got[0].grounded);
        assert_eq!(got[1].kind, NoteKind::Cloze);
        assert_eq!(got[1].block, Some(10));
        assert!(got[1].grounded);
        assert!(!got[2].grounded);
        assert!(got[2].missing.contains(&"120".to_owned()));
    }

    #[test]
    fn stream_yields_questions_with_the_answer_in_place() {
        let notes = "Page: Retina\n[1] A retina é sensorial.\n[2] A fóvea fica no centro da mácula.\n";
        let mut s = Stream::for_language(vec![10, 20], notes, "Portuguese");
        s.questions = true;
        let json = r#"{"questions": [{"question": "Onde fica a fóvea?", "correct": "No centro da mácula", "wrong": ["Na íris", "No nervo óptico", "Na córnea"], "explanation": "A fóvea fica no centro da mácula.", "quote": "A fóvea fica no centro da mácula", "block": 2},
            {"question": "Qual opção?", "correct": "Retina", "wrong": ["Íris", "Todas as anteriores", "Córnea"], "explanation": "", "quote": "A retina é sensorial", "block": 1},
            {"question": "Repetida?", "correct": "Retina", "wrong": ["retina", "Íris", "Córnea"], "explanation": "", "quote": "A retina é sensorial", "block": 1}]}"#;
        let mut got = vec![];
        for ch in json.chars() {
            got.extend(s.feed(&ch.to_string()));
        }
        // Só a primeira serve: a segunda tem "todas as anteriores", a terceira repete a certa.
        assert_eq!(got.len(), 1);
        let Out::Question(q) = &got[0] else { panic!("questão") };
        assert_eq!(q.options[q.answer], "No centro da mácula");
        let mut opts = q.options.to_vec();
        opts.sort();
        assert_eq!(opts, ["Na córnea", "Na íris", "No centro da mácula", "No nervo óptico"]);
        assert_eq!(q.block, Some(20));
        assert!(q.grounded);
    }

    #[test]
    fn level_and_guide_reach_both_prompts() {
        let blocks = vec![Block::new(BlockKind::Paragraph, "A fóvea fica no centro da mácula.")];
        let src = source("Retina", &blocks);
        let req = Request { source: &src, existing: &[], count: 5, style: Style::Auto, focus: None, level: Level::Hard, guide: "foque na \"fóvea\"" };
        for p in [prompt(&req), prompt_questions(&req)] {
            assert!(p.user.contains("Make it demanding"));
            assert!(p.user.contains("The student also asked: \"foque na 'fóvea'\""));
            // As regras de só usar as notas vão sempre junto, em qualquer motor.
            assert!(p.system.contains("Never add facts"));
        }
        let easy = Request { level: Level::Medium, guide: "  ", ..req };
        assert!(!prompt(&easy).user.contains("student also asked"));
        assert!(chatml(&prompt(&easy)).starts_with("<|im_start|>system\nYou write"));
    }

    #[test]
    fn source_numbers_blocks_and_skips_empty_ones() {
        let blocks = vec![Block::new(BlockKind::H2, "Fases"), Block::new(BlockKind::Paragraph, ""), Block::new(BlockKind::Bullet, "Prófase")];
        let s = source("Mitose", &blocks);
        assert_eq!(s.ids, vec![blocks[0].id, blocks[2].id]);
        assert!(s.text.contains("[1] ## Fases"));
        assert!(s.text.contains("[2] - Prófase"));
    }
}
