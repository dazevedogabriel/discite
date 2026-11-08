//! IA: gera sugestões de cartões e questões a partir das notas, com um modelo
//! que roda no próprio computador (llama.cpp embutido; da lista ou um .gguf
//! qualquer) ou com uma IA online por API (`online.rs`). Em todos os casos as
//! regras e a conferência com as notas são as mesmas.

pub mod engine;
pub mod ground;
#[cfg(test)]
mod eval;
pub mod models;
pub mod online;
pub mod prompt;
