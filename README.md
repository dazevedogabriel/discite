# Discite

Caderno de estudos e flashcards com repetição espaçada, em Rust (eframe/egui), seguindo o design "Discite — Interface" e a especificação em [SPEC.md](SPEC.md).

## Rodar

```
cargo run --release
```

O executável fica em `target/release/discite.exe` (no macOS, `target/release/discite`), e não precisa de nada instalado ao lado dele.

- **Dados:** `%APPDATA%\Discite` no Windows e `~/Library/Application Support/Discite` no macOS (`discite.db` em SQLite, `media` com as imagens, `backups`). Para testar sem mexer nos seus dados, defina a variável de ambiente `DISCITE_DATA` com outra pasta.
- **Exemplo:** com o banco vazio, o Painel oferece "Carregar exemplo". Ele carrega as quatro disciplinas do design, com páginas, baralhos, notas e um histórico de revisões simulado. Com `DISCITE_DEMO=1`, o exemplo é carregado já na abertura.

## O que tem

**Caderno**
- Páginas e subpáginas por disciplina, com editor por blocos: parágrafo, títulos, listas (marcadores, numerada, checklist), citação, destaque, código, fórmula, tabela, divisor e imagem.
- Marcação viva: `**negrito**`, `*itálico*`, `__sublinhado__`, `~~tachado~~`, `` `código` ``, `==realce==`, `[[Link para página]]`. Ctrl+clique segue o link.
- Atalhos ao digitar: `# `, `## `, `- `, `1. `, `[] `, `> `, ```` ``` ````, `---`, além do menu `/`.
- Imagens: cole com Ctrl+V, arraste um arquivo para a janela ou use "Imagem". Clicar na imagem amplia.
- Salvamento automático, histórico de versões, backlinks, sumário, tags, busca de texto completo (Ctrl+K) e importação/exportação de Markdown.
- Criar flashcard (Ctrl+Shift+F) a partir da seleção. O cartão guarda o vínculo com a página de origem.

**Flashcards**
- Tipos de nota: Básico, Básico + invertido, Cloze (`{{c1::resposta::dica}}`, Ctrl+Shift+C), Digitar resposta e Oclusão de imagem (as máscaras são desenhadas sobre a imagem).
- Agendamento FSRS-6 pela crate `fsrs`, a mesma usada pelo Anki:
  - passos de aprendizado e reaprendizado;
  - ordem garantida Difícil < Bom < Fácil;
  - fuzz e balanceamento de carga;
  - limites diários e virada do dia configurável (04:00 por padrão);
  - irmãos enterrados e sanguessugas.
  - O SM-2 fica disponível como alternativa.
- Registro de cada resposta no `revlog`: nota, tempo, dias decorridos, estados, D/S antes e depois e R prevista. Desfazer (Ctrl+Z) não apaga nada, só marca a resposta.
- Presets com otimizador (treina os parâmetros no seu histórico e mostra log loss e RMSE antes/depois) e simulador de carga × retenção em 365 dias.
- Navegador de cartões com linguagem de busca: `deck:`, `subject:`, `tag:`, `is:due/new/learn/review/suspended/flagged/leech`, `prop:r<0.8`, `prop:d>7`, `lapses>2`, `added:7`, `rated:1:1`, `"texto"`, `-negação`. Inclui ações em lote e um painel por cartão com a curva de recuperabilidade e o histórico.
- Estudo personalizado: esquecidos, antecipar, novos recentes, R baixa, marcados ou qualquer busca.

**Painel e estatísticas**
- Tela "Hoje", com a estimativa de tempo; um card por disciplina; mapa de calor do ano; páginas recentes; previsão de 7 dias.
- Estatísticas:
  - revisões por dia por tipo, previsão de 30 dias e calibração (R prevista × observada);
  - contagem de cartões, frequência dos botões e acerto por hora;
  - distribuições de D, S e R;
  - conhecimento retido.

## Atalhos

| Onde | Tecla | Ação |
|---|---|---|
| Global | Ctrl K | Paleta de comandos |
| Global | Ctrl N | Nova página |
| Global | Ctrl Shift A | Adicionar nota |
| Global | Ctrl \\ | Recolher/expandir barra lateral |
| Global | Ctrl F | Buscar cartões |
| Painel | Espaço | Estudar tudo |
| Revisão | Espaço / Enter | Mostrar resposta, depois Bom |
| Revisão | 1 2 3 4 | Errei, Difícil, Bom, Fácil |
| Revisão | Ctrl Z | Desfazer |
| Revisão | E / I / O | Editar, Info, Origem |
| Revisão | @ / - / * | Suspender, Enterrar, Marcar |
| Caderno | Ctrl B / I / U / E | Negrito, itálico, sublinhado, código |
| Caderno | Ctrl Shift H | Realce |
| Caderno | Tab / Shift Tab | Recuar ou avançar na lista |
| Editor de nota | Ctrl Enter | Adicionar ou salvar |

## Builds e releases (GitHub Actions)

- **`.github/workflows/build.yml`:** compila e testa em Windows x64 e macOS universal (Apple Silicon + Intel, macOS 10.13 ou mais novo). Roda a cada push na `main`, em pull requests e manualmente (Actions → Build → Run workflow). Os pacotes ficam nos artefatos da execução:
  - `Discite-<versão>-windows-x64.zip` com o `Discite.exe`;
  - `Discite-<versão>-macos-universal.zip` com o `Discite.app`.
- **`.github/workflows/release.yml`:** publica uma release com os dois pacotes quando uma tag `v*` é enviada. A tag precisa bater com a `version` do `Cargo.toml`:

  ```
  git tag v0.1.0
  git push origin v0.1.0
  ```

O `.app` do macOS tem só assinatura ad hoc, sem conta de desenvolvedor Apple. Por isso o macOS bloqueia a primeira abertura:
- até o macOS 14: botão direito no app → **Abrir**;
- no macOS 15 ou mais novo: tente abrir e depois vá em **Ajustes do Sistema → Privacidade e Segurança → Abrir Mesmo Assim**;
- se aparecer "o app está danificado": `xattr -dr com.apple.quarantine /Applications/Discite.app`.

No macOS a janela usa a barra de título nativa e os atalhos aparecem com ⌘ no lugar de Ctrl.

## Código

| Arquivo | Conteúdo |
|---|---|
| `src/srs.rs` | Agendador puro (FSRS + passos + fuzz + SM-2), com testes |
| `src/db.rs` | Esquema SQLite, migrações e consultas |
| `src/cards.rs` | Tipos de nota, cloze, máscaras, diff da resposta digitada |
| `src/rich.rs` | Marcação inline |
| `src/search.rs` | Linguagem de busca → SQL |
| `src/stats.rs` | Estatísticas |
| `src/gui/` | Uma tela por arquivo, mais `theme.rs` (cores do design) e `widgets.rs` |

Testes: `cargo test`.
