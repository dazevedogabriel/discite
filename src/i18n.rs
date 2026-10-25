//! Translations. Every text the user sees comes from `locales/<code>.xml`:
//!
//! ```xml
//! <language code="pt-BR" name="Português (Brasil)">
//!   <string key="list.empty">Nada por aqui.</string>
//!   <plural key="list.count"><one>{n} item</one><other>{n} itens</other></plural>
//! </language>
//! ```
//!
//! The built-in languages are embedded in the executable. Any `*.xml` in
//! `<exe folder>/locales` or `<data folder>/locales` is loaded as well: it
//! adds a language, or overrides keys of a built-in one with the same code.
//! A key missing from the chosen language falls back to English, then to
//! Portuguese, and finally shows the key itself, so nothing is ever blank.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

/// Languages shipped inside the executable.
const BUILT_IN: [&str; 2] = [include_str!("../locales/pt-BR.xml"), include_str!("../locales/en.xml")];
/// Tried, in order, after the chosen language.
const FALLBACKS: [&str; 2] = ["en", "pt-BR"];

#[derive(Clone, Debug, PartialEq)]
pub struct Language {
    /// BCP 47 code, e.g. `pt-BR`.
    pub code: String,
    /// Name in the language itself, e.g. `Português (Brasil)`.
    pub name: String,
}

#[derive(Clone, Debug)]
enum Entry {
    Text(String),
    Plural { one: String, other: String },
}

#[derive(Default)]
struct Catalog {
    languages: Vec<Language>,
    entries: HashMap<String, HashMap<String, Entry>>,
    /// Chosen language followed by the fallbacks.
    chain: Vec<String>,
}

static CATALOG: OnceLock<RwLock<Catalog>> = OnceLock::new();

fn catalog() -> &'static RwLock<Catalog> {
    CATALOG.get_or_init(|| {
        let mut c = Catalog::default();
        for xml in BUILT_IN {
            if let Err(e) = c.load(xml) {
                // Built-in files are checked by the tests; this is a bug.
                eprintln!("invalid built-in locale: {e}");
            }
        }
        for dir in extra_dirs() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            for file in read.flatten().filter(|f| f.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"))) {
                match std::fs::read_to_string(file.path()) {
                    Ok(xml) => {
                        if let Err(e) = c.load(&xml) {
                            eprintln!("{}: {e}", file.path().display());
                        }
                    }
                    Err(e) => eprintln!("{}: {e}", file.path().display()),
                }
            }
        }
        c.select(None);
        RwLock::new(c)
    })
}

/// Folders searched for extra or overriding `*.xml` translations.
pub fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("locales"))) {
        dirs.push(dir);
    }
    // `%APPDATA%\Discite\locales` no Windows, `~/Library/Application Support/Discite/locales` no macOS.
    dirs.push(crate::db::data_root().join("locales"));
    dirs
}

