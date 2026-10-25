<p align="center">
  <img src="assets/logo.svg" alt="Discite logo" width="160">
</p>

<h1 align="center">Discite</h1>

<p align="center">
  <strong>A study notebook and spaced-repetition flashcards in one desktop app — written in Rust 🦀</strong><br>
  Take notes, turn them into cards, and let FSRS decide when you see them again.
</p>

<p align="center">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-dea584?logo=rust&logoColor=white">
  <img alt="Rust edition 2024" src="https://img.shields.io/badge/edition-2024-f0643a">
  <img alt="GUI with egui" src="https://img.shields.io/badge/GUI-egui-3aa6c5">
  <img alt="Scheduling with FSRS-6" src="https://img.shields.io/badge/scheduler-FSRS--6-8b8cf8">
  <img alt="Storage in SQLite" src="https://img.shields.io/badge/storage-SQLite-4a90c8?logo=sqlite&logoColor=white">
  <img alt="Windows and macOS" src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS-555">
</p>

<p align="center">
  <a href="#-features">Features</a> ·
  <a href="#-how-scheduling-works">How scheduling works</a> ·
  <a href="#-getting-started">Getting started</a> ·
  <a href="#%EF%B8%8F-shortcuts">Shortcuts</a> ·
  <a href="#-releases">Releases</a> ·
  <a href="#-translations">Translations</a> ·
  <a href="#%EF%B8%8F-project-layout">Project layout</a>
</p>

<p align="center">
  <img src="docs/images/dashboard.png" alt="Discite dashboard: today's cards, one tile per subject, a year of reviews and the next 7 days" width="900">
</p>

---

**Discite** (Latin for *"learn!"*) puts the two halves of studying in one place.
You write your notes in a block editor, grouped by subject. You turn any passage
into a flashcard without leaving the page. Every card keeps a link back to the
note it came from, and every answer you give is logged, so the scheduler
learns how *your* memory works. It is a single native executable, and all
your data lives in one SQLite file on your machine.

<table>
  <tr>
    <td><img src="docs/images/notebook.png" alt="A notebook page with outline, backlinks and the cards created from it"></td>
    <td><img src="docs/images/review.png" alt="Reviewing a cloze card, with the next interval on each button"></td>
  </tr>
  <tr>
    <td><img src="docs/images/cards.png" alt="Card browser with search, filters and a card's retrievability curve and history"></td>
    <td><img src="docs/images/statistics.png" alt="Statistics: reviews per day, forecast, calibration, card count, buttons, hour of day"></td>
  </tr>
</table>

## ✨ Features

### 📓 Notebook

- **Block editor.** Pages and sub-pages per subject, with paragraphs, headings,
  bulleted, numbered and checklists, quotes, callouts, code, formulas, tables,
  dividers and images.
