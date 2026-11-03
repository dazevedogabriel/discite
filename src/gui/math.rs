//! Fórmulas LaTeX desenhadas com o painter do egui: o LaTeX comum em
//! anotações (frações, raízes, índices, letras gregas, símbolos, `\text`,
//! `\left…\right`, matrizes, `cases` e `aligned`). O que não conhece aparece
//! como o próprio comando, sem quebrar o resto.

use super::theme::{W, serif};
use super::widgets::a11y;
use eframe::egui::{self, Color32, Galley, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Font {
    Italic,
    Roman,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Acc {
    Hat,
    Bar,
    Arrow,
    Dot,
    DDot,
    Tilde,
    Under,
    Boxed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Env {
    /// Linhas separadas por `\\`, centralizadas.
    Lines,
    Matrix,
    Cases,
    Aligned,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Sym(String, Font),
    /// Operador ou relação: ganha espaço dos dois lados quando está entre dois termos.
    Op(String),
    Punct(String),
    /// sin, log, lim…; `true`: índices embaixo e em cima (lim, max).
    Func(String, bool),
    /// ∑, ∫…; `true`: índices embaixo e em cima.
    Big(String, bool),
    Row(Vec<Node>),
    /// `false`: sem traço (`\binom`).
    Frac(Box<Node>, Box<Node>, bool),
    Sqrt(Option<Box<Node>>, Box<Node>),
    Scripts { base: Box<Node>, sup: Option<Box<Node>>, sub: Option<Box<Node>> },
    Delim(String, Box<Node>, String),
    Accent(Acc, Box<Node>),
    /// Em "em" (1.0 = o tamanho da fonte).
    Space(f32),
    Grid(Vec<Vec<Node>>, Env),
}

// ───────────────────────────── Leitura ─────────────────────────────

fn parse(src: &str) -> Node {
    let mut p = Parser { s: src.chars().collect(), i: 0 };
    p.grid(Env::Lines, true)
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    /// O comando em `\nome` (ou `\x` de um caractere) e onde ele termina.
    fn cmd_at(&self) -> Option<(String, usize)> {
        if self.peek() != Some('\\') {
            return None;
        }
        let start = self.i + 1;
        let mut j = start;
        while self.s.get(j).is_some_and(|c| c.is_ascii_alphabetic()) {
            j += 1;
        }
        if j == start {
            return self.s.get(j).map(|c| (c.to_string(), j + 1));
        }
        Some((self.s[start..j].iter().collect(), j))
    }

    /// Onde uma sequência termina: fim de grupo, de célula, de linha ou de `\left`.
    fn stop(&self) -> bool {
        match self.peek() {
            None | Some('}') | Some('&') => true,
            Some('\\') => matches!(self.cmd_at().map(|c| c.0).as_deref(), Some("\\" | "cr" | "end" | "right")),
            _ => false,
        }
    }

    /// O conteúdo de `{…}` como texto (para `\text` e nomes de ambiente).
    fn raw_group(&mut self) -> String {
        self.skip_ws();
        if self.peek() != Some('{') {
            return self
                .peek()
                .map(|c| {
                    self.i += 1;
                    c.to_string()
                })
                .unwrap_or_default();
        }
        self.i += 1;
        let mut depth = 0;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                '\\' => {
                    if let Some(n) = self.peek().filter(|n| "{}$%&#_ ".contains(*n)) {
                        self.i += 1;
                        out.push(n);
                        continue;
                    }
                }
                _ => {}
            }
            out.push(c);
        }
        out
    }

    /// Argumento de um comando ou índice: `{…}` ou um único elemento.
    fn arg(&mut self) -> Node {
        self.skip_ws();
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                self.i += 1;
                Node::Sym(c.to_string(), Font::Roman)
            }
            _ if self.stop() => Node::Row(vec![]),
            _ => self.atom().unwrap_or(Node::Row(vec![])),
        }
    }

    fn row(&mut self) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        loop {
            self.skip_ws();
            if self.stop() {
                break;
            }
            match self.peek() {
                Some(c @ ('^' | '_')) => {
                    self.i += 1;
                    let a = self.arg();
                    let base = out.pop().unwrap_or(Node::Row(vec![]));
                    out.push(attach(base, c == '^', a));
                }
                Some('\'') => {
                    self.i += 1;
                    let base = out.pop().unwrap_or(Node::Row(vec![]));
                    out.push(attach(base, true, Node::Sym("′".into(), Font::Roman)));
                }
                _ => out.extend(self.atom()),
            }
        }
        out
    }

    /// Um elemento. Sempre avança pelo menos um caractere.
    fn atom(&mut self) -> Option<Node> {
        let c = self.peek()?;
        if c == '\\' {
            return self.command();
        }
        self.i += 1;
        if c == '{' {
            let r = self.row();
            if self.peek() == Some('}') {
                self.i += 1;
            }
            return Some(Node::Row(r));
        }
        if c.is_ascii_digit() {
            let mut s = c.to_string();
            // "2,5" e "2.5" ficam juntos (vírgula decimal).
            while let Some(d) = self.peek() {
                let next_digit = self.s.get(self.i + 1).is_some_and(char::is_ascii_digit);
                if d.is_ascii_digit() || ((d == '.' || d == ',') && next_digit) {
                    s.push(d);
                    self.i += 1;
                } else {
                    break;
                }
            }
            return Some(Node::Sym(s, Font::Roman));
        }
        Some(match c {
            '$' => return None,
            '-' | '−' => Node::Op("−".into()),
            '*' => Node::Op("∗".into()),
            '+' | '=' | '<' | '>' | '×' | '÷' | '±' | '∓' | '·' | '≤' | '≥' | '≠' | '≈' | '≡' | '→' | '⇒' | '⇔' | '∈' => Node::Op(c.to_string()),
            ',' | ';' => Node::Punct(c.to_string()),
            '~' => Node::Space(0.33),
            c if c.is_alphabetic() => Node::Sym(c.to_string(), Font::Italic),
            c => Node::Sym(c.to_string(), Font::Roman),
        })
    }

    fn command(&mut self) -> Option<Node> {
        let Some((name, j)) = self.cmd_at() else {
            self.i += 1;
            return None;
        };
        self.i = j;
        let n = name.as_str();
        let sym = |s: &str| Some(Node::Sym(s.into(), Font::Roman));
        let space = |em: f32| Some(Node::Space(em));
        match n {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg();
                let b = self.arg();
                Some(Node::Frac(Box::new(a), Box::new(b), true))
            }
            "binom" | "dbinom" | "tbinom" => {
                let a = self.arg();
                let b = self.arg();
                Some(Node::Delim("(".into(), Box::new(Node::Frac(Box::new(a), Box::new(b), false)), ")".into()))
            }
            "sqrt" => {
                self.skip_ws();
                let index = if self.peek() == Some('[') {
                    self.i += 1;
                    let mut s = String::new();
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == ']' {
                            break;
                        }
                        s.push(c);
                    }
                    Some(Box::new(Node::Row(Parser { s: s.chars().collect(), i: 0 }.row())))
                } else {
                    None
                };
                let body = self.arg();
                Some(Node::Sqrt(index, Box::new(body)))
            }
            "text" | "textrm" | "textnormal" | "mbox" | "hbox" | "textup" => Some(Node::Sym(self.raw_group(), Font::Roman)),
            "textit" | "emph" => Some(Node::Sym(self.raw_group(), Font::Italic)),
            "textbf" => Some(Node::Sym(self.raw_group(), Font::Bold)),
            "mathrm" | "operatorname" | "mathsf" | "mathtt" | "mathcal" | "mathscr" | "mathfrak" => Some(restyle(self.arg(), Font::Roman)),
            "mathbf" | "boldsymbol" | "bm" | "pmb" => Some(restyle(self.arg(), Font::Bold)),
            "mathit" => Some(restyle(self.arg(), Font::Italic)),
            "mathbb" => Some(board(self.arg())),
            "left" => {
                let l = self.delim();
                let body = self.row();
                let r = if self.cmd_at().is_some_and(|c| c.0 == "right") {
                    self.i = self.cmd_at().unwrap().1;
                    self.delim()
                } else {
                    String::new()
                };
                Some(Node::Delim(l, Box::new(Node::Row(body)), r))
            }
            "begin" => Some(self.env()),
            "color" | "label" | "tag" => {
                self.raw_group();
                None
            }
            "textcolor" => {
                self.raw_group();
                Some(self.arg())
            }
            "not" => {
                self.skip_ws();
                if self.peek() == Some('=') {
                    self.i += 1;
                    return Some(Node::Op("≠".into()));
                }
                match self.cmd_at().map(|c| c.0).as_deref() {
                    Some("in") => {
                        self.i = self.cmd_at().unwrap().1;
                        Some(Node::Op("∉".into()))
                    }
                    Some("subset") => {
                        self.i = self.cmd_at().unwrap().1;
                        Some(Node::Op("⊄".into()))
                    }
                    _ => None,
                }
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" | "middle" | "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "nonumber" | "notag" => None,
            "," | "thinspace" => space(0.17),
            ":" | ">" | "medspace" => space(0.22),
            ";" | "thickspace" => space(0.28),
            "!" => space(-0.17),
            " " => space(0.33),
            "enspace" => space(0.5),
            "quad" => space(1.0),
            "qquad" => space(2.0),
            "{" | "}" | "%" | "$" | "#" | "&" | "_" => sym(n),
            "|" => sym("‖"),
            "hat" | "widehat" => Some(Node::Accent(Acc::Hat, Box::new(self.arg()))),
            "bar" | "overline" => Some(Node::Accent(Acc::Bar, Box::new(self.arg()))),
            "vec" | "overrightarrow" => Some(Node::Accent(Acc::Arrow, Box::new(self.arg()))),
            "dot" => Some(Node::Accent(Acc::Dot, Box::new(self.arg()))),
            "ddot" => Some(Node::Accent(Acc::DDot, Box::new(self.arg()))),
            "tilde" | "widetilde" => Some(Node::Accent(Acc::Tilde, Box::new(self.arg()))),
            "underline" => Some(Node::Accent(Acc::Under, Box::new(self.arg()))),
            "boxed" | "fbox" => Some(Node::Accent(Acc::Boxed, Box::new(self.arg()))),
            _ => {
                if let Some(f) = FUNCS.iter().find(|f| **f == n) {
                    return Some(Node::Func((*f).into(), matches!(n, "lim" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "Pr" | "liminf" | "limsup" | "argmax" | "argmin")));
                }
                if let Some((_, s, limits)) = BIG.iter().find(|b| b.0 == n) {
                    return Some(Node::Big((*s).into(), *limits));
                }
                if let Some((_, s)) = GREEK.iter().find(|g| g.0 == n) {
                    let upper = s.chars().next().is_some_and(char::is_uppercase);
                    return Some(Node::Sym((*s).into(), if upper { Font::Roman } else { Font::Italic }));
                }
                if let Some((_, s, op)) = SYMBOLS.iter().find(|x| x.0 == n) {
                    return Some(if *op { Node::Op((*s).into()) } else { Node::Sym((*s).into(), Font::Roman) });
                }
                Some(Node::Sym(format!("\\{n}"), Font::Roman))
            }
        }
    }

    fn delim(&mut self) -> String {
        self.skip_ws();
        if let Some((n, j)) = self.cmd_at() {
            self.i = j;
            return match n.as_str() {
                "{" | "lbrace" => "{",
                "}" | "rbrace" => "}",
                "langle" => "⟨",
                "rangle" => "⟩",
                "|" | "Vert" | "lVert" | "rVert" => "‖",
                "vert" | "lvert" | "rvert" | "mid" => "|",
                "lfloor" => "⌊",
                "rfloor" => "⌋",
                "lceil" => "⌈",
                "rceil" => "⌉",
                "lbrack" => "[",
                "rbrack" => "]",
                _ => "",
            }
            .to_owned();
        }
        match self.peek() {
            Some('.') => {
                self.i += 1;
                String::new()
            }
            Some(c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    fn env(&mut self) -> Node {
        let name = self.raw_group();
        let name = name.trim();
        if name == "array" || name.starts_with("alignat") {
            self.raw_group();
        }
        let (kind, l, r) = match name {
            "matrix" | "smallmatrix" | "array" => (Env::Matrix, "", ""),
            "pmatrix" => (Env::Matrix, "(", ")"),
            "bmatrix" => (Env::Matrix, "[", "]"),
            "Bmatrix" => (Env::Matrix, "{", "}"),
            "vmatrix" => (Env::Matrix, "|", "|"),
            "Vmatrix" => (Env::Matrix, "‖", "‖"),
            "cases" | "dcases" => (Env::Cases, "{", ""),
            n if n.starts_with("align") || n.starts_with("split") || n.starts_with("eqnarray") => (Env::Aligned, "", ""),
            _ => (Env::Lines, "", ""),
        };
        let g = self.grid(kind, false);
        if l.is_empty() && r.is_empty() { g } else { Node::Delim(l.into(), Box::new(g), r.into()) }
    }

    /// Células separadas por `&` e linhas por `\\`, até `\end` (ou o fim, em `top`).
    fn grid(&mut self, env: Env, top: bool) -> Node {
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut cells: Vec<Node> = Vec::new();
        let mut cur: Vec<Node> = Vec::new();
        loop {
            cur.extend(self.row());
            match self.peek() {
                None => break,
                Some('&') => {
                    self.i += 1;
                    cells.push(Node::Row(std::mem::take(&mut cur)));
                }
                Some('\\') => {
                    let (name, j) = self.cmd_at().unwrap_or_default();
                    self.i = j.max(self.i + 1);
                    match name.as_str() {
                        "\\" | "cr" => {
                            // `\\[2pt]`: o espaço extra é ignorado.
                            self.skip_ws();
                            if self.peek() == Some('[') {
                                while let Some(c) = self.peek() {
                                    self.i += 1;
                                    if c == ']' {
                                        break;
                                    }
                                }
                            }
                            cells.push(Node::Row(std::mem::take(&mut cur)));
                            rows.push(std::mem::take(&mut cells));
                        }
                        "end" => {
                            self.raw_group();
                            if !top {
                                break;
                            }
                        }
                        "right" => {
                            self.delim();
                        }
                        _ => {}
                    }
                }
                // Chave sobrando: ignora e segue na mesma célula.
                Some(_) => self.i += 1,
            }
        }
        cells.push(Node::Row(cur));
        // Uma linha final vazia (depois de um `\\` no fim) não conta.
        if !(cells.len() == 1 && cells[0] == Node::Row(vec![]) && !rows.is_empty()) {
            rows.push(cells);
        }
        Node::Grid(rows, env)
    }
}

/// Põe `a` como expoente (`sup`) ou índice de `base`.
fn attach(base: Node, sup: bool, a: Node) -> Node {
    match base {
        Node::Scripts { base, sup: s, sub } if (sup && s.is_none()) || (!sup && sub.is_none()) => {
            if sup {
                Node::Scripts { base, sup: Some(Box::new(a)), sub }
            } else {
                Node::Scripts { base, sup: s, sub: Some(Box::new(a)) }
            }
        }
        base => {
            let a = Some(Box::new(a));
            if sup {
                Node::Scripts { base: Box::new(base), sup: a, sub: None }
            } else {
                Node::Scripts { base: Box::new(base), sup: None, sub: a }
            }
        }
    }
}

fn restyle(n: Node, f: Font) -> Node {
    match n {
        Node::Sym(s, _) => Node::Sym(s, f),
        Node::Row(v) => Node::Row(v.into_iter().map(|n| restyle(n, f)).collect()),
        Node::Scripts { base, sup, sub } => Node::Scripts { base: Box::new(restyle(*base, f)), sup, sub },
        n => n,
    }
}

/// `\mathbb{R}` → ℝ.
fn board(n: Node) -> Node {
    match n {
        Node::Sym(s, _) => Node::Sym(
            s.chars()
                .map(|c| match c {
                    'R' => 'ℝ',
                    'N' => 'ℕ',
                    'Z' => 'ℤ',
                    'Q' => 'ℚ',
                    'C' => 'ℂ',
                    'P' => 'ℙ',
                    'H' => 'ℍ',
                    c => c,
                })
                .collect(),
            Font::Roman,
        ),
        Node::Row(v) => Node::Row(v.into_iter().map(board).collect()),
        n => n,
    }
}

const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log", "ln", "lg", "exp", "lim", "liminf", "limsup", "max", "min", "sup", "inf", "det", "dim", "ker", "deg", "arg", "gcd", "Pr", "argmax", "argmin",
];

const BIG: &[(&str, &str, bool)] = &[
    ("sum", "∑", true),
    ("prod", "∏", true),
    ("coprod", "∐", true),
    ("bigcup", "⋃", true),
    ("bigcap", "⋂", true),
    ("bigoplus", "⊕", true),
    ("bigotimes", "⊗", true),
    ("bigvee", "⋁", true),
    ("bigwedge", "⋀", true),
    ("int", "∫", false),
    ("iint", "∬", false),
    ("iiint", "∭", false),
    ("oint", "∮", false),
];

const GREEK: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ϵ"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("pi", "π"),
    ("varpi", "ϖ"),
    ("rho", "ρ"),
    ("varrho", "ϱ"),
    ("sigma", "σ"),
    ("varsigma", "ς"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "ϕ"),
    ("varphi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
];

/// Comando, símbolo e se é operador/relação (espaçado).
const SYMBOLS: &[(&str, &str, bool)] = &[
    ("times", "×", true),
    ("cdot", "·", true),
    ("div", "÷", true),
    ("pm", "±", true),
    ("mp", "∓", true),
    ("ast", "∗", true),
    ("star", "⋆", true),
    ("circ", "∘", true),
    ("bullet", "•", true),
    ("oplus", "⊕", true),
    ("otimes", "⊗", true),
    ("cup", "∪", true),
    ("cap", "∩", true),
    ("land", "∧", true),
    ("wedge", "∧", true),
    ("lor", "∨", true),
    ("vee", "∨", true),
    ("setminus", "∖", true),
    ("leq", "≤", true),
    ("le", "≤", true),
    ("geq", "≥", true),
    ("ge", "≥", true),
    ("leqslant", "⩽", true),
    ("geqslant", "⩾", true),
    ("neq", "≠", true),
    ("ne", "≠", true),
    ("approx", "≈", true),
    ("equiv", "≡", true),
    ("sim", "∼", true),
    ("simeq", "≃", true),
    ("cong", "≅", true),
    ("propto", "∝", true),
    ("ll", "≪", true),
    ("gg", "≫", true),
    ("in", "∈", true),
    ("notin", "∉", true),
    ("ni", "∋", true),
    ("subset", "⊂", true),
    ("subseteq", "⊆", true),
    ("supset", "⊃", true),
    ("supseteq", "⊇", true),
    ("perp", "⊥", true),
    ("parallel", "∥", true),
    ("mid", "|", true),
    ("to", "→", true),
    ("rightarrow", "→", true),
    ("leftarrow", "←", true),
    ("gets", "←", true),
    ("leftrightarrow", "↔", true),
    ("Rightarrow", "⇒", true),
    ("Leftarrow", "⇐", true),
    ("Leftrightarrow", "⇔", true),
    ("implies", "⟹", true),
    ("impliedby", "⟸", true),
    ("iff", "⟺", true),
    ("longrightarrow", "⟶", true),
    ("longleftarrow", "⟵", true),
    ("mapsto", "↦", true),
    ("uparrow", "↑", true),
    ("downarrow", "↓", true),
    ("rightleftharpoons", "⇌", true),
    ("infty", "∞", false),
    ("partial", "∂", false),
    ("nabla", "∇", false),
    ("forall", "∀", false),
    ("exists", "∃", false),
    ("nexists", "∄", false),
    ("emptyset", "∅", false),
    ("varnothing", "∅", false),
    ("neg", "¬", false),
    ("lnot", "¬", false),
    ("angle", "∠", false),
    ("triangle", "△", false),
    ("square", "□", false),
    ("degree", "°", false),
    ("prime", "′", false),
    ("ldots", "…", false),
    ("dots", "…", false),
    ("cdots", "⋯", false),
    ("vdots", "⋮", false),
    ("ddots", "⋱", false),
    ("therefore", "∴", false),
    ("because", "∵", false),
    ("hbar", "ℏ", false),
    ("ell", "ℓ", false),
    ("Re", "ℜ", false),
    ("Im", "ℑ", false),
    ("aleph", "ℵ", false),
    ("langle", "⟨", false),
    ("rangle", "⟩", false),
    ("lfloor", "⌊", false),
    ("rfloor", "⌋", false),
    ("lceil", "⌈", false),
    ("rceil", "⌉", false),
    ("lbrace", "{", false),
    ("rbrace", "}", false),
    ("vert", "|", false),
    ("Vert", "‖", false),
    ("checkmark", "✓", false),
    ("dagger", "†", false),
];

// ───────────────────────────── Diagramação ─────────────────────────────

/// Uma fórmula montada: origem na linha de base, à esquerda; `asc` sobe, `desc` desce.
struct Lay {
    w: f32,
    asc: f32,
    desc: f32,
    items: Vec<(Vec2, Item)>,
}

enum Item {
    /// Texto e a distância do topo do galley até a linha de base.
    Text(Arc<Galley>, f32),
    Path(Vec<Vec2>, f32),
    Frame(Vec2, f32),
    Dot(f32),
}

impl Lay {
    fn new(w: f32, asc: f32, desc: f32) -> Lay {
        Lay { w, asc, desc, items: Vec::new() }
    }

    fn place(&mut self, l: Lay, dx: f32, dy: f32) {
        self.asc = self.asc.max(l.asc - dy);
        self.desc = self.desc.max(l.desc + dy);
        self.items.extend(l.items.into_iter().map(|(p, it)| (p + vec2(dx, dy), it)));
    }

    fn path(&mut self, pts: Vec<Vec2>, t: f32) {
        self.items.push((Vec2::ZERO, Item::Path(pts, t)));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Ord,
    Op,
    Punct,
    Open,
    Func,
}

fn class(n: &Node) -> Class {
    match n {
        Node::Op(_) => Class::Op,
        Node::Punct(_) => Class::Punct,
        Node::Sym(s, _) if matches!(s.as_str(), "(" | "[" | "{" | "⟨") => Class::Open,
        Node::Func(..) | Node::Big(..) => Class::Func,
        Node::Scripts { base, .. } => match class(base) {
            Class::Func => Class::Func,
            _ => Class::Ord,
        },
        _ => Class::Ord,
    }
}

struct Cx<'a> {
    painter: &'a Painter,
    color: Color32,
}

/// Altura do eixo (onde fica o traço da fração e o meio do "+").
fn axis(size: f32) -> f32 {
    size * 0.27
}

fn script(size: f32) -> f32 {
    (size * 0.7).max(9.0)
}

impl Cx<'_> {
    fn text(&self, s: &str, font: Font, size: f32) -> Lay {
        let w = match font {
            Font::Italic => W::Italic,
            Font::Roman => W::Regular,
            Font::Bold => W::Semi,
        };
        let g = self.painter.layout_no_wrap(s.to_owned(), serif(size, w), self.color);
        let base = g.rows.first().map_or(size * 0.8, |r| r.pos.y + r.row.glyphs.first().map_or(size * 0.8, |gl| gl.pos.y));
        let mut l = Lay::new(g.size().x, size * 0.72, size * 0.24);
        l.items.push((Vec2::ZERO, Item::Text(g, base)));
        l
    }

    fn lay(&self, n: &Node, size: f32) -> Lay {
        match n {
            Node::Sym(s, f) => self.text(s, *f, size),
            Node::Op(s) | Node::Punct(s) | Node::Func(s, _) => self.text(s, Font::Roman, size),
            Node::Big(s, _) => {
                // A integral das fontes é baixa: sai maior que o somatório.
                let fs = if matches!(s.as_str(), "∫" | "∬" | "∭" | "∮") { size * 2.0 } else { size * 1.45 };
                let l = self.text(s, Font::Roman, fs);
                // Centraliza o símbolo no eixo.
                let dy = fs * 0.36 - axis(size);
                let mut out = Lay::new(l.w, fs * 0.74 - dy, dy + fs * 0.12);
                out.place(l, 0.0, dy);
                out.asc = fs * 0.74 - dy;
                out.desc = dy + fs * 0.12;
                out
            }
            Node::Space(em) => Lay::new(em * size, 0.0, 0.0),
            Node::Row(v) => self.row(v, size),
            Node::Frac(a, b, line) => {
                let cs = (size * 0.82).max(9.0);
                let (la, lb) = (self.lay(a, cs), self.lay(b, cs));
                let pad = size * 0.12;
                let w = la.w.max(lb.w) + 2.0 * pad;
                let t = (size * 0.05).max(1.0);
                let gap = size * 0.16;
                let ax = axis(size);
                let ny = -ax - t / 2.0 - gap - la.desc;
                let dy = -ax + t / 2.0 + gap + lb.asc;
                let mut out = Lay::new(w + size * 0.1, 0.0, 0.0);
                let x0 = size * 0.05;
                let (aw, bw) = (la.w, lb.w);
                out.place(la, x0 + (w - aw) / 2.0, ny);
                out.place(lb, x0 + (w - bw) / 2.0, dy);
                if *line {
                    out.path(vec![vec2(x0, -ax), vec2(x0 + w, -ax)], t);
                }
                out
            }
            Node::Sqrt(index, body) => {
                let lb = self.lay(body, size);
                let gap = size * 0.1;
                let t = (size * 0.055).max(1.0);
                let top = -(lb.asc + gap + t);
                let bottom = lb.desc + size * 0.04;
                let h = bottom - top;
                let sw = (size * 0.55).min(h * 0.6).max(size * 0.4);
                let li = index.as_ref().map(|i| self.lay(i, (size * 0.55).max(8.0)));
                let x0 = li.as_ref().map_or(0.0, |l| (l.w - sw * 0.45).max(0.0));
                let bw = lb.w;
                let mut out = Lay::new(x0 + sw + bw + gap * 1.5, -top + t / 2.0, bottom);
                out.path(
                    vec![
                        vec2(x0, top + h * 0.6),
                        vec2(x0 + sw * 0.22, top + h * 0.52),
                        vec2(x0 + sw * 0.5, bottom),
                        vec2(x0 + sw, top),
                        vec2(x0 + sw + bw + gap, top),
                    ],
                    t,
                );
                out.place(lb, x0 + sw + gap * 0.5, 0.0);
                if let Some(li) = li {
                    let iw = li.w;
                    out.place(li, (x0 + sw * 0.45 - iw).max(0.0), top + h * 0.5 - size * 0.06);
                }
                out
            }
            Node::Scripts { base, sup, sub } => {
                let limits = matches!(**base, Node::Big(_, true) | Node::Func(_, true));
                let lb = self.lay(base, size);
                let ss = script(size);
                let la = sup.as_ref().map(|s| self.lay(s, ss));
                let ld = sub.as_ref().map(|s| self.lay(s, ss));
                if limits {
                    let w = lb.w.max(la.as_ref().map_or(0.0, |l| l.w)).max(ld.as_ref().map_or(0.0, |l| l.w));
                    let (basc, bdesc, bw) = (lb.asc, lb.desc, lb.w);
                    let mut out = Lay::new(w, 0.0, 0.0);
                    out.place(lb, (w - bw) / 2.0, 0.0);
                    if let Some(la) = la {
                        let (aw, ad) = (la.w, la.desc);
                        out.place(la, (w - aw) / 2.0, -basc - size * 0.1 - ad);
                    }
                    if let Some(ld) = ld {
                        let (dw, da) = (ld.w, ld.asc);
                        out.place(ld, (w - dw) / 2.0, bdesc + size * 0.1 + da);
                    }
                    return out;
                }
                let big = matches!(**base, Node::Big(..));
                let x = lb.w + size * if big { 0.02 } else { 0.04 };
                let up = if big { lb.asc - ss * 0.4 } else { (size * 0.38).max(lb.asc - ss * 0.45) };
                let mut down = if big { lb.desc + ss * 0.15 } else { (size * 0.2).max(lb.desc - size * 0.08) };
                if la.is_some() && ld.is_some() {
                    down = down.max(size * 0.28);
                }
                let sw = la.as_ref().map_or(0.0, |l| l.w).max(ld.as_ref().map_or(0.0, |l| l.w));
                let mut out = Lay::new(x + sw + size * 0.04, 0.0, 0.0);
                out.place(lb, 0.0, 0.0);
                if let Some(la) = la {
                    out.place(la, x, -up);
                }
                if let Some(ld) = ld {
                    out.place(ld, x, down);
                }
                out
            }
            Node::Delim(l, body, r) => {
                let lb = self.lay(body, size);
                let (asc, desc) = (lb.asc.max(size * 0.72), lb.desc.max(size * 0.24));
                let dl = self.delim(l, asc, desc, size);
                let dr = self.delim(r, asc, desc, size);
                let pad = if l.is_empty() && r.is_empty() { 0.0 } else { size * 0.06 };
                let (lw, bw, rw) = (dl.w, lb.w, dr.w);
                let mut out = Lay::new(lw + bw + rw + 2.0 * pad, 0.0, 0.0);
                out.place(dl, 0.0, 0.0);
                out.place(lb, lw + pad, 0.0);
                out.place(dr, lw + bw + 2.0 * pad, 0.0);
                out
            }
            Node::Accent(a, body) => {
                let lb = self.lay(body, size);
                let gap = size * 0.08;
                let t = (size * 0.05).max(1.0);
                let (w, basc, bdesc) = (lb.w, lb.asc, lb.desc);
                // Uma letra só (itálica): o acento fica um pouco à direita.
                let cx = if w < size * 0.8 { w * 0.58 } else { w / 2.0 };
                let y = -(basc + gap);
                let mut out = Lay::new(w, 0.0, 0.0);
                match a {
                    Acc::Boxed => {
                        let pad = size * 0.2;
                        out.w = w + 2.0 * pad;
                        out.place(lb, pad, 0.0);
                        out.items.push((vec2(0.0, -basc - pad), Item::Frame(vec2(w + 2.0 * pad, basc + bdesc + 2.0 * pad), t)));
                        out.asc = basc + pad + t;
                        out.desc = bdesc + pad + t;
                        return out;
                    }
                    Acc::Under => out.path(vec![vec2(0.0, bdesc + gap), vec2(w, bdesc + gap)], t),
                    Acc::Bar => out.path(vec![vec2(0.0, y), vec2(w, y)], t),
                    Acc::Arrow => {
                        let y = y - size * 0.04;
                        let wa = w.max(size * 0.45);
                        let x0 = cx - wa / 2.0;
                        let x1 = x0 + wa;
                        out.path(vec![vec2(x0, y), vec2(x1, y)], t);
                        out.path(vec![vec2(x1 - size * 0.13, y - size * 0.09), vec2(x1, y), vec2(x1 - size * 0.13, y + size * 0.09)], t);
                    }
                    Acc::Hat => {
                        let hw = (w * 0.8).max(size * 0.32);
                        out.path(vec![vec2(cx - hw / 2.0, y), vec2(cx, y - size * 0.14), vec2(cx + hw / 2.0, y)], t);
                    }
                    Acc::Tilde => {
                        let hw = (w * 0.8).max(size * 0.36);
                        let pts = (0..=12)
                            .map(|k| {
                                let f = k as f32 / 12.0;
                                vec2(cx - hw / 2.0 + f * hw, y - size * 0.06 - (f * std::f32::consts::TAU).sin() * size * 0.05)
                            })
                            .collect();
                        out.path(pts, t);
                    }
                    Acc::Dot => out.items.push((vec2(cx, y - size * 0.04), Item::Dot(size * 0.055))),
                    Acc::DDot => {
                        out.items.push((vec2(cx - size * 0.11, y - size * 0.04), Item::Dot(size * 0.055)));
                        out.items.push((vec2(cx + size * 0.11, y - size * 0.04), Item::Dot(size * 0.055)));
                    }
                }
                out.place(lb, 0.0, 0.0);
                match a {
                    Acc::Under => out.desc = bdesc + gap + t,
                    _ => out.asc = basc + gap + size * 0.18,
                }
                out
            }
            Node::Grid(rows, env) => self.grid(rows, *env, size),
        }
    }

    fn row(&self, v: &[Node], size: f32) -> Lay {
        let mut out = Lay::new(0.0, 0.0, 0.0);
        let mut prev: Option<Class> = None;
        let mut pending = 0.0;
        // Nos índices (letra menor), operadores quase sem espaço, como no TeX.
        let op_gap = if size < 15.0 { size * 0.08 } else { size * 0.24 };
        for n in v {
            let c = class(n);
            let mut after = 0.0;
            match c {
                // Binário entre dois termos; no começo ou depois de outro operador, é sinal ("-x").
                Class::Op if matches!(prev, Some(Class::Ord | Class::Func)) => {
                    pending += op_gap;
                    after = op_gap;
                }
                Class::Punct => after = size * 0.17,
                Class::Func => after = size * 0.1,
                _ => {}
            }
            if c == Class::Open && prev == Some(Class::Func) {
                pending = 0.0;
            }
            let l = self.lay(n, size);
            let x = out.w + pending;
            out.w = x + l.w;
            out.place(l, x, 0.0);
            pending = after;
            prev = Some(c);
        }
        out
    }

    fn grid(&self, rows: &[Vec<Node>], env: Env, size: f32) -> Lay {
        if rows.len() == 1 && rows[0].len() == 1 {
            return self.lay(&rows[0][0], size);
        }
        let cells: Vec<Vec<Lay>> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(c, n)| match (env, n) {
                        // Em `aligned`, a coluna depois do `&` começa com "=": conta como entre dois termos.
                        (Env::Aligned, Node::Row(v)) if c % 2 == 1 => self.row(&[vec![Node::Row(vec![])], v.clone()].concat(), size),
                        _ => self.lay(n, size),
                    })
                    .collect()
            })
            .collect();
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0f32; ncols];
        for r in &cells {
            for (c, l) in r.iter().enumerate() {
                widths[c] = widths[c].max(l.w);
            }
        }
        let col_gap = |c: usize| match env {
            Env::Matrix => size * 0.9,
            Env::Cases => size * 1.0,
            Env::Aligned if c % 2 == 1 => size * 1.5,
            _ => 0.0,
        };
        let row_gap = match env {
            Env::Lines | Env::Aligned => size * 0.45,
            _ => size * 0.3,
        };
        let heights: Vec<(f32, f32)> = cells.iter().map(|r| r.iter().fold((size * 0.72, size * 0.24), |(a, d), l| (a.max(l.asc), d.max(l.desc)))).collect();
        let total_h: f32 = heights.iter().map(|(a, d)| a + d).sum::<f32>() + row_gap * (heights.len().saturating_sub(1)) as f32;
        let total_w: f32 = widths.iter().sum::<f32>() + (0..ncols.saturating_sub(1)).map(col_gap).sum::<f32>();
        let top = -axis(size) - total_h / 2.0;
        let mut out = Lay::new(total_w, axis(size) + total_h / 2.0, total_h / 2.0 - axis(size));
        let mut y = top;
        for (r, (a, d)) in cells.into_iter().zip(heights) {
            let base = y + a;
            let mut x = 0.0;
            for (c, l) in r.into_iter().enumerate() {
                let free = widths[c] - l.w;
                let dx = match env {
                    Env::Cases => 0.0,
                    Env::Aligned if c % 2 == 0 => free,
                    Env::Aligned => 0.0,
                    _ => free / 2.0,
                };
                out.place(l, x + dx, base);
                x += widths[c] + col_gap(c);
            }
            y = base + d + row_gap;
        }
        out
    }

    /// Parêntese, colchete, chave… da altura do conteúdo.
    fn delim(&self, s: &str, asc: f32, desc: f32, size: f32) -> Lay {
        if s.is_empty() {
            return Lay::new(0.0, 0.0, 0.0);
        }
        if asc <= size * 0.8 && desc <= size * 0.3 {
            return self.text(s, Font::Roman, size);
        }
        let t = (size * 0.06).max(1.0);
        let (top, bottom) = (-asc - size * 0.04, desc + size * 0.04);
        let h = bottom - top;
        let mid = (top + bottom) / 2.0;
        let w = match s {
            "|" => size * 0.25,
            "‖" => size * 0.35,
            _ => (size * 0.38).min(size * 0.2 + h * 0.06),
        };
        let mut out = Lay::new(w, asc + size * 0.04, desc + size * 0.04);
        let curve = |mirror: bool| -> Vec<Vec2> {
            (0..=16)
                .map(|k| {
                    let f = k as f32 / 8.0 - 1.0;
                    let x = w * (0.85 - 0.6 * (1.0 - f * f));
                    vec2(if mirror { w - x } else { x }, mid + f * h / 2.0)
                })
                .collect()
        };
        let brace = |mirror: bool| -> Vec<Vec2> {
            let pts = [(0.85, top), (0.55, top + h * 0.06), (0.5, mid - h * 0.08), (0.2, mid), (0.5, mid + h * 0.08), (0.55, bottom - h * 0.06), (0.85, bottom)];
            pts.iter().map(|(x, y)| vec2(if mirror { w - w * x } else { w * x }, *y)).collect()
        };
        match s {
            "(" => out.path(curve(false), t),
            ")" => out.path(curve(true), t),
            "[" => out.path(vec![vec2(w * 0.75, top), vec2(w * 0.3, top), vec2(w * 0.3, bottom), vec2(w * 0.75, bottom)], t),
            "]" => out.path(vec![vec2(w * 0.25, top), vec2(w * 0.7, top), vec2(w * 0.7, bottom), vec2(w * 0.25, bottom)], t),
            "{" => out.path(brace(false), t),
            "}" => out.path(brace(true), t),
            "⟨" => out.path(vec![vec2(w * 0.8, top), vec2(w * 0.2, mid), vec2(w * 0.8, bottom)], t),
            "⟩" => out.path(vec![vec2(w * 0.2, top), vec2(w * 0.8, mid), vec2(w * 0.2, bottom)], t),
            "|" => out.path(vec![vec2(w * 0.5, top), vec2(w * 0.5, bottom)], t),
            "‖" => {
                out.path(vec![vec2(w * 0.35, top), vec2(w * 0.35, bottom)], t);
                out.path(vec![vec2(w * 0.65, top), vec2(w * 0.65, bottom)], t);
            }
            _ => {
                let l = self.text(s, Font::Roman, h * 0.85);
                let lw = l.w;
                out.w = lw;
                out.place(l, 0.0, mid + h * 0.85 * 0.27);
            }
        }
        out
    }
}

