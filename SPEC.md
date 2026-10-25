# Discite — Especificação do Produto

> *Discite* (latim): "aprendei". Aplicação desktop para centralizar estudos: anotações ricas e um sistema completo de flashcards com repetição espaçada baseada em evidência.

| Item | Valor |
|---|---|
| Versão do documento | 0.1 (rascunho) |
| Data | 2026-10-24 |
| Plataforma | Desktop (Windows primeiro; Linux/macOS depois) |
| Linguagem | Rust |
| Persistência | SQLite local (arquivo único) + pasta de mídia |
| Status | Especificação — nenhuma linha de código ainda |

---

## 1. Visão geral

O Discite tem **três pilares**, todos organizados por **Disciplina** (assunto):

1. **Caderno** — editor de texto para anotações, com formatação, imagens, tabelas, código e fórmulas.
2. **Flashcards** — um subsistema praticamente independente, com profundidade comparável ao Anki: tipos de cartão, agendamento por repetição espaçada (FSRS), registro completo de cada revisão, otimização de parâmetros e estatísticas.
3. **Painel** — visão do dia: o que revisar, quanto tempo vai levar, progresso por disciplina.

O usuário-alvo é um único estudante, offline, que quer controle total dos próprios dados.

### 1.1 Princípios

- **Local-first**: tudo funciona offline; os dados são do usuário, num formato aberto (SQLite).
- **Evidência acima de intuição**: o agendamento usa modelos validados empiricamente (FSRS), e as escolhas de UX favorecem práticas com suporte na literatura (prática de recuperação, espaçamento, intercalação).
- **Registro imutável de revisões**: o log de revisões é a fonte da verdade; o estado dos cartões pode ser reconstruído a partir dele.
- **Teclado primeiro**: toda ação frequente tem atalho; a sessão de revisão é 100% operável pelo teclado.
- **Rápido**: abrir o app e começar a revisar em menos de 1 segundo.

### 1.2 Fora de escopo (v1)

- Sincronização em nuvem / multi-dispositivo (prever no modelo de dados, não implementar).
- Colaboração ou compartilhamento.
- App mobile.
- IA generativa para criar cartões (possível módulo futuro).

---

## 2. Base científica

Esta seção justifica as decisões do subsistema de flashcards.

| Princípio | Evidência (resumo) | Como o Discite aplica |
|---|---|---|
| **Efeito de espaçamento** (Ebbinghaus 1885; Cepeda et al. 2006, meta-análise) | Distribuir revisões no tempo produz retenção de longo prazo muito superior a revisões concentradas. | Agendamento por repetição espaçada; intervalos crescentes. |
| **Prática de recuperação / testing effect** (Roediger & Karpicke 2006; Dunlosky et al. 2013) | Tentar lembrar ativamente é mais eficaz que reler. Dunlosky classifica *practice testing* e *distributed practice* como as duas técnicas de "alta utilidade". | A frente do cartão sempre exige recuperação antes de mostrar a resposta; modo "digitar resposta" opcional. |
| **Curva de esquecimento / memória de dois componentes** (Bjork & Bjork 1992; Ye et al. 2022, modelo DSR) | A memória pode ser descrita por *estabilidade* (quanto tempo dura) e *recuperabilidade* (probabilidade de lembrar agora). Revisar quando a recuperabilidade está mais baixa aumenta mais a estabilidade ("dificuldade desejável"). | Algoritmo **FSRS**, que modela Dificuldade, Estabilidade e Recuperabilidade. |
| **Intercalação** (Rohrer & Taylor 2007) | Misturar tipos de problema melhora discriminação e transferência. | Opção de sessão "misturada" entre baralhos de uma disciplina ou entre disciplinas. |
| **Elaboração / codificação dupla** (Paivio; Mayer) | Combinar texto e imagem melhora a codificação. | Cartões aceitam imagens; oclusão de imagem; vínculo cartão ↔ anotação de origem. |
| **Princípio da informação mínima** (Wozniak, "20 rules") | Cartões atômicos são lembrados com mais consistência. | Cloze com múltiplas lacunas gera um cartão por lacuna; dicas de UX ao criar cartões longos. |

### 2.1 Por que FSRS e não SM-2

- **SM-2** (SuperMemo, 1987; base do Anki clássico) usa um "fator de facilidade" heurístico, sem modelo explícito de probabilidade de lembrança, e sofre de *ease hell*.
- **FSRS** (Free Spaced Repetition Scheduler, Jarrett Ye et al., integrado ao Anki desde a 23.10) é treinado sobre centenas de milhões de revisões reais. Em benchmarks públicos (open-spaced-repetition/srs-benchmark) prediz a probabilidade de lembrança com erro significativamente menor que SM-2, exigindo **menos revisões para a mesma retenção**.
- FSRS permite ao usuário escolher diretamente a **retenção desejada** (ex.: 90%) e personalizar os parâmetros a partir do próprio histórico.

**Decisão**: FSRS é o agendador padrão. SM-2 é implementado como alternativa (para comparação e para usuários com pouco histórico que prefiram o comportamento clássico), selecionável por *preset*.

---

## 3. Conceitos e glossário

