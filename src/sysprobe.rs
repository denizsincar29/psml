//! Дешёвая ("без побочных эффектов") проверка существования команды —
//! общая для `<if command="...">` (`parser.rs`) и `check-path` у
//! `<git>`/`<cmd>` (`validate.rs`). Чистый Rust, без вызова `which`/`where`:
//! `std::env::split_paths` уже сам умеет правильно бить `PATH` и на Unix
//! (`:`), и на Windows (`;`).

use std::path::Path;

/// Ищет `name` в `PATH`. На Windows дополнительно пробует расширения
/// `.exe`/`.bat`/`.cmd` (как это делает сам Windows при разрешении команд).
pub fn command_in_path(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return true;
        }
        if cfg!(windows) {
            for ext in ["exe", "bat", "cmd"] {
                if candidate.with_extension(ext).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

/// Проверка существования команды: либо по явному `path`, либо (если он не
/// задан) поиском в `PATH` через [`command_in_path`].
pub fn command_available(name: &str, explicit_path: Option<&str>) -> bool {
    match explicit_path {
        Some(p) => Path::new(p).is_file(),
        None => command_in_path(name),
    }
}
