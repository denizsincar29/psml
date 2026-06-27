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

/// Уровень генерация-time проверки `<git check="..">`/`<cmd check="..">`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    /// `0` (по умолчанию) — не проверять вообще, прежнее поведение.
    Off,
    /// `1` — реально выполнить команду при генерации; если код возврата
    /// не 0 — напечатать stdout/stderr и ПРЕРВАТЬ генерацию.
    Error,
    /// `2` — то же самое, но при ошибке только предупреждение, генерация продолжается.
    Warn,
}

impl CheckLevel {
    pub fn parse(s: &str) -> Result<Self, PsmlError> {
        Ok(match s {
            "0" => CheckLevel::Off,
            "1" => CheckLevel::Error,
            "2" => CheckLevel::Warn,
            _ => return Err(err!("атрибут check: ожидалось 0, 1 или 2, получено {:?}", s)),
        })
    }

    pub fn is_off(self) -> bool {
        matches!(self, CheckLevel::Off)
    }
}

/// Настройки генерация-time проверки для `<git>`/`<cmd>` — см. [`CheckLevel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecCheck {
    pub level: CheckLevel,
    /// `check-path` (по умолчанию `true`, имеет смысл только при `level != Off`):
    /// сначала дёшево проверить, что команда вообще существует (в PATH или
    /// по `path`), и только если она нашлась — выполнять её целиком.
    pub check_path: bool,
    /// `path` — необязательный явный путь к исполняемому файлу вместо поиска
    /// в PATH (для `check_path`).
    pub path: Option<String>,
}

impl Default for ExecCheck {
    fn default() -> Self {
        ExecCheck { level: CheckLevel::Off, check_path: true, path: None }
    }
}
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

    /// strftime-формат для этого режима — используют бэкенды, которые в
    /// итоге зовут системную команду `date` (fish, nu) или `--preview`.
    pub fn strftime_fmt(self) -> &'static str {
        match self {
            TimeMode::H24 => "%H:%M:%S",
            TimeMode::H12 => "%I:%M:%S",
            TimeMode::AmPm => "%I:%M %p",
            TimeMode::H24Short => "%H:%M",
        }
    }
}

/// strftime-формат даты по умолчанию (когда `<date>` без `fmt`) — общий для
/// тех же трёх мест, что и `TimeMode::strftime_fmt`.
pub const DEFAULT_DATE_FMT: &str = "%a %b %d";

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
    Git { prefix: String, suffix: String, check: ExecCheck },
    /// `<cmd run="...">` — вывод произвольной POSIX shell-команды.
    Cmd { run: String, check: ExecCheck },
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
