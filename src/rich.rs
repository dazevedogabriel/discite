//! Marcação inline do Discite, usada nas páginas e nos campos dos cartões:
//! `**negrito**`, `*itálico*`, `__sublinhado__`, `~~tachado~~`, `` `código` ``,
//! `==realce==`, `{red|texto em cor}`, `{=green|realce em cor}` e
//! `[[Link para página]]`. Nos campos dos cartões, uma linha
//! `![](arquivo.png)` é uma imagem.
//!
//! O editor mostra os marcadores esmaecidos (o texto editado é o mesmo que
//! fica salvo); a leitura e os cartões escondem os marcadores.

use crate::cards::{GAP_CLOSE, GAP_OPEN};

/// Cores da marcação `{cor|…}` / `{=cor|…}`. O nome salvo é o inglês, para o
/// texto não depender do idioma do app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl Ink {
    pub const ALL: [Ink; 7] = [Ink::Red, Ink::Orange, Ink::Yellow, Ink::Green, Ink::Blue, Ink::Purple, Ink::Gray];

    pub fn name(self) -> &'static str {
        match self {
            Ink::Red => "red",
            Ink::Orange => "orange",
            Ink::Yellow => "yellow",
            Ink::Green => "green",
            Ink::Blue => "blue",
            Ink::Purple => "purple",
            Ink::Gray => "gray",
        }
    }

    fn from_name(s: &str) -> Option<Ink> {
        Ink::ALL.into_iter().find(|i| i.name() == s)
    }
}

/// `{red|` ou `{=red|` no início de `rest`: (tamanho, é realce, cor).
fn color_open(rest: &str) -> Option<(usize, bool, Ink)> {
    let inner = rest.strip_prefix('{')?;
    let bar = inner.char_indices().take(9).find(|&(_, c)| c == '|')?.0;
    let name = &inner[..bar];
    let (bg, name) = match name.strip_prefix('=') {
        Some(n) => (true, n),
        None => (false, name),
    };
    Some((bar + 2, bg, Ink::from_name(name)?))
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub highlight: bool,
    /// Cor do texto e cor do realce (`{red|…}`, `{=red|…}`).
    pub ink: Option<Ink>,
    pub bg: Option<Ink>,
    pub link: bool,
    /// Lacuna de cloze (frente) ou resposta destacada (verso).
    pub gap: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Marcador de formatação (`**`, `[[`…): visível só no editor.
    pub marker: bool,
}

pub fn parse(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut st = Style::default();
    let push = |runs: &mut Vec<Run>, s: &str, style: Style, marker: bool| {
        if s.is_empty() {
            return;
        }
        if let Some(last) = runs.last_mut() {
            if last.style == style && last.marker == marker {
                last.text.push_str(s);
                return;
            }
        }
        runs.push(Run { text: s.to_owned(), style, marker });
    };
    let mut i = 0;
    let bytes = text.as_bytes();
    // Cores abertas: (é realce, cor de antes), para o `}` desfazer.
    let mut colors: Vec<(bool, Option<Ink>)> = Vec::new();
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap();
        let clen = c.len_utf8();
        if st.code {
            if c == '`' {
                push(&mut runs, "`", st, true);
                st.code = false;
            } else {
                push(&mut runs, &rest[..clen], st, false);
            }
            i += clen;
            continue;
        }
        if c == GAP_OPEN || c == GAP_CLOSE {
            st.gap = c == GAP_OPEN;
            i += clen;
            continue;
        }
        if rest.starts_with("[[") {
            if let Some(close) = rest[2..].find("]]") {
                let inner = &rest[2..2 + close];
                if !inner.contains('\n') && !inner.is_empty() {
                    push(&mut runs, "[[", st, true);
                    push(&mut runs, inner, Style { link: true, ..st }, false);
                    push(&mut runs, "]]", st, true);
                    i += 4 + close;
                    continue;
                }
            }
        }
        if let Some((len, bg, ink)) = color_open(rest) {
            let slot = if bg { &mut st.bg } else { &mut st.ink };
            colors.push((bg, slot.replace(ink)));
            push(&mut runs, &rest[..len], st, true);
            i += len;
            continue;
        }
        if c == '}' {
            if let Some((bg, prev)) = colors.pop() {
                push(&mut runs, "}", st, true);
                *(if bg { &mut st.bg } else { &mut st.ink }) = prev;
                i += 1;
                continue;
            }
        }
        let two = rest.get(..2).unwrap_or("");
        let toggled = match two {
            "**" => Some(&mut st.bold),
            "__" => Some(&mut st.underline),
            "~~" => Some(&mut st.strike),
            "==" => Some(&mut st.highlight),
            _ => None,
        };
        if let Some(flag) = toggled {
            *flag = !*flag;
            // O marcador de fechamento fica com o estilo de dentro.
            push(&mut runs, two, st, true);
            i += 2;
            continue;
        }
        if c == '`' && rest[1..].contains('`') {
            push(&mut runs, "`", st, true);
            st.code = true;
            i += 1;
            continue;
        }
        if c == '*' {
            let next = rest[1..].chars().next();
            let prev = if i > 0 { text[..i].chars().next_back() } else { None };
            let opens = !st.italic && next.is_some_and(|n| !n.is_whitespace()) && rest[1..].contains('*');
            let closes = st.italic && prev.is_some_and(|p| !p.is_whitespace());
            if opens || closes {
                st.italic = !st.italic;
                push(&mut runs, "*", st, true);
                i += 1;
                continue;
            }
        }
        let _ = bytes;
        push(&mut runs, &rest[..clen], st, false);
        i += clen;
    }
    runs
}

/// Texto sem marcação (busca, títulos, prévias).
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    for seg in segments(text) {
        if let Seg::Text(t) = seg {
            for r in parse(&t) {
                if !r.marker {
                    out.push_str(&r.text);
                }
            }
            out.push('\n');
        }
    }
    out.trim_end().replace(['\u{E000}', '\u{E001}'], "")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Text(String),
    Image(String),
}

/// Separa as linhas `![](arquivo)` do texto em volta.
pub fn segments(text: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in text.split('\n') {
        if let Some(name) = image_line(line) {
            if !buf.is_empty() {
                out.push(Seg::Text(std::mem::take(&mut buf).trim_end_matches('\n').to_owned()));
            }
            out.push(Seg::Image(name));
        } else {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
    }
    if !buf.trim().is_empty() {
        out.push(Seg::Text(buf));
    }
    out
}

pub fn image_line(line: &str) -> Option<String> {
    let t = line.trim();
    let inner = t.strip_prefix("![")?.strip_suffix(')')?;
    let (_, name) = inner.split_once("](")?;
    (!name.is_empty()).then(|| name.to_owned())
}

pub fn image_markup(name: &str) -> String {
    format!("![]({name})")
}

/// Alvos de `[[links]]`.
pub fn links(text: &str) -> Vec<String> {
    parse(text).into_iter().filter(|r| r.style.link && !r.marker).map(|r| r.text).collect()
}

/// Liga/desliga um marcador em volta da seleção (bytes `[a, b)`), como os
/// botões B/I/U da barra. Devolve o novo texto e a nova seleção.
pub fn toggle_wrap(text: &str, a: usize, b: usize, marker: &str) -> (String, usize, usize) {
    let (a, b) = (a.min(b), a.max(b));
    let m = marker.len();
    if a == b {
        // Sem seleção: par vazio "****" some; cursor antes do fechamento pula
        // para depois dele (sai do negrito); senão abre um par e fica dentro.
        if a >= m && text[..a].ends_with(marker) && text[a..].starts_with(marker) {
            let s = format!("{}{}", &text[..a - m], &text[a + m..]);
            return (s, a - m, a - m);
        }
        if text[a..].starts_with(marker) {
            return (text.to_owned(), a + m, a + m);
        }
        let s = format!("{}{marker}{marker}{}", &text[..a], &text[a..]);
        return (s, a + m, a + m);
    }
    // Já envolvido por fora? Remove.
    if a >= m && text[..a].ends_with(marker) && text[b..].starts_with(marker) {
        let mut s = String::with_capacity(text.len());
        s.push_str(&text[..a - m]);
        s.push_str(&text[a..b]);
        s.push_str(&text[b + m..]);
        return (s, a - m, b - m);
    }
    // Seleção inclui os marcadores? Remove.
    let sel = &text[a..b];
    if sel.len() >= 2 * m && sel.starts_with(marker) && sel.ends_with(marker) {
        let mut s = String::with_capacity(text.len());
        s.push_str(&text[..a]);
        s.push_str(&sel[m..sel.len() - m]);
        s.push_str(&text[b..]);
        return (s, a, b - 2 * m);
    }
    let mut s = String::with_capacity(text.len() + 2 * m);
    s.push_str(&text[..a]);
    s.push_str(marker);
    s.push_str(sel);
    s.push_str(marker);
    s.push_str(&text[b..]);
    (s, a + m, b + m)
}

/// Um trecho `{cor|…}` fechado, em bytes: abertura `[open, start)`, conteúdo
/// `[start, end)` e o `}` em `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorSpan {
    pub open: usize,
    pub start: usize,
    pub end: usize,
    pub bg: bool,
    pub ink: Ink,
}