fn paint(painter: &Painter, l: &Lay, origin: Pos2, color: Color32) {
    for (off, it) in &l.items {
        let o = origin + *off;
        match it {
            Item::Text(g, base) => painter.galley(pos2(o.x, o.y - base), g.clone(), color),
            Item::Path(pts, t) => {
                painter.add(egui::Shape::line(pts.iter().map(|v| origin + *v + *off).collect(), Stroke::new(*t, color)));
            }
            Item::Frame(size, t) => {
                painter.rect_stroke(Rect::from_min_size(o, *size), 2.0, Stroke::new(*t, color), StrokeKind::Middle);
            }
            Item::Dot(r) => {
                painter.circle_filled(o, *r, color);
            }
        }
    }
}

/// Desenha a fórmula centralizada na largura `w`. Se não couber, diminui (até 55%).
pub fn show(ui: &mut Ui, src: &str, size: f32, color: Color32, w: f32) -> egui::Response {
    let node = parse(src);
    let painter = ui.painter().clone();
    let cx = Cx { painter: &painter, color };
    let mut l = cx.lay(&node, size);
    if l.w > w && l.w > 0.0 {
        l = cx.lay(&node, (size * w / l.w).max(size * 0.55));
    }
    let pad = size * 0.25;
    // Só clique: com foco, roubaria o teclado do campo de LaTeX que abre no clique.
    let (rect, resp) = ui.allocate_exact_size(vec2(w, l.asc + l.desc + 2.0 * pad), Sense::CLICK);
    let x = (rect.center().x - l.w / 2.0).max(rect.left());
    paint(ui.painter(), &l, pos2(x, rect.top() + pad + l.asc), color);
    a11y(&resp, egui::accesskit::Role::Image, src, None);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_common_latex() {
        let Node::Grid(rows, _) = parse(r"\frac{a}{b}") else { panic!() };
        assert!(matches!(rows[0][0], Node::Row(ref v) if matches!(v[0], Node::Frac(..))));
        let Node::Grid(rows, Env::Lines) = parse(r"a = b \\ c = d") else { panic!() };
        assert_eq!(rows.len(), 2);
        let Node::Grid(rows, _) = parse(r"\text{Peso (kg)} = 2 \times \text{idade}") else { panic!() };
        let Node::Row(v) = &rows[0][0] else { panic!() };
        assert_eq!(v[0], Node::Sym("Peso (kg)".into(), Font::Roman));
        assert_eq!(v[1], Node::Op("=".into()));
        assert_eq!(v[3], Node::Op("×".into()));
        let Node::Grid(rows, _) = parse(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}") else { panic!() };
        let Node::Row(v) = &rows[0][0] else { panic!() };
        let Node::Delim(l, g, r) = &v[0] else { panic!("{v:?}") };
        assert_eq!((l.as_str(), r.as_str()), ("(", ")"));
        assert!(matches!(**g, Node::Grid(ref rs, Env::Matrix) if rs.len() == 2 && rs[1].len() == 2));
    }

    #[test]
    fn broken_input_still_ends() {
        for s in ["", "\\", "{", "}", "}}a{", "\\frac", "x^", "_", "\\left(", "\\right)", "a & b", "\\end{x}", "\\sqrt[", "\\text{abc", "\\begin{cases}", "\\\\\\\\", "\\left\\{ x", "2,5^{", "\\not", "$x$"] {
            let _ = parse(s);
        }
    }
}
