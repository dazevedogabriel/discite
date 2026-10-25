//! Confere se um cartão sugerido se apoia nas notas: o trecho que o modelo
//! copiou ("quote") tem de existir nelas, e a resposta só pode usar palavras
//! que aparecem nelas (números, sempre). Não entende o sentido: pega fatos
//! trazidos de fora e respostas trocadas ("aumentam" onde a nota diz
//! "reduzem"), mas não uma associação errada feita só com palavras da nota.

use std::collections::HashSet;

/// Minúsculas, sem acentos, com pontuação virando espaço.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        out.push(c);
    }
    out
}

fn words(s: &str) -> Vec<String> {
    fold(s).split_whitespace().map(str::to_owned).collect()
}

/// Palavras que não dizem nada sozinhas (português e inglês, já sem acento).
const STOP: &[&str] = &[
    "that", "this", "with", "from", "have", "been", "were", "which", "what", "when", "where", "there", "their", "they", "them", "into", "than", "then", "also", "such", "does", "each",
    "other", "more", "most", "only", "some", "very", "about", "after", "before", "being", "both", "during", "while", "would", "could", "should", "will", "your", "these", "those", "because",
    "para", "como", "mais", "pelo", "pela", "pelos", "pelas", "sobre", "entre", "quando", "onde", "qual", "quais", "isso", "esta", "este", "essa", "esse", "estes", "estas", "essas", "esses",
    "sao", "sua", "seu", "suas", "seus", "tambem", "muito", "pode", "podem", "cada", "outro", "outra", "outros", "outras", "ser", "sendo", "foram", "durante", "apos", "antes", "ainda", "mesmo",
    "porque", "atraves", "meio", "sem", "nao", "tem", "tera", "pois", "assim", "depois", "dos", "das", "nos", "nas", "uma", "umas", "uns", "ele", "ela", "eles", "elas", "lhe", "seja",
];

/// Verbos e expressões de ligação que uma paráfrase correta usa sem trazer
/// fato novo ("os cones são responsáveis por…", "it occurs in…"). Palavras com
/// conteúdo ("área", "causar") ficam de fora de propósito.
const GLUE: &[&str] = &[
    "responsavel", "responsaveis", "possui", "possuem", "existe", "existem", "encontra", "encontram", "localiza", "localizada", "localizado", "localizadas", "localizados", "chamado",
    "chamada", "chamados", "chamadas", "considerado", "considerada", "significa", "refere", "ocorre", "ocorrem", "acontece", "acontecem", "envolve", "envolvem", "permite", "permitem",
    "permitir", "consiste", "composta", "composto", "formada", "formado", "occurs", "occur", "happens", "happen", "located", "called", "refers", "means", "involves", "consists",
    "composed", "responsible", "known", "takes", "place", "found",
];

/// Palavras da resposta que contam: números e palavras com 4+ letras que não
/// são de ligação.
fn content(s: &str) -> Vec<String> {
    words(s)
        .into_iter()
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()) || (w.chars().count() >= 4 && !STOP.contains(&w.as_str()) && !GLUE.contains(&w.as_str())))
        .collect()
}

/// As notas prontas para consulta.
pub struct Index {
    /// Uma linha por bloco, já normalizada (na ordem dos números `[n]`).
    lines: Vec<Vec<String>>,
    words: HashSet<String>,
    /// Começos de 5 letras, para aceitar flexões ("reduzem" × "reduz…").
    stems: HashSet<String>,
}

fn stem(w: &str) -> Option<String> {
    (w.chars().count() >= 5 && !w.chars().any(|c| c.is_ascii_digit())).then(|| w.chars().take(5).collect())
}

impl Index {
    /// `notes` é o texto mandado ao modelo (`[n] …` por linha).
    pub fn new(notes: &str) -> Index {
        let lines: Vec<Vec<String>> = notes.lines().filter(|l| l.starts_with('[')).map(|l| words(l.split_once(']').map(|(_, t)| t).unwrap_or(l))).collect();
        let all = words(notes);
        let stems = all.iter().filter_map(|w| stem(w)).collect();
        Index { lines, words: all.into_iter().collect(), stems }
    }

    fn knows(&self, w: &str) -> bool {
        if self.words.contains(w) || stem(w).is_some_and(|s| self.stems.contains(&s)) {
            return true;
        }
        // Flexões curtas nos dois sentidos: "casos" × "caso", "sobe" × "sobem".
        let chars: Vec<char> = w.chars().collect();
        if w.chars().any(|c| c.is_ascii_digit()) || chars.len() < 4 {
            return false;
        }
        (1..=2).any(|cut| chars.len() - cut >= 4 && self.words.contains(&chars[..chars.len() - cut].iter().collect::<String>()))
            || self.words.iter().any(|n| n.len() > w.len() && n.len() <= w.len() + 2 && n.starts_with(w))
    }

