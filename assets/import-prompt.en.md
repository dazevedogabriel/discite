You will create flashcards and/or multiple-choice questions for me to import into Discite, my spaced-repetition study app. Reply with ONE valid JSON object, no comments and no text outside it (a ```json block is fine).

## Format

```json
{
  "cards": [
    {"type": "basic", "front": "Question", "back": "Answer", "extra": "Optional context", "tags": "topic subtopic"},
    {"type": "cloze", "text": "Sentence with a {{c1::gap}} and {{c2::another gap::hint}}.", "extra": "Optional context", "tags": "topic"},
    {"type": "reversed", "front": "Term", "back": "Definition"}
  ],
  "questions": [
    {
      "stem": "Question text",
      "options": ["Option A", "Option B", "Option C", "Option D"],
      "answer": "B",
      "explanation": "Why B is right and why each of the others is wrong."
    }
  ]
}
```

- `cards` and `questions` are both optional: send only what I ask for.
- Card types:
  - `basic`: front and back.
  - `cloze`: text with gaps `{{c1::answer}}`. Each number (c1, c2…) becomes one card; gaps with the same number show up together. An optional hint goes after `::` (`{{c1::answer::hint}}`).
  - `reversed`: front and back that also make the reverse card (back → front). Use it only for pairs that work both ways.
- `extra` (optional) shows on the back, after the answer.
- `tags` (optional): space-separated words; a tag cannot contain spaces (use hyphens).
- Each question has exactly 4 options and only one is right. `answer` is the right option's letter: "A", "B", "C" or "D".

## Formatting inside the texts

- `**bold**`, `*italic*`, `__underline__`, `~~strike~~`, `` `code` ``, `==highlight==`.
- Color: `{red|text}`. Colored highlight: `{=yellow|text}`. Colors: red, orange, yellow, green, blue, purple, gray.
- Line break: `\n` inside the JSON string. Quotes inside the text: `\"`.
- No HTML, LaTeX, or Markdown headings or lists.

## Writing good cards

- One fact per card (minimum information principle). Split lists and long concepts into several cards.
- The front needs enough context to have a single right answer, with no ambiguity.
- Short answers. The explanation goes in `extra`.
- Prefer `cloze` for definitions, numbers, reference values and sequences; `basic` for reasoning questions ("why…", "what is the next step…").
- Do not make things up: only correct, current facts. If sources disagree, say so in `extra`.

## Writing good questions

- Exam style: a case or concrete situation when it fits.
- Plausible distractors, of the same kind and length as the right option. No "all of the above" or "none of the above".
- Spread the right letter across A, B, C and D.
- The explanation justifies the right option and says why each distractor is wrong.

Write in the language of my request.

My request:
