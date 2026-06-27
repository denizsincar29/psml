//! PSML (Prompt String Markup Language) -> готовый PS1/PROMPT/`function
//! prompt`/... для конкретного шелла.
//!
//! Архитектура крейта в трёх слоях:
//!
//! 1. [`parser::parse_psml`] — PSML-текст -> [`ir::Document`] (дерево
//!    [`ir::Node`]). Это единственное место, которое знает про синтаксис
//!    PSML/XML; ему совершенно не важно, какой шелл выбран — за одним
//!    исключением: preprocessing-теги `<if shell="...">`/`<else>` решаются
//!    прямо во время разбора, поэтому `parse_psml` принимает целевой шелл
//!    аргументом (см. doc-комментарий в `parser.rs`).
//! 2. [`validate::run_checks`] — необязательный шаг между парсингом и
//!    рендером: реально выполняет `<git check="1|2">`/`<cmd check="1|2">`
//!    прямо во время генерации и либо прерывает её, либо предупреждает.
//! 3. [`render::ShellBackend`] — знает только про дерево [`ir::Node`] и
//!    ничего про разбор PSML. Каждый шелл (bash, zsh, fish, powershell,
//!    cmd, nu) — отдельный модуль в [`render`], реализующий этот трейт.
//! 4. [`convert`] — склейка всех предыдущих пунктов; это то, что
//!    используют CLI (`main.rs`) и тесты.
//!
//! Отдельно — [`preview::render_preview`] (`--preview` в CLI): не бэкенд
//! шелла, а прямой "интерпретатор" дерева [`ir::Node`] в готовый ANSI-текст
//! для немедленного просмотра в терминале, без правки конфига шелла.
//!
//! Добавить новый шелл = написать новый модуль в `src/render/`,
//! реализовать там `ShellBackend` и добавить ссылку на него в
//! `render::BACKENDS`. Парсер и остальные бэкенды трогать не нужно.

pub mod ir;
pub mod parser;
pub mod preview;
pub mod render;
pub mod sysprobe;
pub mod validate;

pub use ir::{CheckLevel, Document, ExecCheck, Node, PsmlError, TimeMode};
pub use parser::{parse_psml, peek_shell_attr};
pub use preview::render_preview;
pub use render::{find_backend, resolve_shell, shell_keys, ShellBackend, BACKENDS};

/// Главная функция крейта: разбирает `psml_text` и рендерит результат для
/// нужного шелла.
///
/// `shell` (если есть) важнее атрибута `<psml shell="...">` внутри файла;
/// если не указано ни то, ни другое — по умолчанию `bash` (как и раньше).
/// `raw` — печатать только вычисленное значение промпта, без обёртки
/// (`PS1=...` / `function prompt {...}` / и т.п. — у каждого шелла своя).
///
/// Шелл резолвится ДО полного разбора документа (через дешёвый
/// [`peek_shell_attr`]) — это нужно, чтобы `<if shell="...">` внутри
/// документа знал, для какого шелла он сейчас решает свою судьбу.
pub fn convert(psml_text: &str, shell: Option<&str>, raw: bool) -> Result<String, PsmlError> {
    let doc_shell_attr = parser::peek_shell_attr(psml_text);
    let backend = render::resolve_shell(shell, doc_shell_attr.as_deref())?;
    let doc = parser::parse_psml(psml_text, backend.key())?;
    validate::run_checks(&doc)?;
    backend.render_document(&doc, raw)
}

/// POSIX single-quote экранирование (`'...'`) — оставлено в публичном API
/// ради обратной совместимости с тем, чем оно было раньше (использовалось
/// для сборки `PS1='...'`/`PROMPT='...'`).
pub fn shell_quote_single(value: &str) -> String {
    render::util::posix_quote_single(value)
}
