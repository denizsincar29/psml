//! Intermediate representation (IR) PSML-документа.
//!
//! Это сердце архитектуры: парсер (`parser.rs`) знает только про PSML и
//! строит дерево [`Node`] — он НИЧЕГО не знает про bash/zsh/fish/итд.
//! Бэкенды (`render/*.rs`) знают только про [`Node`]/[`Document`] — они
//! НИЧЕГО не знают про разбор XML/PSML.
//!
//! Добавить новый шелл = реализовать `ShellBackend` для нового дерева,
//! ничего не трогая в парсере. Добавить новый тег PSML = добавить вариант
//! в `Node` + ветку в парсере — рендереры явно увидят `unsupported_node`
//! по умолчанию для тегов, которые не успели реализовать.

use std::fmt;

/// Ошибка PSML — общая для разбора и для рендера в конкретный шелл.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsmlError(pub String);

impl fmt::Display for PsmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PsmlError {}

#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::ir::PsmlError(format!($($arg)*)) };
}

/// Режим тега `<time mode="...">`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeMode {
    H24,
    H12,
    AmPm,
    H24Short,
}

impl TimeMode {
    pub fn parse(s: &str) -> Result<Self, PsmlError> {
        Ok(match s {
            "24" => TimeMode::H24,
            "12" => TimeMode::H12,
            "ampm" => TimeMode::AmPm,
            "24short" => TimeMode::H24Short,
            _ => return Err(err!("<time mode=...>: неизвестный режим {:?}", s)),
        })
    }
}

/// Один узел дерева промпта. Контейнерные варианты (Bold/Underline/Italic/
/// Color) хранят уже вложенное содержимое — никакого стека стилей в
/// рендерерах не нужно, вложенность статически зафиксирована структурой.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Текстовый узел (HTML-сущности уже раскрыты парсером).
    Text(String),
    /// `<br>` / `<br/>` — перевод строки.
    Br,
    User,
    Host,
    HostFull,
    Cwd,
    CwdBase,
    /// `$`/`#`-символ приглашения (для root — другой символ, если шелл это умеет).
    Symbol,
    /// Число фоновых job'ов.
    Jobs,
    /// Код возврата последней команды.
    ExitCode,
    Time(TimeMode),
    /// `<date fmt="...">`, `fmt` — strftime-формат (None = формат шелла по умолчанию).
    Date(Option<String>),
    /// `<git prefix=".." suffix="..">` — текущая git-ветка.
    Git { prefix: String, suffix: String },
    /// `<cmd run="...">` — вывод произвольной POSIX shell-команды.
    Cmd(String),
    /// Сброс всех цветов/стилей.
    Reset,
    Bold(Vec<Node>),
    Underline(Vec<Node>),
    Italic(Vec<Node>),
    /// `fg`/`bg` — сырые строки атрибутов (имя цвета / число 0-255 / `#rrggbb`),
    /// валидируются и резолвятся в рендерере конкретного шелла, потому что
    /// поддерживаемая палитра — это свойство шелла/терминала, а не PSML.
    Color {
        fg: Option<String>,
        bg: Option<String>,
        children: Vec<Node>,
    },
}

/// Разобранный PSML-документ: заголовок окна (только текст), тело
/// промпта (дерево [`Node`]) и опциональный шелл из `<psml shell="...">`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub shell: Option<String>,
    pub title: String,
    pub body: Vec<Node>,
}