    /// Linha (1 = `[1]`) onde está o trecho, aceitando pequenas diferenças:
    /// pelo menos 70% dos trios de palavras seguidas do trecho na mesma linha.
    pub fn find_quote(&self, quote: &str) -> Option<usize> {
        let q = words(quote);
        if q.is_empty() {
            return None;
        }
        let mut best: Option<(usize, f32)> = None;
        for (i, line) in self.lines.iter().enumerate() {
            let score = if q.len() < 3 {
                // Trecho curto: todas as palavras, em sequência.
                let joined = format!(" {} ", line.join(" "));
                if joined.contains(&format!(" {} ", q.join(" "))) { 1.0 } else { 0.0 }
            } else {
                let have: HashSet<&[String]> = line.windows(3).collect();
                let tri: Vec<&[String]> = q.windows(3).collect();
                tri.iter().filter(|t| have.contains(*t)).count() as f32 / tri.len() as f32
            };
            if score >= 0.7 && best.is_none_or(|(_, s)| score > s) {
                best = Some((i + 1, score));
            }
        }
        best.map(|(n, _)| n)
    }

    /// Palavras de `text` que não aparecem nas notas.
    pub fn missing(&self, text: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for w in content(text) {
            if !self.knows(&w) && !out.contains(&w) {
                out.push(w);
            }
        }
        out
    }

    /// O cartão se apoia nas notas? `answer` é o verso (básico) ou o texto
    /// inteiro (lacuna). Aceita uma palavra fora da nota numa resposta longa
    /// (paráfrase: "permitir a visão…"), nunca um número.
    pub fn check(&self, quote: &str, answer: &str) -> (bool, Option<usize>, Vec<String>) {
        let line = self.find_quote(quote);
        let missing = self.missing(answer);
        let n = content(answer).len();
        let numbers = missing.iter().any(|w| w.chars().any(|c| c.is_ascii_digit()));
        let allowed = if n >= 12 { 2 } else if n >= 4 { 1 } else { 0 };
        (line.is_some() && !numbers && missing.len() <= allowed, line, missing)
    }
}

/// Idioma das notas, quando dá para dizer: "Portuguese" ou "English".
pub fn language(notes: &str) -> Option<&'static str> {
    let w = words(notes);
    let count = |list: &[&str]| w.iter().filter(|x| list.contains(&x.as_str())).count();
    let pt = count(&["de", "da", "do", "que", "nao", "uma", "com", "para", "os", "as", "em", "no", "na", "e"]);
    let en = count(&["the", "of", "and", "to", "is", "in", "with", "for", "it", "are", "on", "by"]);
    match (pt, en) {
        (p, e) if p >= 3 && p > e * 2 => Some("Portuguese"),
        (p, e) if e >= 3 && e > p * 2 => Some("English"),
        _ => None,
    }
}

/// Quantos fatos as notas têm, mais ou menos: frases (e itens de lista) com
/// pelo menos três palavras. Títulos não contam.
pub fn facts(notes: &str) -> usize {
    notes
        .lines()
        .filter_map(|l| l.split_once(']').map(|(_, t)| t.trim()))
        .filter(|t| !t.starts_with('#'))
        .flat_map(|t| t.split(['.', ';', '!', '?']))
        .filter(|s| s.split_whitespace().count() >= 3)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "Page: x\n[1] Bloqueiam a conversão de angiotensina I em angiotensina II e reduzem a degradação de bradicinina (tosse, angioedema). Contraindicados na gestação.\n[2] A fração de ejeção reduzida é ≤ 40%.";

    #[test]
    fn quote_must_come_from_the_notes() {
        let ix = Index::new(NOTES);
        assert_eq!(ix.find_quote("reduzem a degradação de bradicinina"), Some(1));
        assert_eq!(ix.find_quote("A fração de ejeção reduzida é ≤ 40%"), Some(2));
        assert_eq!(ix.find_quote("podem causar efeitos adversos ao feto"), None);
    }

    #[test]
    fn answers_with_outside_facts_are_flagged() {
        let ix = Index::new(NOTES);
        // Paráfrase com palavras da nota: ok.
        assert!(ix.check("reduzem a degradação de bradicinina", "Reduzem a degradação da bradicinina, causando tosse e angioedema.").0);
        // "aumentam" e "medicamentos" não estão na nota.
        assert!(!ix.check("reduzem a degradação de bradicinina", "Medicamentos que aumentam a degradação da bradicinina.").0);
        // Plural e verbo de ligação: ok.
        assert!(ix.check("Contraindicados na gestação", "São contraindicados nas gestações.").0);
        // Número que não está na nota.
        assert!(!ix.check("A fração de ejeção reduzida é ≤ 40%", "≤ 35%").0);
        // Trecho inventado.
        assert!(!ix.check("efeitos adversos ao feto", "Contraindicados na gestação").0);
    }

    #[test]
    fn detects_language_and_counts_facts() {
        assert_eq!(language(NOTES), Some("Portuguese"));
        assert_eq!(language("[1] The fluid mosaic model: a phospholipid bilayer with integral and peripheral proteins; cholesterol modulates fluidity."), Some("English"));
        assert_eq!(facts(NOTES), 3);
        assert_eq!(facts("[1] ## Fases\n[2] Prófase"), 0);
    }
}