impl Catalog {
    fn load(&mut self, xml: &str) -> Result<(), String> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if root.tag_name().name() != "language" {
            return Err("the root element must be <language>".into());
        }
        let code = root.attribute("code").ok_or("<language> needs a code attribute")?.to_owned();
        let name = root.attribute("name").unwrap_or(&code).to_owned();
        let table = self.entries.entry(code.clone()).or_default();
        for node in root.children().filter(|n| n.is_element()) {
            let Some(key) = node.attribute("key") else { continue };
            let entry = match node.tag_name().name() {
                "string" => Entry::Text(node.text().unwrap_or_default().to_owned()),
                "plural" => {
                    let form = |tag: &str| {
                        node.children()
                            .find(|c| c.has_tag_name(tag))
                            .and_then(|c| c.text())
                            .unwrap_or_default()
                            .to_owned()
                    };
                    Entry::Plural { one: form("one"), other: form("other") }
                }
                _ => continue,
            };
            table.insert(key.to_owned(), entry);
        }
        match self.languages.iter_mut().find(|l| l.code == code) {
            Some(l) => l.name = name,
            None => self.languages.push(Language { code, name }),
        }
        Ok(())
    }

    /// `None` = follow the system language.
    fn select(&mut self, code: Option<&str>) {
        let wanted = code.map(str::to_owned).unwrap_or_else(system_language);
        let chosen = self.best_match(&wanted);
        self.chain = std::iter::once(chosen)
            .chain(FALLBACKS.iter().map(|s| s.to_string()))
            .fold(Vec::new(), |mut v, c| {
                if !v.contains(&c) {
                    v.push(c);
                }
                v
            });
    }

    /// Exact code, then same language (`pt-PT` → `pt-BR`), then English.
    fn best_match(&self, wanted: &str) -> String {
        let has = |c: &str| self.languages.iter().find(|l| l.code.eq_ignore_ascii_case(c)).map(|l| l.code.clone());
        let base = wanted.split(['-', '_']).next().unwrap_or(wanted);
        has(wanted)
            .or_else(|| {
                self.languages
                    .iter()
                    .find(|l| l.code.split('-').next().is_some_and(|b| b.eq_ignore_ascii_case(base)))
                    .map(|l| l.code.clone())
            })
            .unwrap_or_else(|| "en".into())
    }

    fn lookup(&self, key: &str) -> Option<&Entry> {
        self.chain.iter().find_map(|code| self.entries.get(code).and_then(|t| t.get(key)))
    }
}

/// The user's Windows display language, e.g. `pt-BR`.
fn system_language() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if n > 1 {
            return String::from_utf16_lossy(&buf[..n as usize - 1]);
        }
    }
    std::env::var("LANG").ok().and_then(|l| l.split('.').next().map(|s| s.replace('_', "-"))).unwrap_or_else(|| "en".into())
}

// ------------------------------------------------------------------ API

/// Code of the language in use (first of the chain), e.g. `en`.
pub fn current() -> String {
    catalog().read().unwrap().chain.first().cloned().unwrap_or_else(|| "en".into())
}

/// Number with the language's decimal separator and no trailing ",0"
/// (`format.decimal` in the XML: "." in English, "," in Portuguese).
pub fn decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    s.replace('.', &text("format.decimal", &[]))
}

/// Number with a fixed number of decimals ("0.90" / "0,90").
pub fn fixed(v: f64, places: usize) -> String {
    format!("{v:.places$}").replace('.', &text("format.decimal", &[]))
}

/// Integer with thousands separators (`format.thousands`: "1,946" / "1.946").
pub fn thousands(n: i64) -> String {
    let sep = text("format.thousands", &[]);
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push_str(&sep);
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Languages available, in load order (built-in first).
pub fn languages() -> Vec<Language> {
    catalog().read().unwrap().languages.clone()
}

/// Switches the language; `None` follows the system.
pub fn set_language(code: Option<&str>) {
    catalog().write().unwrap().select(code);
}

fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Text for `key` with `{name}` placeholders filled from `args`.
pub fn text(key: &str, args: &[(&str, String)]) -> String {
    let c = catalog().read().unwrap();
    match c.lookup(key) {
        Some(Entry::Text(t)) => fill(t, args),
        Some(Entry::Plural { other, .. }) => fill(other, args),
        None => key.to_owned(),
    }
}

/// Plural text for `key`: the `one` form when `n == 1`, `other` otherwise.
/// `{n}` is filled automatically.
pub fn plural(key: &str, n: u64, args: &[(&str, String)]) -> String {
    let c = catalog().read().unwrap();
    let mut all = vec![("n", n.to_string())];
    all.extend(args.iter().cloned());
    match c.lookup(key) {
        Some(Entry::Plural { one, other }) => fill(if n == 1 { one } else { other }, &all),
        Some(Entry::Text(t)) => fill(t, &all),
        None => key.to_owned(),
    }
}

/// `tr!("key")` or `tr!("key", name = value, …)` → translated `String`.
#[macro_export]
macro_rules! tr {
    ($key:expr $(,)?) => { $crate::i18n::text($key, &[]) };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::text($key, &[$((stringify!($name), $value.to_string())),+])
    };
}

