Você vai gerar flashcards e/ou questões de múltipla escolha para eu importar no Discite, meu app de estudo com repetição espaçada. Responda com UM único objeto JSON válido, sem comentários e sem texto fora dele (pode vir dentro de um bloco ```json).

## Formato

```json
{
  "cards": [
    {"type": "basic", "front": "Pergunta", "back": "Resposta", "extra": "Contexto opcional", "tags": "tema subtema"},
    {"type": "cloze", "text": "Frase com {{c1::lacuna}} e {{c2::outra lacuna::dica}}.", "extra": "Contexto opcional", "tags": "tema"},
    {"type": "reversed", "front": "Termo", "back": "Definição"}
  ],
  "questions": [
    {
      "stem": "Enunciado da questão",
      "options": ["Alternativa A", "Alternativa B", "Alternativa C", "Alternativa D"],
      "answer": "B",
      "explanation": "Por que a B está certa e por que cada uma das outras está errada."
    }
  ]
}
```

- `cards` e `questions` são opcionais: mande só o que eu pedir.
- Tipos de cartão:
  - `basic`: frente e verso.
  - `cloze`: texto com lacunas `{{c1::resposta}}`. Cada número (c1, c2…) vira um cartão; lacunas com o mesmo número aparecem juntas. Uma dica opcional vai depois de `::` (`{{c1::resposta::dica}}`).
  - `reversed`: frente e verso que também geram o cartão inverso (verso → frente). Use só para pares que fazem sentido nos dois sentidos.
- `extra` (opcional) aparece no verso, depois da resposta.
- `tags` (opcional): palavras separadas por espaço; uma tag não pode ter espaço (use hífen).
- Cada questão tem exatamente 4 alternativas e só uma certa. `answer` é a letra da certa: "A", "B", "C" ou "D".

## Formatação dentro dos textos

- `**negrito**`, `*itálico*`, `__sublinhado__`, `~~tachado~~`, `` `código` ``, `==realce==`.
- Cor: `{red|texto}`. Realce colorido: `{=yellow|texto}`. Cores: red, orange, yellow, green, blue, purple, gray.
- Quebra de linha: `\n` dentro da string JSON. Aspas dentro do texto: `\"`.
- Não use HTML, LaTeX nem Markdown de títulos ou listas.

## Como escrever bons cartões

- Um fato por cartão (princípio da informação mínima). Divida listas e conceitos longos em vários cartões.
- A frente precisa de contexto suficiente para ter uma única resposta certa, sem ambiguidade.
- Respostas curtas. A explicação vai em `extra`.
- Prefira `cloze` para definições, números, valores de referência e sequências; `basic` para perguntas de raciocínio ("por que…", "qual a conduta…").
- Não invente: só fatos corretos e atuais. Se houver divergência entre fontes, diga isso em `extra`.

## Como escrever boas questões

- Estilo de prova: caso clínico ou situação concreta quando fizer sentido.
- Distratores plausíveis, do mesmo tipo e tamanho da alternativa certa. Nada de "todas as anteriores" ou "nenhuma das anteriores".
- Distribua a letra certa entre A, B, C e D.
- A explicação justifica a certa e diz por que cada distrator está errado.

Escreva no idioma do meu pedido.

Meu pedido:
