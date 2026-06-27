//! Реестр бэкендов-шеллов. Это единственное место, которое нужно трогать,
//! чтобы добавить новый шелл: написать модуль с `impl ShellBackend`, и
//! добавить ссылку на него в `BACKENDS`. Парсер (`crate::parser`) и IR
//! (`crate::ir`) трогать не нужно — они уже умеют всё, что нужно любому шеллу.

pub mod bash;
pub mod cmd;
pub mod fish;
pub mod nu;
pub mod powershell;
pub mod util;
pub mod zsh;

use crate::err;
use crate::ir::{Document, PsmlError};

/// Один шелл-бэкенд: умеет превратить IR-дерево в готовый для `eval`/
/// `source` скрипт на своём языке.
pub trait ShellBackend: Sync {
    /// Канонический ключ — то, что пишут в `--shell` и `<psml shell="...">`.
    fn key(&self) -> &'static str;
    /// Альтернативные имена (например, `pwsh` для PowerShell).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
    /// Рендерит документ целиком: и тело промпта, и финальную обёртку
    /// (присвоение переменной / определение функции — у каждого шелла своё),
    /// и `raw`-режим (только "сырое" значение промпта, без обёртки).
    fn render_document(&self, doc: &Document, raw: bool) -> Result<String, PsmlError>;
}

/// Реестр всех известных шеллов. Порядок — это и порядок в `--help`/списке
/// поддерживаемых шеллов.
pub const BACKENDS: &[&dyn ShellBackend] = &[
    &bash::Bash,
    &zsh::Zsh,
    &fish::Fish,
    &powershell::PowerShell,
    &cmd::Cmd,
    &nu::Nu,
];

fn matches_key(backend: &dyn ShellBackend, name: &str) -> bool {
    backend.key() == name || backend.aliases().contains(&name)
}

pub fn find_backend(name: &str) -> Option<&'static dyn ShellBackend> {
    let lname = name.to_lowercase();
    BACKENDS
        .iter()
        .copied()
        .find(|b| matches_key(*b, &lname))
}

/// Список ключей для сообщений об ошибках / `--help`.
pub fn shell_keys() -> Vec<&'static str> {
    BACKENDS.iter().map(|b| b.key()).collect()
}

/// Определяет итоговый шелл: явный аргумент (`--shell`) важнее атрибута в
/// файле (`<psml shell="...">`), а если не указано ни то, ни другое — bash
/// по умолчанию (как и было раньше).
pub fn resolve_shell(
    cli_override: Option<&str>,
    doc_attr: Option<&str>,
) -> Result<&'static dyn ShellBackend, PsmlError> {
    let chosen = cli_override.or(doc_attr).unwrap_or("bash");
    find_backend(chosen).ok_or_else(|| {
        err!(
            "неизвестный shell: {:?} (доступны: {})",
            chosen,
            shell_keys().join(", ")
        )
    })
}
