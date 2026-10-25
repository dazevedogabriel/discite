//! IA local: gera sugestões de cartões a partir das notas com um modelo que
//! roda no próprio computador (llama.cpp embutido). Nada sai da máquina; o
//! único acesso à rede é o download do modelo, quando o usuário pede.

pub mod engine;
pub mod ground;
#[cfg(test)]
mod eval;
pub mod models;
pub mod prompt;