| Termo | Definição |
|---|---|
| **Disciplina** | Agrupador de topo (ex.: "Anatomia", "Direito Constitucional"). Tem cor e ícone. Contém cadernos e baralhos. |
| **Caderno** | Coleção de páginas de anotação dentro de uma disciplina. Hierárquico (páginas podem ter subpáginas). |
| **Página** | Documento editável composto por blocos (texto, imagem, tabela…). |
| **Baralho (Deck)** | Coleção de cartões dentro de uma disciplina. Hierárquico (`Anatomia::Membros::Superior`). |
| **Nota (Note)** | Unidade de conteúdo de flashcard: um conjunto de campos (Frente, Verso, Extra…). Uma nota gera **um ou mais cartões**. |
| **Tipo de nota (Note type)** | Define os campos e os modelos de cartão (templates) de uma nota. |
| **Cartão (Card)** | Item revisável individualmente, gerado a partir de uma nota + um template. Tem estado de agendamento próprio. |
| **Revisão (Review)** | Um evento de resposta a um cartão: nota dada, tempo gasto, estado antes/depois. |
| **Preset de opções** | Conjunto de configurações de agendamento compartilhado por um ou mais baralhos. |
| **Retenção desejada** | Probabilidade-alvo de lembrar um cartão no momento da revisão (padrão 0,90). |

---

## 4. Arquitetura técnica

### 4.1 Stack

| Camada | Escolha | Observação |
|---|---|---|
| GUI | **eframe / egui** | Imediato, nativo, rápido; tema customizado para visual moderno. |
| Banco | **rusqlite** (feature `bundled`) | SQLite embutido, WAL, `foreign_keys=ON`. |
| Busca | **SQLite FTS5** | Índice de texto completo para páginas e notas. |
| Agendamento | **crate `fsrs`** (open-spaced-repetition/fsrs-rs) | Mesma implementação usada pelo Anki; inclui o otimizador de parâmetros. |
| Migrações | `rusqlite_migration` ou migrações SQL versionadas próprias | `PRAGMA user_version`. |
| Datas | `chrono` / `jiff` | Todos os timestamps em UTC (ms); conversão para fuso local só na UI. |
| Imagens | `image`, `egui_extras` (loaders) | Decodificação e miniaturas. |
| Clipboard | `arboard` | Colar imagens direto da área de transferência. |
| Diálogos de arquivo | `rfd` | |
| IDs | `ulid` ou UUIDv7 | Ordenáveis por tempo; preparados para sync futuro. |
| Hash de mídia | `sha2` / `blake3` | Deduplicação de imagens. |
| Markdown / export | `pulldown-cmark` | Importar/exportar páginas em Markdown. |
| Fórmulas | Renderização LaTeX (ex.: crate de layout matemático ou rasterização em cache) | Pode entrar na v1.1. |

### 4.2 Estrutura de crates (workspace)

```
discite/
├── crates/
│   ├── discite-core/      # modelo de domínio, regras, sem GUI
│   ├── discite-db/        # schema, migrações, repositórios SQLite
│   ├── discite-srs/       # agendador (wrapper FSRS + SM-2), filas, limites diários
│   ├── discite-notes/     # modelo de blocos do editor, serialização, export Markdown
│   ├── discite-media/     # armazenamento e deduplicação de imagens
│   └── discite-app/       # binário eframe: telas, componentes, tema
└── SPEC.md
```

`discite-srs` não depende de GUI nem de banco: recebe estado + resposta, devolve novo estado. Isso permite testes determinísticos e simulações.

### 4.3 Armazenamento em disco

```
%APPDATA%\Discite\            (sobrescrevível por DISCITE_DATA)
├── discite.db                # SQLite (WAL)
├── media\                    # imagens, nomeadas pelo hash: ab\ab12cd…ef.png
├── backups\                  # cópias automáticas do .db (rotação)
└── settings.toml             # preferências de UI (tema, fonte, atalhos)
```

- Imagens ficam **fora** do banco (arquivos endereçados por conteúdo) e são referenciadas pela tabela `media`.
- Backup automático: ao fechar o app e a cada N horas de uso, via `VACUUM INTO`. Manter os últimos 10 diários + 4 semanais.
- Verificação de integridade (`PRAGMA integrity_check`) e limpeza de mídia órfã sob demanda.

---

## 5. Módulo Caderno (anotações)

### 5.1 Requisitos funcionais