/// Trechos coloridos do texto, com as mesmas regras do `parse` (nada dentro
/// de `código`).
pub fn color_spans(text: &str) -> Vec<ColorSpan> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize, bool, Ink)> = Vec::new();
    let mut code = false;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap();
        if code {
            code = c != '`';
        } else if c == '`' && rest[1..].contains('`') {
            code = true;
        } else if let Some((len, bg, ink)) = color_open(rest) {
            stack.push((i, i + len, bg, ink));
            i += len;
            continue;
        } else if c == '}' {
            if let Some((open, start, bg, ink)) = stack.pop() {
                out.push(ColorSpan { open, start, end: i, bg, ink });
            }
        }
        i += c.len_utf8();
    }
    out
}

/// Aplica (ou tira, com `None`) uma cor de texto ou de realce na seleção
/// (bytes `[a, b)`). Escolher a cor que o trecho já tem tira a cor; outra
/// cor troca. Devolve o novo texto e a nova seleção.
pub fn set_color(text: &str, a: usize, b: usize, bg: bool, ink: Option<Ink>) -> (String, usize, usize) {
    let (a, b) = (a.min(b), a.max(b));
    // O trecho mais interno do mesmo tipo que contém a seleção (ou que ela cobre inteiro).
    let hit = color_spans(text)
        .into_iter()
        .filter(|s| s.bg == bg && ((a >= s.start && b <= s.end) || (a <= s.open && b > s.end)))
        .min_by_key(|s| s.end - s.open);
    if let Some(s) = hit {
        // Seleção parcial com outra cor: vira um trecho novo dentro deste.
        let whole = a == b || (a <= s.start && b >= s.end);
        if ink.is_none() || ink == Some(s.ink) || whole {
            let inner = &text[s.start..s.end];
            let open = match ink {
                Some(k) if k != s.ink => format!("{{{}{}|", if bg { "=" } else { "" }, k.name()),
                _ => String::new(),
            };
            let close = if open.is_empty() { "" } else { "}" };
            let out = format!("{}{open}{inner}{close}{}", &text[..s.open], &text[s.end + 1..]);
            let old_len = s.end + 1 - s.open;
            let new_len = open.len() + inner.len() + close.len();
            let fix = |x: usize| -> usize {
                if x <= s.open {
                    x
                } else if x > s.end {
                    x + new_len - old_len
                } else {
                    s.open + open.len() + (x.max(s.start) - s.start)
                }
            };
            return (out, fix(a), fix(b));
        }
    }
    let Some(ink) = ink else {
        // "Sem realce" também tira o `==…==` em volta da seleção.
        if bg && a >= 2 && text[..a].ends_with("==") && text[b..].starts_with("==") {
            return toggle_wrap(text, a, b, "==");
        }
        return (text.to_owned(), a, b);
    };
    let open = format!("{{{}{}|", if bg { "=" } else { "" }, ink.name());
    let out = format!("{}{open}{}}}{}", &text[..a], &text[a..b], &text[b..]);
    (out, a + open.len(), b + open.len())
}

/// Índice de byte do caractere `ci`.
pub fn byte_of_char(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(text.len())
}