- **Live markup.** `**bold**`, `*italic*`, `__underline__`, `~~strike~~`,
  `` `code` ``, `==highlight==` and `[[Page links]]` (Ctrl+click follows them).
  Typing `# `, `- `, `1. `, `[] `, `> `, ```` ``` ```` or `---` turns the line
  into that block, and `/` opens the block menu.
- **Images.** Paste with Ctrl+V, drop a file on the window or pick one from
  disk. Each image is copied into Discite's `media` folder, so the note never
  breaks when the original moves. Click an image to zoom in.
- **Autosave**, version history, backlinks, outline, tags, full-text search
  (Ctrl+K), and Markdown import/export.
- **Create flashcard** (Ctrl+Shift+F) from the selected text. The card
  remembers its source page, and the page lists every card made from it.

### 🗂️ Flashcards

- **Note types.** Basic, Basic + reversed, Cloze (`{{c1::answer::hint}}`,
  Ctrl+Shift+C), Type the answer, and Image occlusion (masks drawn on the image).
- **FSRS-6 scheduling** through the [`fsrs`](https://crates.io/crates/fsrs)
  crate, the same engine Anki uses. It includes:
  - learning and relearning steps;
  - guaranteed Hard < Good < Easy;
  - fuzz and load balancing;
  - daily limits and a configurable day rollover (04:00 by default);
  - buried siblings and leech detection;
  - SM-2 as an alternative.
- **Everything is logged.** Each answer goes to the `revlog` with its grade,
  time, elapsed days, states, D/S before and after, and the predicted R. Undo
  (Ctrl+Z) never deletes a row; it only marks the answer as undone.
- **Presets with an optimizer.** It trains the parameters on your own history
  and shows log loss and RMSE before and after. A simulator shows the workload
  against retention over 365 days.
- **Card browser** with a search language: `deck:`, `subject:`, `tag:`,
  `is:due/new/learn/review/suspended/flagged/leech`, `prop:r<0.8`, `prop:d>7`,
  `lapses>2`, `added:7`, `rated:1:1`, `"text"` and `-negation`. It also has
  bulk actions and a side panel with each card's retrievability curve and history.
- **Custom study.** Forgotten cards, review ahead, recent new cards, low R,
  flagged cards, or any search.

### 📊 Dashboard and statistics

- **A "Today" panel** with the estimated study time, a tile per subject, a
  year-long heatmap, recent pages, and a 7-day forecast.
- **Statistics:**
  - reviews per day by type, a 30-day forecast, and calibration (predicted vs observed R);
  - card count, button frequency, and accuracy by hour;
  - D, S and R distributions;
  - knowledge retained over time.

### 🎨 And also

- A command palette for every action, plus dark and light themes based on a
  Claude Design canvas. Literata and JetBrains Mono fonts are built in.
- On Windows, a custom title bar. On macOS, the native one, with shortcuts
  shown with ⌘.
- English and Brazilian Portuguese are built in. More languages can be added
  as plain XML files (see [Translations](#-translations)).

<p align="center">
  <img src="docs/images/light.png" alt="The notebook in the light theme" width="900">
</p>

## 🧠 How scheduling works

FSRS models each card with three numbers:

| | Meaning |
|---|---|
| **D**: difficulty | How hard the card is for you, from 1 to 10. |
| **S**: stability | How many days it takes for the chance of remembering it to fall to 90%. |
| **R**: retrievability | The chance you remember it *right now*. It decays with time following a power curve. |

Each answer (Again, Hard, Good, Easy) updates D and S. The next review is
scheduled for the day R is expected to reach your **desired retention**
(90% by default). The buttons show that interval before you press them.
A higher target means more reviews and better recall. The simulator in each
preset shows the trade-off before you change it.

Because every answer is logged with the R that was predicted for it, Discite
can check itself. The calibration chart compares predicted and observed recall.
The optimizer fits the 21 FSRS parameters to your history and shows how well the
old and new ones predict it before you apply them.

## 🚀 Getting started

You need [Rust](https://rustup.rs) (stable, edition 2024). On Windows you also
need the Visual Studio Build Tools (MSVC); on macOS, the Xcode command line tools.

```bash
git clone https://github.com/dazevedogabriel/Discite.git discite
cd discite
cargo build --release
```

Run `target/release/discite.exe` (Windows) or `target/release/discite` (macOS).
It needs nothing installed next to it.

- **Your data** lives in `%APPDATA%\Discite` on Windows and in
  `~/Library/Application Support/Discite` on macOS. It holds `discite.db`
  (SQLite), `media/` with the images, and `backups/` with automatic backups.
  To try things without touching your data, point the `DISCITE_DATA`
  environment variable at another folder.
- **Sample content.** On an empty database, the dashboard offers to load an
  example: four subjects with pages, decks, notes and a simulated year of reviews
  (the screenshots above). With `DISCITE_DEMO=1` it loads at startup.
- **Tests:** `cargo test`.

## ⌨️ Shortcuts

On macOS, Ctrl is ⌘.

| Where | Key | Action |
|---|---|---|
| Anywhere | Ctrl K | Command palette |
| Anywhere | Ctrl N | New page |
| Anywhere | Ctrl Shift A | Add note |
| Anywhere | Ctrl \\ | Collapse/expand the sidebar |
| Anywhere | Ctrl F | Search cards |
| Dashboard | Space | Study all |
| Review | Space / Enter | Show answer, then Good |
| Review | 1 2 3 4 | Again, Hard, Good, Easy |
| Review | Ctrl Z | Undo |
| Review | E / I / O | Edit, Info, Source |
| Review | @ / - / * | Suspend, Bury, Flag |
| Notebook | Ctrl B / I / U / E | Bold, italic, underline, code |
| Notebook | Ctrl Shift H | Highlight |
| Notebook | Ctrl Shift F | Create flashcard from selection |
| Notebook | Tab / Shift Tab | Indent / outdent list item |
| Note editor | Ctrl Enter | Add or save |

## 📦 Releases

Two GitHub Actions workflows build the app for both platforms:

- **[`build.yml`](.github/workflows/build.yml)** builds and tests on Windows x64
  and macOS universal (Apple Silicon + Intel, macOS 10.13 or newer). It runs on
  every push to `main`, on pull requests, and on demand (*Actions → Build → Run
  workflow*). The packages are attached to the run:
  - `Discite-<version>-windows-x64.zip` with `Discite.exe`;
  - `Discite-<version>-macos-universal.zip` with `Discite.app`.
- **[`release.yml`](.github/workflows/release.yml)** publishes a GitHub release
  with both packages when a `v*` tag is pushed. The tag must match `version` in
  `Cargo.toml`:

  ```bash
  git tag v0.1.0
  git push origin v0.1.0
  ```

### Opening the macOS app

The `.app` only has an ad-hoc signature (no Apple developer account), so macOS
blocks the first launch:

- **macOS 14 or older:** right-click the app → **Open**.
- **macOS 15 or newer:** try to open it, then go to **System Settings → Privacy
  & Security → Open Anyway**.
- **"The app is damaged":** run `xattr -dr com.apple.quarantine /Applications/Discite.app`.

## 🌍 Translations

Every text Discite shows comes from [`locales/`](locales). **English** and
**Português (Brasil)** ship inside the executable. English is the default. Switch
with the language button in the title bar (**EN**) or in **Settings → Appearance
→ Language**. *Automatic* follows the system language. The change applies at once.

Adding a language takes no compiler:

1. Copy [`locales/en.xml`](locales/en.xml) and change `code` and `name`:
   ```xml
   <language code="es" name="Español">
     <string key="review.show_answer">Mostrar respuesta</string>
     <plural key="time.months_ago"><one>hace {n} mes</one><other>hace {n} meses</other></plural>
   </language>
   ```
2. Translate the text between the tags. Keep each `key` and every
   `{placeholder}`. A `<plural>` takes `<one>` (for 1) and `<other>`.
   `format.decimal` and `format.thousands` set the number separators.
3. Save it as `es.xml` in the `locales` folder inside the data folder
   (`%APPDATA%\Discite\locales\` or `~/Library/Application Support/Discite/locales/`),
   or in a `locales` folder next to the executable. Reopen Discite, and the
   language shows up in the menu.

Missing keys fall back to English, then to Portuguese, so a partial translation
still works. A file with the code of a built-in language overrides just the keys
it has. To ship a language inside the executable, add the file to `locales/`
and to `BUILT_IN` in [`src/i18n.rs`](src/i18n.rs). `cargo test` then checks
that it has every key the code uses.

## 🗂️ Project layout

```text
discite/
├── src/
│   ├── main.rs        entry point
│   ├── srs.rs         pure scheduler: FSRS, learning steps, fuzz, SM-2 (with tests)
│   ├── db.rs          SQLite schema, migrations and queries
│   ├── cards.rs       note types, cloze, occlusion masks, typed-answer diff
│   ├── rich.rs        inline markup
│   ├── search.rs      card search language → SQL
│   ├── stats.rs       statistics
│   ├── media.rs       image storage
│   ├── i18n.rs        translations loaded from locales/*.xml
│   └── gui/           one file per screen, plus theme.rs (design tokens) and widgets.rs
├── locales/           en.xml and pt-BR.xml: every text the app shows
├── assets/            logo.svg (also the app icon) and fonts
├── packaging/macos/   Info.plist for Discite.app
├── docs/images/       screenshots
├── .github/workflows/ build and release
├── build.rs           turns assets/logo.svg into the Windows .ico and the macOS .icns
└── SPEC.md            the original specification
```

Want a different icon? Replace `assets/logo.svg` and rebuild. The executable,
window and app icons are generated from it.

## 🙏 Credits

- [Literata](https://github.com/googlefonts/literata) by TypeTogether and
  [JetBrains Mono](https://www.jetbrains.com/lp/mono/) by JetBrains, both under
  the SIL Open Font License.
- Scheduling by [fsrs-rs](https://github.com/open-spaced-repetition/fsrs-rs),
  from the [Open Spaced Repetition](https://github.com/open-spaced-repetition)
  project.
- Built on [egui/eframe](https://github.com/emilk/egui),
  [rusqlite](https://github.com/rusqlite/rusqlite),
  [resvg](https://github.com/linebender/resvg) and
  [roxmltree](https://github.com/RazrFalcon/roxmltree).
- Inspired by [Anki](https://apps.ankiweb.net). Discite is an independent
  project, not affiliated with Anki.

<p align="center"><sub>Made with 🦀 and a lot of reviews rated <em>Good</em>.</sub></p>
