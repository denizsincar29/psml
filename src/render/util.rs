//! Общие для нескольких бэкендов кусочки: кавычки разных шеллов и резолвер
//! "имя/число/hex -> SGR-параметр" для шеллов, которые в итоге используют
//! голые ANSI SGR-последовательности (bash, fish, powershell, nu, cmd —
//! отличаются только тем, как каждый из них текстуально обозначает сам
//! ESC-байт в исходнике конфига). zsh сюда не входит: у него есть свои
//! "родные" токены %F{}/%K{}, не SGR-числа напрямую.

use crate::err;
use crate::ir::PsmlError;

/// POSIX-кавычки (`'...'`) — для bash и zsh. Единственный спецсимвол внутри
/// одиночных кавычек в POSIX-шеллах — сама кавычка; экранируется классическим
/// приёмом "выйти из кавычек, экранированная кавычка, снова войти".
pub fn posix_quote_single(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// fish-кавычки (`'...'`). В отличие от POSIX, fish понимает внутри одиночных
/// кавычек ровно два экранирования: `\\` и `\'` — обратный слеш нужно
/// экранировать первым, иначе экранирование кавычки задвоится.
pub fn fish_quote_single(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{}'", escaped)
}

/// PowerShell single-quote строка (`'...'`). Единственное экранирование в
/// одиночных кавычках PowerShell — сама кавычка, удваивается.
pub fn powershell_quote_single(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Экранирование для содержимого PowerShell double-quoted строки (мы строим
/// весь промпт как одну `"..."`-строку с интерполяцией `$(...)`/`$var`).
/// Спецсимволы там — backtick (служит escape-символом), сама кавычка `"` и
/// `$` (запускает интерполяцию) — экранируются через backtick.
pub fn powershell_escape_double(value: &str) -> String {
    value
        .replace('`', "``")
        .replace('"', "`\"")
        .replace('$', "`$")
}

/// "Сырая" nu-строка `r#'...'#` — никакие символы внутри не экранируются
/// и не интерполируются, кроме самой последовательности `'#` (которая
/// прервёт строку). Используется там, где нужно протащить произвольный
/// (бэш-синтаксический) текст пользователя как один аргумент без риска,
/// что nu примет в нём что-то за свой собственный синтаксис.
pub fn nu_quote_raw(value: &str) -> String {
    format!("r#'{}'#", value)
}

/// Экранирование для содержимого nu interpolated-строки (`$"..."`). nu
/// внутри неё понимает `\\`, `\"` и `\(` (последнее — чтобы буквальная
/// открывающая скобка не была принята за начало `(expr)`).
pub fn nu_escape_interp(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('(', "\\(")
}

fn is_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn fg_name_code(name: &str) -> Option<i32> {
    Some(match name {
        "black" => 30,
        "red" => 31,
        "green" => 32,
        "yellow" => 33,
        "blue" => 34,
        "magenta" => 35,
        "cyan" => 36,
        "white" => 37,
        "brightblack" => 90,
        "brightred" => 91,
        "brightgreen" => 92,
        "brightyellow" => 93,
        "brightblue" => 94,
        "brightmagenta" => 95,
        "brightcyan" => 96,
        "brightwhite" => 97,
        "gray" => 90,
        "grey" => 90,
        _ => return None,
    })
}

/// Резолвит значение атрибута `fg`/`bg` (имя цвета / число 0-255 / `#rrggbb`)
/// в SGR-параметр (то, что идёт в `\033[<это>m`). Общий для всех ANSI-SGR
/// бэкендов — палитра PSML одна и та же независимо от шелла, разница только
/// в том, ПОДДЕРЖИВАЕТ ли терминал её показать (это уже не наша забота).
pub fn resolve_sgr_color(value: &str, is_bg: bool) -> Result<String, PsmlError> {
    if value.starts_with('#') && value.chars().count() == 7 {
        let hex = &value[1..7];
        let mut bytes = [0u8; 3];
        for i in 0..3 {
            let part = hex.get(i * 2..i * 2 + 2).unwrap_or("");
            bytes[i] = u8::from_str_radix(part, 16)
                .map_err(|_| err!("некорректный hex-цвет: {:?}", value))?;
        }
        return Ok(format!(
            "{};2;{};{};{}",
            if is_bg { 48 } else { 38 },
            bytes[0],
            bytes[1],
            bytes[2]
        ));
    }
    if is_ascii_digits(value) {
        let n: i64 = value
            .parse()
            .map_err(|_| err!("номер цвета должен быть 0-255, получено {:?}", value))?;
        if !(0..=255).contains(&n) {
            return Err(err!("номер цвета должен быть 0-255, получено {:?}", value));
        }
        return Ok(format!("{};5;{}", if is_bg { 48 } else { 38 }, n));
    }
    let name = value.to_lowercase();
    let code = fg_name_code(&name).ok_or_else(|| err!("неизвестное имя цвета: {:?}", value))?;
    Ok(if is_bg {
        (code + 10).to_string()
    } else {
        code.to_string()
    })
}

/// Голая ESC-последовательность с реальным байтом 0x1B (для fish/powershell/
/// nu — им, в отличие от bash/zsh, не нужно текстовое представление ESC,
/// потому что их построчные редакторы сами умеют вычислять ширину промпта
/// без зон "невидимого" текста типа bash-овых `\[...\]`).
pub fn raw_sgr(codes: &str) -> String {
    format!("\u{1b}[{}m", codes)
}
