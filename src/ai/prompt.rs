//! O que vai para o modelo (instruções + notas numeradas por bloco) e o que
//! volta: uma gramática que só aceita o JSON dos cartões e um leitor que
//! entrega cada cartão assim que o objeto dele fecha.

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

/// Quantos cartões pedir de fato: no máximo dois por fato das notas, para o
/// modelo não "encher" uma página de uma frase com o que ele sabe de fora.
pub fn cap(requested: u32, source: &Source) -> u32 {
    requested.min((super::ground::facts(&source.text) as u32 * 2).max(2))
}

pub struct Request<'a> {
    pub source: &'a Source,
    /// Perguntas/lacunas dos cartões que a página já tem.
    pub existing: &'a [String],
    pub count: u32,
    pub style: Style,
    /// Trecho escolhido pelo usuário dentro das notas (seleção ou bloco).
    pub focus: Option<&'a str>,
}

/// Prompt no formato ChatML (o dos modelos Qwen), com o raciocínio
/// ("thinking") desligado: a resposta começa direto no JSON.
pub fn prompt(r: &Request) -> String {
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
    user.push_str(&format!("\nWrite {} new flashcards; write fewer only if the notes run out of distinct facts. {kind}", r.count));
    // Dito com todas as letras: o modelo pequeno respondia em inglês a notas em português.
    if let Some(lang) = super::ground::language(&r.source.text) {
        user.push_str(&format!(" The notes are in {lang}: write every card in {lang}."));
    }
    format!("<|im_start|>system\n{SYSTEM}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n")
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
    fn from_raw(r: Raw, ids: &[i64], notes: &Index) -> Option<Suggestion> {
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

/// Tira o marcador de lista ou título que o modelo às vezes copia das notas
/// ("1. Prófase — …", "- Cones: …", "### …").
fn unlist(s: &str) -> String {
    let t = s.trim();
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
}

impl Stream {
    /// `notes` é o texto mandado ao modelo (`Source::text`); `ids`, os blocos.
    pub fn new(ids: Vec<i64>, notes: &str) -> Stream {
        Stream { buf: String::new(), pos: 0, depth: 0, in_str: false, esc: false, start: None, ids, notes: Index::new(notes) }
    }

    pub fn feed(&mut self, piece: &str) -> Vec<Suggestion> {
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
                                if let Ok(raw) = serde_json::from_str::<Raw>(&self.buf[s..=self.pos]) {
                                    out.extend(Suggestion::from_raw(raw, &self.ids, &self.notes));
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
    fn similar_catches_rewordings_only() {
        assert!(similar("Qual a função dos bastonetes?", "Qual é a função dos bastonetes?"));
        assert!(!similar("Qual a função dos bastonetes?", "Qual a função dos cones?"));
    }

    #[test]
    fn stream_yields_each_card_checked_against_the_notes() {
        let notes = "Page: Retina\n[1] A retina é sensorial.\n[2] A fóvea é a região \"central\" da mácula.\n";
        let mut s = Stream::new(vec![10, 20], notes);
        // O primeiro diz bloco 1, mas o trecho está no 2; o terceiro inventa um número.
        let json = r#"{"cards": [{"type": "basic", "block": 1, "quote": "A fóvea é a região \"central\" da mácula", "front": "O que é a {fóvea}?", "back": "Região \"central\" da mácula"},
            {"type": "cloze", "block": 1, "quote": "A retina é sensorial", "text": "A {{c1::retina}} é sensorial."},
            {"type": "basic", "block": 1, "quote": "A retina tem 120 milhões de bastonetes", "front": "Quantos bastonetes?", "back": "120 milhões"}]}"#;
        let mut got = vec![];
        for ch in json.chars() {
            got.extend(s.feed(&ch.to_string()));
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
    fn source_numbers_blocks_and_skips_empty_ones() {
        let blocks = vec![Block::new(BlockKind::H2, "Fases"), Block::new(BlockKind::Paragraph, ""), Block::new(BlockKind::Bullet, "Prófase")];
        let s = source("Mitose", &blocks);
        assert_eq!(s.ids, vec![blocks[0].id, blocks[2].id]);
        assert!(s.text.contains("[1] ## Fases"));
        assert!(s.text.contains("[2] - Prófase"));
    }
}
