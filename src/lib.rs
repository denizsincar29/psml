//! PSML (Prompt String Markup Language) -> готовый PS1/PROMPT/`function
//! prompt`/... для конкретного шелла.
//!
//! Архитектура крейта в трёх слоях:
//!
//! 1. [`parser::parse_psml`] — PSML-текст -> [`ir::Document`] (дерево
//!    [`ir::Node`]). Это единственное место, которое знает про синтаксис
//!    PSML/XML; ему совершенно не важно, какой шелл выбран.
//! 2. [`render::ShellBackend`] — наоборот, знает только про дерево
//!    [`ir::Node`] и ничего про разбор PSML. Каждый шелл (bash, zsh, fish,
//!    powershell, cmd, nu) — отдельный модуль в [`render`], реализующий
//!    этот трейт.
//! 3. [`convert`] — тонкая склейка двух предыдущих пунктов; это то, что
//!    используют CLI (`main.rs`) и тесты.
//!
//! Добавить новый шелл = написать новый модуль в `src/render/`,
//! реализовать там `ShellBackend` и добавить ссылку на него в
//! `render::BACKENDS`. Парсер и остальные бэкенды трогать не нужно.

pub mod ir;
pub mod parser;
pub mod render;

pub use ir::{Document, Node, PsmlError, TimeMode};
pub use parser::parse_psml;
pub use render::{find_backend, resolve_shell, shell_keys, ShellBackend, BACKENDS};

/// Главная функция крейта: разбирает `psml_text` и рендерит результат для
/// нужного шелла.
///
/// `shell` (если есть) важнее атрибута `<psml shell="...">` внутри файла;
/// если не указано ни то, ни другое — по умолчанию `bash` (как и раньше).
/// `raw` — печатать только вычисленное значение промпта, без обёртки
/// (`PS1=...` / `function prompt {...}` / и т.п. — у каждого шелла своя).
pub fn convert(psml_text: &str, shell: Option<&str>, raw: bool) -> Result<String, PsmlError> {
    let doc = parser::parse_psml(psml_text)?;
    let backend = render::resolve_shell(shell, doc.shell.as_deref())?;
    backend.render_document(&doc, raw)
}

/// POSIX single-quote экранирование (`'...'`) — оставлено в публичном API
/// ради обратной совместимости с тем, чем оно было раньше (использовалось
/// для сборки `PS1='...'`/`PROMPT='...'`).
pub fn shell_quote_single(value: &str) -> String {
    render::util::posix_quote_single(value)
}