/// `tr_n!("key", n)` or `tr_n!("key", n, name = value, …)` → plural form.
#[macro_export]
macro_rules! tr_n {
    ($key:expr, $n:expr $(,)?) => { $crate::i18n::plural($key, $n as u64, &[]) };
    ($key:expr, $n:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::plural($key, $n as u64, &[$((stringify!($name), $value.to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys_of(xml: &str) -> (String, BTreeSet<String>) {
        let mut c = Catalog::default();
        c.load(xml).expect("XML válido");
        let code = c.languages[0].code.clone();
        let keys = c.entries[&code].keys().cloned().collect();
        (code, keys)
    }

    /// Every key used in the source exists in every built-in language, and
    /// no language carries keys the code no longer uses.
    #[test]
    fn built_in_languages_match_the_code() {
        let (_, reference) = keys_of(BUILT_IN[0]);
        let namespaces: BTreeSet<String> = reference.iter().filter_map(|k| k.split('.').next()).map(str::to_owned).collect();
        let used = used_keys(&namespaces);
        assert!(used.len() > 50, "too few keys found: {}", used.len());
        for xml in BUILT_IN {
            let (code, keys) = keys_of(xml);
            let missing: Vec<_> = used.difference(&keys).collect();
            let unused: Vec<_> = keys.difference(&used).collect();
            assert!(missing.is_empty(), "{code}: missing {missing:?}");
            assert!(unused.is_empty(), "{code}: unused {unused:?}");
        }
    }

    /// Every string literal under src/ shaped like `namespace.name` with a
    /// known namespace: covers `tr!("…")` as well as keys picked in a
    /// `match` and passed on (`tr!(match s { … => "action.pause" })`).
    fn used_keys(namespaces: &BTreeSet<String>) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        let mut stack = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let src = std::fs::read_to_string(&p).unwrap();
                    // `"` + [a-z0-9_.]+ + `"`, wherever it appears (no quote pairing,
                    // which char literals like '"' would throw off).
                    for (i, _) in src.match_indices('"') {
                        let rest = &src[i + 1..];
                        let len = rest.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')).unwrap_or(0);
                        if !rest[len..].starts_with('"') {
                            continue;
                        }
                        let literal = &rest[..len];
                        let Some((ns, name)) = literal.split_once('.') else { continue };
                        if !name.is_empty() && !name.contains('.') && namespaces.contains(ns) {
                            keys.insert(literal.to_owned());
                        }
                    }
                }
            }
        }
        keys
    }

    #[test]
    fn placeholders_and_plurals() {
        let mut c = Catalog::default();
        c.load(r#"<language code="xx" name="Test"><string key="a">Olá, {nome}!</string><plural key="p"><one>{n} item</one><other>{n} itens</other></plural></language>"#).unwrap();
        c.chain = vec!["xx".into()];
        let Some(Entry::Text(t)) = c.lookup("a") else { panic!() };
        assert_eq!(fill(t, &[("nome", "Ferris".into())]), "Olá, Ferris!");
        let Some(Entry::Plural { one, other }) = c.lookup("p") else { panic!() };
        assert_eq!(fill(one, &[("n", "1".into())]), "1 item");
        assert_eq!(fill(other, &[("n", "3".into())]), "3 itens");
    }

    #[test]
    fn language_matching() {
        let mut c = Catalog::default();
        for xml in BUILT_IN {
            c.load(xml).unwrap();
        }
        assert_eq!(c.best_match("pt-BR"), "pt-BR");
        assert_eq!(c.best_match("pt-PT"), "pt-BR");
        assert_eq!(c.best_match("en-GB"), "en");
        assert_eq!(c.best_match("ja-JP"), "en");
    }
}