| ID | Requisito |
|---|---|
| N-01 | Criar, renomear, mover, arquivar e excluir páginas dentro de um caderno de uma disciplina. |
| N-02 | Páginas hierárquicas (subpáginas), reordenáveis por arrastar. |
| N-03 | Editor por **blocos**: parágrafo, títulos (H1–H3), listas (marcadores, numerada, checklist), citação, bloco de código (com destaque de sintaxe), tabela, divisor, callout (info/aviso/dica), imagem, fórmula (LaTeX), toggle (seção recolhível). |
| N-04 | Formatação inline: negrito, itálico, sublinhado, tachado, código inline, realce (cores), link. |
| N-05 | Atalhos estilo Markdown durante a digitação (`# ` → título, `- ` → lista, `**x**` → negrito, ` ``` ` → código, `/` → menu de blocos). |
| N-06 | **Imagens**: inserir por colar (Ctrl+V), arrastar arquivo, ou diálogo. Redimensionar, alinhar, legenda. Clique abre em visualizador com zoom. |
| N-07 | Salvamento automático contínuo (debounce ~500 ms); histórico de versões por página (snapshots periódicos). |
| N-08 | Busca de texto completo em todas as páginas (FTS5), com filtro por disciplina. |
| N-09 | Links internos entre páginas (`[[Nome da página]]`) com backlinks. |
| N-10 | **Ponte com flashcards**: selecionar texto (ou imagem) → "Criar flashcard" abre o editor de nota pré-preenchido e guarda o vínculo com a página/bloco de origem. |
| N-11 | Exportar página para Markdown (+ pasta de imagens) e PDF (v1.1). Importar Markdown. |
| N-12 | Tags em páginas. |
| N-13 | Modo foco (esconde barras laterais) e largura de leitura configurável. |

### 5.2 Modelo do documento

Uma página é uma lista ordenada de blocos. Cada bloco é armazenado como JSON tipado (`serde`) na tabela `note_blocks`, o que permite: editar um bloco sem reescrever a página, referenciar blocos individualmente (vínculo com flashcards) e indexar o texto puro no FTS.

```jsonc
{ "type": "paragraph", "spans": [ {"text": "A ", "marks": []}, {"text": "mitose", "marks": ["bold"]} ] }
{ "type": "image", "media_id": "01J…", "width": 480, "align": "center", "caption": "Fases da mitose" }
{ "type": "code", "lang": "rust", "text": "fn main() {}" }
```

> **Risco técnico**: egui não tem um editor rich-text pronto. O editor de blocos será um componente próprio (cada bloco de texto é um `TextEdit` com `LayoutJob` customizado para os spans). Este é o maior item de implementação do módulo e deve ser prototipado cedo.

---

## 6. Módulo Flashcards

### 6.1 Tipos de nota (embutidos)

| Tipo | Campos | Cartões gerados |
|---|---|---|
| **Básico** | Frente, Verso, Extra | 1 (Frente → Verso) |
| **Básico + invertido** | Frente, Verso, Extra | 2 (Frente → Verso, Verso → Frente) |
| **Invertido opcional** | Frente, Verso, Extra, "Inverter?" | 1 ou 2 |
| **Digitar resposta** | Frente, Verso | 1; usuário digita e o app mostra diff caractere a caractere |
| **Cloze** | Texto, Extra | 1 por lacuna: `{{c1::mitocôndria}}`, `{{c2::ATP::dica}}` |
| **Oclusão de imagem** | Imagem, Máscaras, Cabeçalho, Extra | 1 por máscara (modo "esconder uma" ou "esconder todas, revelar uma") |

- Usuário pode criar **tipos de nota próprios**: definir campos e templates de frente/verso com placeholders (`{{Campo}}`, `{{FrontSide}}`, condicionais `{{#Campo}}…{{/Campo}}`).
- Todos os campos aceitam texto formatado e imagens (mesmo motor do caderno, versão simplificada).
- Campo opcional de **áudio** fica para versão futura.

### 6.2 Estados do cartão

```
          ┌────────┐  1ª resposta   ┌───────────┐  conclui passos  ┌────────┐
          │  Novo  │──────────────▶│ Aprendendo │────────────────▶│ Revisão │◀─┐
          └────────┘                └───────────┘                  └────────┘  │
                                                                     │ Errei  │
                                                                     ▼        │
                                                               ┌─────────────┐│
                                                               │Reaprendendo ├┘
                                                               └─────────────┘
Estados ortogonais: Suspenso · Enterrado (até amanhã) · Sanguessuga (leech, marcado)
```

### 6.3 Respostas

Quatro botões, como no Anki e exigidos pelo FSRS:

| Botão | Atalho | Significado |
|---|---|---|
| **Errei** (Again) | `1` | Não lembrei / lembrei errado. |
| **Difícil** (Hard) | `2` | Lembrei com esforço significativo. |
| **Bom** (Good) | `3` / `Espaço` | Lembrei com esforço normal. |
| **Fácil** (Easy) | `4` | Lembrei sem esforço. |

Cada botão exibe o **próximo intervalo previsto** (ex.: "10 min", "3 d", "2,1 meses") antes do clique.

### 6.4 Agendamento (FSRS)

O núcleo usa a crate `fsrs` como **fonte da verdade** para as fórmulas; a especificação abaixo documenta o modelo para referência e testes.

**Variáveis por cartão**
- `D` — Dificuldade, ∈ [1, 10].
- `S` — Estabilidade, em dias: intervalo no qual R cai para 90%.
- `R` — Recuperabilidade: probabilidade de lembrar agora (calculada, não armazenada).

**Curva de esquecimento** (FSRS-6; no FSRS-4.5/5 `decay = −0,5` fixo):

```
R(t, S) = (1 + FACTOR · t / S) ^ (−decay)        FACTOR = 0,9 ^ (−1/decay) − 1
```

**Intervalo** para uma retenção desejada `r`:

```
I(r, S) = (S / FACTOR) · (r ^ (1/(−decay)) − 1)    → com r = 0,9, I = S
```

**Após cada resposta** (G ∈ {1,2,3,4}):
- 1ª resposta: `S₀ = w[G−1]`, `D₀(G) = w4 − e^(w5·(G−1)) + 1`.
- Dificuldade: `D' = D − w6·(G−3)`, amortecida linearmente `·(10−D)/9`, com reversão à média em direção a `D₀(4)` (peso `w7`).
- Sucesso (G ≥ 2): `S' = S · (1 + e^w8 · (11−D) · S^(−w9) · (e^(w10·(1−R)) − 1) · penalidadeDifícil · bônusFácil)`.
- Lapso (G = 1): `S' = min(S, w11 · D^(−w12) · ((S+1)^w13 − 1) · e^(w14·(1−R)))`.
- Revisão no mesmo dia (passos curtos): `S' = S · e^(w17·(G−3+w18)) · S^(−w19)`.

**Comportamentos de agendamento**

| Recurso | Descrição | Padrão |
|---|---|---|
| Retenção desejada | Por preset, 0,70–0,99. UI mostra estimativa de carga diária para o valor escolhido. | 0,90 |
| Passos de aprendizado | Intervalos curtos antes do primeiro intervalo FSRS. | `1m 10m` |
| Passos de reaprendizado | Após lapso. | `10m` |
| Intervalo máximo | Teto em dias. | 36 500 |
| Fuzz | Aleatoriedade de ±~5% no intervalo para evitar aglomeração de cartões criados juntos. | Ligado |
| Balanceamento de carga | Dentro da janela de fuzz, escolher o dia com menos revisões previstas. | Ligado |
| Otimizador | Treina os parâmetros `w` a partir do `review_log` do preset (crate `fsrs`). Disponível a partir de ~400 revisões; sugerido mensalmente. Mostra *log loss* e RMSE antes/depois. | Manual + lembrete |
| Simulador | Simula N dias de estudo para comparar retenções desejadas (carga × conhecimento retido). | — |
| Reagendar ao mudar parâmetros | Recalcula `due` de todos os cartões a partir do histórico. | Pergunta ao usuário |

**SM-2 (alternativo)**: implementação clássica (fator de facilidade inicial 2,5; Errei −0,20; Difícil −0,15 e ×1,2; Fácil +0,15 e bônus 1,3). Selecionável por preset.

### 6.5 Filas e sessão diária

| Recurso | Descrição | Padrão |
|---|---|---|
| Virada do dia | Hora em que o "dia de estudo" muda (evita quebrar à meia-noite). | 04:00 local |
| Limite de novos/dia | Por preset; herda pela hierarquia de baralhos. | 20 |
| Limite de revisões/dia | Por preset. | 200 |
| Ordem de novos | Ordem de criação / aleatória / por posição. | Criação |
| Ordem das revisões | Vencimento / recuperabilidade crescente (mais "em risco" primeiro) / aleatória / intercalada por baralho. | Recuperabilidade |
| Mistura | Novos intercalados com revisões, antes ou depois. | Intercalado |
| Enterrar irmãos | Cartões da mesma nota não aparecem no mesmo dia. | Ligado |
| Sanguessuga (leech) | Após N lapsos: marcar e/ou suspender automaticamente; listar para reescrita. | 8 lapsos, marcar |
| Atrasos | Cartões vencidos há muito são tratados corretamente pelo FSRS (R real usado no cálculo). | — |

### 6.6 Sessão de revisão (UX)

- Tela limpa, centralizada, sem distrações. Mostra disciplina/baralho, contadores **Novos · Aprendendo · Revisão** restantes.
- Fluxo: mostra frente → usuário pensa → `Espaço` revela → avalia (1–4).
- Desfazer última resposta (`Ctrl+Z`) — reverte estado e marca o log como desfeito.
- Ações durante a revisão: editar nota (`E`), suspender (`@`), enterrar (`-`), marcar (`*`), ver origem no caderno (`O`), ver histórico do cartão (`I`).
- Cronômetro por cartão (registrado no log; teto configurável, ex.: 60 s, para não distorcer estatísticas quando o usuário se distrai).
- Fim da sessão: resumo (cartões vistos, % acerto, tempo, próxima carga prevista).

### 6.7 Estudo personalizado

- **Sessão filtrada**: por busca (tag, disciplina, "esquecidos hoje", "R < 70%", "marcados"), sem ou com efeito no agendamento.
- **Revisão antecipada / atrasada**: estudar antes do vencimento (registrado como tipo `manual`/`filtrado`, que o FSRS trata corretamente).
- **Sessão intercalada**: misturar baralhos de várias disciplinas.

### 6.8 Navegador de cartões

- Tabela pesquisável de notas/cartões: colunas configuráveis (campo, baralho, vencimento, intervalo, D, S, R atual, lapsos, revisões, criado em).
- Linguagem de busca: `deck:Anatomia tag:prova is:due prop:r<0.8 lapses>3 added:7 "texto exato"`.
- Ações em lote: mover de baralho, adicionar/remover tags, suspender, reposicionar novos, redefinir, excluir, mudar tipo de nota.
- Painel lateral com edição da nota e **histórico do cartão** (cada revisão: data, resposta, intervalo, D/S/R naquele momento) e gráfico da R prevista.

### 6.9 Importação/Exportação

- Importar **CSV/TSV** (mapear colunas → campos).
- Importar **Anki `.apkg` / `.colpkg`** incluindo histórico de revisões (v1.1) — permite migrar e já otimizar o FSRS.
- Exportar baralho para formato próprio (`.discite`, um SQLite + mídia zipados) e CSV.

---

## 7. Painel e estatísticas

### 7.1 Painel inicial

- Saudação + **"Hoje"**: total vencido, novos disponíveis, tempo estimado (baseado no tempo médio por cartão do usuário), botão grande **"Estudar"**.
- Cartões por disciplina com: vencidos hoje, retenção dos últimos 30 dias, sequência de dias estudados.
- Atalhos para páginas editadas recentemente.
- Heatmap anual de revisões (estilo GitHub).

### 7.2 Estatísticas (por disciplina, baralho, preset ou global)

| Gráfico | Descrição |
|---|---|
| Previsão | Revisões vencendo nos próximos 7/30/365 dias. |
| Revisões por dia | Barras empilhadas por tipo (aprendizado, revisão, reaprendizado, filtrado), com tempo gasto. |
| Retenção real | % de acertos em revisões maduras (intervalo ≥ 21 d) vs. retenção desejada. |
| Calibração | R prevista × taxa de acerto observada (verifica se o modelo está bem ajustado). |
| Distribuição de D, S e R | Histogramas. |
| Intervalos | Distribuição dos intervalos atuais. |
| Contagem de cartões | Novos / aprendendo / jovens / maduros / suspensos. |
| Horário | Taxa de acerto por hora do dia. |
| Botões | Frequência de cada resposta por tipo de cartão. |
| Conhecimento retido estimado | Σ R de todos os cartões ao longo do tempo. |

---

## 8. Modelo de dados (SQLite)

Convenções: IDs `TEXT` (ULID); timestamps `INTEGER` em ms UTC; `deleted_at` para exclusão lógica (preparação para sync); `updated_at` em todas as tabelas mutáveis.

```sql
-- ===== Organização =====
CREATE TABLE subjects (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  color       TEXT NOT NULL,           -- hex
  icon        TEXT,
  position    INTEGER NOT NULL,
  archived    INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER
);

CREATE TABLE tags (
  id   TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

-- ===== Caderno =====
CREATE TABLE pages (
  id          TEXT PRIMARY KEY,
  subject_id  TEXT NOT NULL REFERENCES subjects(id),
  parent_id   TEXT REFERENCES pages(id),
  title       TEXT NOT NULL,
  icon        TEXT,
  position    INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER
);

CREATE TABLE page_blocks (
  id          TEXT PRIMARY KEY,
  page_id     TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  position    REAL NOT NULL,           -- fracionário: inserir entre blocos sem renumerar
  kind        TEXT NOT NULL,           -- paragraph, heading, image, code, table...
  content     TEXT NOT NULL,           -- JSON tipado
  plain_text  TEXT NOT NULL,           -- para FTS
  updated_at  INTEGER NOT NULL
);

CREATE TABLE page_versions (
  id          TEXT PRIMARY KEY,
  page_id     TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  snapshot    BLOB NOT NULL,           -- JSON comprimido (zstd) da página inteira
  created_at  INTEGER NOT NULL
);

CREATE TABLE page_links (               -- [[links]] internos
  from_page TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  to_page   TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  PRIMARY KEY (from_page, to_page)
);

CREATE TABLE page_tags (
  page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
  tag_id  TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (page_id, tag_id)
);

-- ===== Mídia =====
CREATE TABLE media (
  id          TEXT PRIMARY KEY,
  hash        TEXT NOT NULL UNIQUE,    -- blake3 do conteúdo
  file_name   TEXT NOT NULL,           -- caminho relativo em media/
  mime        TEXT NOT NULL,
  width       INTEGER,
  height      INTEGER,
  size_bytes  INTEGER NOT NULL,
  created_at  INTEGER NOT NULL
);

-- ===== Flashcards: configuração =====
CREATE TABLE presets (                  -- opções de agendamento
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  algorithm          TEXT NOT NULL DEFAULT 'fsrs',   -- 'fsrs' | 'sm2'
  desired_retention  REAL NOT NULL DEFAULT 0.9,
  fsrs_params        TEXT,             -- JSON array de floats (w); NULL = padrão
  fsrs_version       TEXT,             -- ex.: 'fsrs-6'
  learning_steps     TEXT NOT NULL DEFAULT '[60,600]',  -- segundos
  relearning_steps   TEXT NOT NULL DEFAULT '[600]',
  new_per_day        INTEGER NOT NULL DEFAULT 20,
  reviews_per_day    INTEGER NOT NULL DEFAULT 200,
  max_interval_days  INTEGER NOT NULL DEFAULT 36500,
  leech_threshold    INTEGER NOT NULL DEFAULT 8,
  leech_action       TEXT NOT NULL DEFAULT 'tag',      -- 'tag' | 'suspend'
  bury_siblings      INTEGER NOT NULL DEFAULT 1,
  new_order          TEXT NOT NULL DEFAULT 'created',
  review_order       TEXT NOT NULL DEFAULT 'retrievability',
  fuzz               INTEGER NOT NULL DEFAULT 1,
  load_balance       INTEGER NOT NULL DEFAULT 1,
  last_optimized_at  INTEGER,
  updated_at         INTEGER NOT NULL
);

CREATE TABLE decks (
  id          TEXT PRIMARY KEY,
  subject_id  TEXT NOT NULL REFERENCES subjects(id),
  parent_id   TEXT REFERENCES decks(id),
  preset_id   TEXT NOT NULL REFERENCES presets(id),
  name        TEXT NOT NULL,
  description TEXT,
  position    INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER
);

CREATE TABLE note_types (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL,           -- 'standard' | 'cloze' | 'image_occlusion'
  fields      TEXT NOT NULL,           -- JSON: [{name, ord, rich, sticky}]
  templates   TEXT NOT NULL,           -- JSON: [{name, ord, front, back}]
  css         TEXT,
  builtin     INTEGER NOT NULL DEFAULT 0,
  updated_at  INTEGER NOT NULL
);

-- ===== Flashcards: conteúdo =====
CREATE TABLE notes (
  id            TEXT PRIMARY KEY,
  note_type_id  TEXT NOT NULL REFERENCES note_types(id),
  fields        TEXT NOT NULL,         -- JSON: {"Frente": <rich>, "Verso": <rich>}
  sort_field    TEXT NOT NULL,         -- texto puro do 1º campo (ordenação/busca)
  checksum      INTEGER NOT NULL,      -- detecção de duplicatas
  source_page   TEXT REFERENCES pages(id),        -- vínculo com o caderno
  source_block  TEXT REFERENCES page_blocks(id),
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  deleted_at    INTEGER
);

CREATE TABLE note_tags (
  note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag_id  TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (note_id, tag_id)
);

CREATE TABLE cards (
  id              TEXT PRIMARY KEY,
  note_id         TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  deck_id         TEXT NOT NULL REFERENCES decks(id),
  template_ord    INTEGER NOT NULL,    -- qual template / qual lacuna cloze
  state           TEXT NOT NULL,       -- 'new' | 'learning' | 'review' | 'relearning'
  queue           TEXT NOT NULL,       -- 'normal' | 'suspended' | 'buried_user' | 'buried_sibling'
  due             INTEGER NOT NULL,    -- ms UTC (novos: posição na fila)
  step            INTEGER NOT NULL DEFAULT 0,  -- índice no passo de (re)aprendizado
  stability       REAL,                -- FSRS S (dias)
  difficulty      REAL,                -- FSRS D
  ease_factor     REAL,                -- SM-2
  interval_days   REAL NOT NULL DEFAULT 0,
  reps            INTEGER NOT NULL DEFAULT 0,
  lapses          INTEGER NOT NULL DEFAULT 0,
  last_review_at  INTEGER,
  flagged         INTEGER NOT NULL DEFAULT 0,
  leech           INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  UNIQUE (note_id, template_ord)
);
CREATE INDEX idx_cards_due  ON cards(deck_id, queue, state, due);
CREATE INDEX idx_cards_note ON cards(note_id);

-- ===== Flashcards: histórico (append-only) =====
CREATE TABLE review_log (
  id                TEXT PRIMARY KEY,
  card_id           TEXT NOT NULL REFERENCES cards(id),
  reviewed_at       INTEGER NOT NULL,   -- ms UTC
  rating            INTEGER NOT NULL,   -- 1 Errei, 2 Difícil, 3 Bom, 4 Fácil
  review_kind       TEXT NOT NULL,      -- 'learning'|'review'|'relearning'|'filtered'|'manual'
  elapsed_days      REAL NOT NULL,      -- desde a revisão anterior
  scheduled_days    REAL NOT NULL,      -- intervalo que havia sido agendado
  duration_ms       INTEGER NOT NULL,   -- tempo para responder (já com teto)
  state_before      TEXT NOT NULL,
  state_after       TEXT NOT NULL,
  stability_before  REAL, stability_after  REAL,
  difficulty_before REAL, difficulty_after REAL,
  retrievability    REAL,               -- R prevista no momento da revisão
  next_due          INTEGER NOT NULL,
  algorithm         TEXT NOT NULL,      -- 'fsrs-6' | 'sm2'
  preset_id         TEXT NOT NULL,
  undone            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_revlog_card ON review_log(card_id, reviewed_at);
CREATE INDEX idx_revlog_time ON review_log(reviewed_at);

CREATE TABLE study_sessions (
  id          TEXT PRIMARY KEY,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  scope       TEXT NOT NULL,           -- JSON: disciplinas/baralhos/filtro
  cards_seen  INTEGER NOT NULL DEFAULT 0
);

-- ===== Busca =====
CREATE VIRTUAL TABLE fts_pages USING fts5(block_id UNINDEXED, page_id UNINDEXED, text, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE fts_notes USING fts5(note_id UNINDEXED, text, tokenize='unicode61 remove_diacritics 2');

-- ===== Meta =====
CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

**Regras de integridade**
- `review_log` é **somente-inserção**; "desfazer" apenas marca `undone = 1` e restaura o cartão a partir de `*_before`.
- Toda resposta grava `review_log` e atualiza `cards` na **mesma transação**.
- Existe uma rotina "reconstruir agendamento" que reprocessa todo o `review_log` de um cartão com os parâmetros atuais.

---

## 9. Interface (para o design)

### 9.1 Identidade visual

- **Tom**: acadêmico-moderno — sóbrio, legível, calmo. Referências: Linear, Notion, Obsidian, Things 3.
- **Tema**: claro e escuro (seguir sistema por padrão). Escuro não é preto puro (ex.: `#16171B` fundo, `#1E2026` superfície).
- **Cor de destaque**: um tom único (sugestão: azul-índigo ou verde-musgo), mais a **cor de cada disciplina** usada em chips, barras laterais e gráficos.
- **Cores semânticas das respostas**: Errei = vermelho, Difícil = âmbar, Bom = verde, Fácil = azul. Consistentes em botões, histórico e gráficos.
- **Tipografia**: sans moderna para UI (Inter, IBM Plex Sans); serifada opcional para leitura de páginas (Literata, Source Serif); mono para código (JetBrains Mono).
- **Forma**: cantos arredondados (8–12 px), sombras sutis, bordas de 1 px de baixo contraste, bastante espaço em branco.
- **Movimento**: transições curtas (120–180 ms); virar cartão com fade/slide discreto, nunca chamativo.
- Logotipo: palavra "Discite" ou monograma "D"; motivo opcional: coruja/ramo de oliveira, minimalista.

### 9.2 Layout geral

```
┌───────────────────────────────────────────────────────────────────────┐
│ ● Discite                                    [🔍 Buscar  Ctrl+K]  ⚙  │
├──────────────┬────────────────────────────────────────────────────────┤
│ 🏠 Painel    │                                                        │
│ 📈 Estatíst. │                                                        │
│ 🗂 Cartões   │                ÁREA PRINCIPAL                          │
│──────────────│                                                        │
│ DISCIPLINAS  │                                                        │
│ ● Anatomia 12│                                                        │
│   ▸ Caderno  │                                                        │
│   ▸ Baralhos │                                                        │
│ ● Farmaco  34│                                                        │
│ ● Direito   0│                                                        │
│ + Disciplina │                                                        │
└──────────────┴────────────────────────────────────────────────────────┘
```

- Barra lateral recolhível; número ao lado da disciplina = cartões vencidos hoje.
- **Paleta de comandos** (`Ctrl+K`): navegar para qualquer página/baralho, executar ações ("Novo cartão", "Estudar Anatomia", "Otimizar parâmetros").

### 9.3 Telas

1. **Painel (Home)** — card "Hoje" em destaque (vencidos, novos, tempo estimado, botão **Estudar tudo**); grade de disciplinas com mini-métricas; heatmap; páginas recentes.
2. **Disciplina** — cabeçalho com cor/ícone; abas **Caderno · Baralhos · Estatísticas**; resumo de retenção e carga.
3. **Caderno / Página** — árvore de páginas à esquerda (dentro da disciplina), editor central com largura de leitura, painel direito opcional com sumário (títulos), backlinks e cartões vinculados a esta página.
4. **Lista de baralhos** — árvore com colunas Novos / Aprendendo / Revisão; ação "Estudar" por linha; menu de opções do preset.
5. **Sessão de revisão** — modo foco: cartão centralizado, contadores no topo, barra de progresso, botões de resposta com intervalos previstos; tela de resumo ao final.
6. **Editor de nota (Adicionar/Editar)** — seletor de tipo de nota e baralho; campos ricos; pré-visualização ao vivo de frente/verso; para cloze, botão "nova lacuna" (`Ctrl+Shift+C`); para oclusão, editor de máscaras sobre a imagem; aviso de duplicata.
7. **Navegador de cartões** — busca no topo, tabela densa, painel lateral de edição + histórico/gráfico de R.
8. **Opções do preset** — retenção desejada com slider e estimativa de carga, passos, limites, ordem, botão **Otimizar** com relatório, **Simulador**.
9. **Estatísticas** — filtros (escopo, período) e grade de gráficos da seção 7.2.
10. **Configurações** — aparência, fonte, virada do dia, atalhos, backup/restauração, pasta de dados, importar/exportar.

### 9.4 Estados vazios e microcópia

- Disciplina sem nada: "Comece criando uma página ou um baralho."
- Sem cartões para hoje: "Tudo em dia. Próximas revisões: 34 amanhã."
- Otimizador sem dados suficientes: "São necessárias cerca de 400 revisões neste preset (você tem 132)."

---

## 10. Atalhos de teclado (padrão)

| Contexto | Atalho | Ação |
|---|---|---|
| Global | `Ctrl+K` | Paleta de comandos |
| Global | `Ctrl+N` | Nova página |
| Global | `Ctrl+Shift+A` | Adicionar flashcard |
| Global | `Ctrl+B` (barra) / `Ctrl+\` | Alternar barra lateral |
| Global | `Ctrl+F` | Buscar |
| Editor | `/` | Menu de blocos |
| Editor | `Ctrl+Shift+F` | Criar flashcard da seleção |
| Revisão | `Espaço` | Mostrar resposta / Bom |
| Revisão | `1` `2` `3` `4` | Errei / Difícil / Bom / Fácil |
| Revisão | `Ctrl+Z` | Desfazer |
| Revisão | `E` / `I` / `O` | Editar / Info do cartão / Abrir origem |
| Revisão | `@` / `-` / `*` | Suspender / Enterrar / Marcar |

Todos os atalhos são reconfiguráveis.

---

## 11. Requisitos não funcionais

| ID | Requisito |
|---|---|
| NF-01 | Inicialização a frio < 1 s com 50 000 cartões e 2 000 páginas. |
| NF-02 | Mostrar próximo cartão < 50 ms após responder. |
| NF-03 | UI a 60 fps; nenhuma operação de I/O ou otimização na thread de UI (otimizador roda em thread de fundo com barra de progresso). |
| NF-04 | Nenhuma perda de dados em fechamento abrupto (SQLite WAL, transações curtas, autosave). |
| NF-05 | Determinismo: dado o mesmo `review_log` e parâmetros, o agendamento é reproduzível (fuzz com semente derivada do ID do cartão + nº de revisões). |
| NF-06 | Acessibilidade: escala de fonte, alto contraste, navegação completa por teclado. |
| NF-07 | Internacionalização: interface em pt-BR na v1, strings externalizadas (ex.: `fluent`) para outros idiomas. |
| NF-08 | Binário único, sem dependências externas de runtime. |

---

## 12. Testes

- **discite-srs**: testes unitários das fórmulas contra vetores de referência da crate `fsrs`; testes de propriedade (intervalos monotônicos com a nota, R ∈ [0,1], D ∈ [1,10]); simulação de 1 ano com usuário sintético para validar carga e retenção.
- **discite-db**: migrações aplicadas do zero e incrementalmente; reconstrução de estado a partir do `review_log` = estado salvo.
- **Importação Anki**: deck de exemplo com histórico; comparar intervalos com os do Anki usando os mesmos parâmetros.
- **UI**: testes de fumaça automatizados (abrir, criar disciplina, criar nota, revisar, fechar) com diretório isolado via `DISCITE_DATA`.

---

## 13. Roadmap

| Fase | Entregas |
|---|---|
| **M0 — Fundação** | Workspace, schema + migrações, disciplinas, tema claro/escuro, layout com barra lateral. |
| **M1 — Flashcards núcleo** | Tipos Básico/Invertido/Cloze, baralhos, presets, FSRS com parâmetros padrão, sessão de revisão, review_log, desfazer. |
| **M2 — Caderno** | Editor de blocos (texto, títulos, listas, código, imagem colada/arrastada), autosave, árvore de páginas, busca FTS. |
| **M3 — Integração** | Criar cartão da seleção, vínculo origem ↔ cartão, backlinks, paleta de comandos. |
| **M4 — Profundidade SRS** | Otimizador, simulador, navegador de cartões com linguagem de busca, sanguessugas, estudo filtrado, estatísticas completas. |
| **M5 — Ecossistema** | Oclusão de imagem, digitar resposta, tipos de nota customizados, importação Anki/CSV, export Markdown/PDF, LaTeX, backups com restauração. |
| **Futuro** | Sync opcional (dados já preparados com ULID + `updated_at` + `deleted_at`), áudio, app mobile, geração de cartões assistida por IA. |

---

## 14. Questões em aberto

1. Imagens dentro do banco (BLOB) vs. pasta `media/` — esta spec adota a pasta; reavaliar se backup único em um arquivo for prioridade.
2. Editor de blocos próprio em egui vs. incorporar um webview só para o editor — esta spec adota egui puro; validar com protótipo em M0.
3. Fórmulas LaTeX: renderização nativa vs. rasterização em cache.
4. Um cartão pode pertencer a uma disciplina diferente da página de origem? (Proposta: sim, o vínculo é só referência.)
5. Meta diária / gamificação (sequências): incluir de forma discreta ou não incluir?

---

## Referências

- Ebbinghaus, H. (1885). *Über das Gedächtnis*.
- Cepeda, N. J. et al. (2006). Distributed practice in verbal recall tasks: A review and quantitative synthesis. *Psychological Bulletin*.
- Roediger, H. L. & Karpicke, J. D. (2006). Test-enhanced learning. *Psychological Science*.
- Dunlosky, J. et al. (2013). Improving students' learning with effective learning techniques. *Psychological Science in the Public Interest*.
- Rohrer, D. & Taylor, K. (2007). The shuffling of mathematics problems improves learning. *Instructional Science*.
- Bjork, R. A. & Bjork, E. L. (1992). A new theory of disuse and an old theory of stimulus fluctuation.
- Ye, J., Su, J. & Cao, Y. (2022). A stochastic shortest path algorithm for optimizing spaced repetition scheduling. *KDD '22*.
- Wozniak, P. (1999). *Effective learning: Twenty rules of formulating knowledge*.
- open-spaced-repetition: `fsrs-rs`, `srs-benchmark`, wiki "The Algorithm" (github.com/open-spaced-repetition).

---

## 15. Estado da implementação (v0.1)

Implementado em `C:\Users\Gabriel\Claude\discite` seguindo o canvas "Discite — Interface". As diferenças em relação a esta especificação são estas:

- **Estrutura:** é uma crate só, com módulos (`srs`, `db`, `cards`, `rich`, `search`, `stats`, `gui/*`), e não um workspace com várias crates. O agendador continua puro e testável.
- **IDs:** inteiros derivados do relógio em milissegundos (sempre crescentes, como no Anki), e não ULIDs em texto. Continuam ordenáveis e preparados para sincronização.
- **Tags:** ficam como texto separado por espaços em `notes.tags` e `pages.tags`, sem tabelas de junção.
- **Texto rico:** usa marcação inline (`**negrito**`, `[[link]]`…) com marcadores visíveis só no bloco em edição.
- **Restauração de backup:** aplicada na próxima abertura (`restore.pending`). O banco anterior vai para `backups/`.

Ainda não implementado:

- importação de `.apkg` do Anki (só CSV);
- tipos de nota personalizados (só os cinco embutidos);
- renderização de LaTeX (a fórmula aparece como código-fonte);
- exportação em PDF (só Markdown);
- desfazer entre blocos no caderno (cada bloco tem o próprio Ctrl+Z);
- sincronização.