pub fn char_of_byte(text: &str, bi: usize) -> usize {
    text[..bi.min(text.len())].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marks() {
        let r = parse("a **b** *c* `d*e` [[Meiose]]");
        let visible: String = r.iter().filter(|r| !r.marker).map(|r| r.text.as_str()).collect();
        assert_eq!(visible, "a b c d*e Meiose");
        assert!(r.iter().any(|r| r.text == "b" && r.style.bold));
        assert!(r.iter().any(|r| r.text == "c" && r.style.italic));
        assert!(r.iter().any(|r| r.text == "d*e" && r.style.code));
        assert_eq!(links("ver [[Meiose]] e [[Apoptose]]"), vec!["Meiose", "Apoptose"]);
        assert_eq!(plain("5 * 3 = 15"), "5 * 3 = 15");
    }

    #[test]
    fn wrap_and_unwrap() {
        let (s, a, b) = toggle_wrap("ola mundo", 4, 9, "**");
        assert_eq!(s, "ola **mundo**");
        let (s2, _, _) = toggle_wrap(&s, a, b, "**");
        assert_eq!(s2, "ola mundo");
    }

    #[test]
    fn wrap_without_selection_opens_then_exits() {
        // Ctrl+B, digita, Ctrl+B: o segundo atalho só sai do negrito.
        let (s, a, _) = toggle_wrap("ola ", 4, 4, "**");
        assert_eq!((s.as_str(), a), ("ola ****", 6));
        let typed = format!("{}neg{}", &s[..a], &s[a..]);
        let (s2, a2, _) = toggle_wrap(&typed, a + 3, a + 3, "**");
        assert_eq!((s2.as_str(), a2), ("ola **neg**", 11));
        // Par vazio desfaz.
        let (s3, a3, _) = toggle_wrap("ola ****", 6, 6, "**");
        assert_eq!((s3.as_str(), a3), ("ola ", 4));
    }

    #[test]
    fn colors() {
        let r = parse("a {red|b {=green|c}} d");
        let visible: String = r.iter().filter(|r| !r.marker).map(|r| r.text.as_str()).collect();
        assert_eq!(visible, "a b c d");
        assert!(r.iter().any(|r| r.text == "b " && r.style.ink == Some(Ink::Red) && r.style.bg.is_none()));
        assert!(r.iter().any(|r| r.text == "c" && r.style.ink == Some(Ink::Red) && r.style.bg == Some(Ink::Green)));
        assert!(r.iter().any(|r| r.text == " d" && r.style == Style::default()));
        // Chaves comuns e nomes desconhecidos ficam como texto.
        assert_eq!(plain("f(x) = {x} {pink|y}"), "f(x) = {x} {pink|y}");
        assert_eq!(plain("`{red|x}`"), "{red|x}");
    }

    #[test]
    fn color_apply_change_remove() {
        let (s, a, b) = set_color("ola mundo", 4, 9, false, Some(Ink::Red));
        assert_eq!((s.as_str(), &s[a..b]), ("ola {red|mundo}", "mundo"));
        let (s2, a2, b2) = set_color(&s, a, b, false, Some(Ink::Blue));
        assert_eq!((s2.as_str(), &s2[a2..b2]), ("ola {blue|mundo}", "mundo"));
        let (s3, a3, b3) = set_color(&s2, a2, b2, false, Some(Ink::Blue));
        assert_eq!((s3.as_str(), &s3[a3..b3]), ("ola mundo", "mundo"));
        // Cursor dentro do trecho e "sem cor".
        let (s4, _, _) = set_color("x {=green|abc} y", 11, 11, true, None);
        assert_eq!(s4, "x abc y");
        // Realce e cor de texto não se misturam.
        let (s5, _, _) = set_color("{red|abc}", 5, 8, true, Some(Ink::Yellow));
        assert_eq!(s5, "{red|{=yellow|abc}}");
        // "Sem realce" também tira o ==…==.
        let (s6, _, _) = set_color("a ==b== c", 4, 5, true, None);
        assert_eq!(s6, "a b c");
        // Sem seleção: abre um par vazio com o cursor dentro.
        let (s7, a7, _) = set_color("ab", 1, 1, false, Some(Ink::Gray));
        assert_eq!((s7.as_str(), a7), ("a{gray|}b", 7));
    }

    #[test]
    fn image_segments() {
        let s = segments("antes\n![](abc.png)\ndepois");
        assert_eq!(s, vec![Seg::Text("antes".into()), Seg::Image("abc.png".into()), Seg::Text("depois".into())]);
    }
}
