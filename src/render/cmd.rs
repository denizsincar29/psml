//! cmd.exe-бэкенд.
//!
//! cmd.exe — принципиально самый ограниченный из всех бэкендов: его
//! `PROMPT` — это просто текст с заранее заданным набором `$`-кодов
//! (см. `prompt /?`), которые cmd подставляет при каждой перерисовке,
//! плюс несколько "живых" `%псевдо-переменных%` (`%CD%`, `%ERRORLEVEL%`,
//! `%RANDOM%`...), которые cmd тоже пересчитывает на лету — но никакого
//! произвольного ВЫПОЛНЕНИЯ команд при отрисовке промпта нет и быть не
//! может: `PROMPT` — статическая строка, а не функция/скрипт, в отличие
//! от всех остальных бэкендов в этом крейте.
//!
//! Поэтому `<git/>`, `<cmd run="...">`, `<jobs/>` и `<cwdbase/>`
//! принципиально нереализуемы и явно ошибаются с понятным объяснением —
//! лучше честная ошибка на этапе генерации, чем промпт, в котором что-то
//! тихо не работает.
//!
//! ANSI-цвет в cmd работает только если включена VT-обработка консоли
//! (по умолчанию включена в Windows Terminal и современных билдах
//! `conhost.exe`). Текстовое обозначение ESC-байта в самом `PROMPT` —
//! официальный код `$E`, тот же, что в классическом трюке
//! `prompt $E[32m$P$E[0m$G`.
//!
//! Заголовок окна — отдельная команда `title ...` перед `prompt ...`
//! (в cmd.exe нет аналога OSC-последовательности, встроенной прямо в
//! текст промпта, как в bash/zsh).

use crate::err;
use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::resolve_sgr_color;
use crate::render::ShellBackend;

pub struct Cmd;

impl ShellBackend for Cmd {
    fn key(&self) -> &'static str {
        "cmd"
    }

    fn render_document(&self, doc: &Document, raw: bool) -> Result<String, PsmlError> {
        let mut body = String::new();
        render_nodes(&doc.body, &mut body)?;
        Ok(finalize(&doc.title, &body, raw))
    }
}

fn render_nodes(nodes: &[Node], out: &mut String) -> Result<(), PsmlError> {
    for n in nodes {
        render_node(n, out)?;
    }
    Ok(())
}

fn render_node(node: &Node, out: &mut String) -> Result<(), PsmlError> {
    match node {
        Node::Text(s) => out.push_str(&escape_literal(s)),
        Node::Br => out.push_str("$_"),
        Node::User => out.push_str("%USERNAME%"),
        Node::Host => out.push_str("%COMPUTERNAME%"),
        // cmd не имеет отдельного "короткое/полное имя хоста" — это
        // приближение FQDN через домен текущего пользователя.
        Node::HostFull => out.push_str("%COMPUTERNAME%.%USERDNSDOMAIN%"),
        Node::Cwd => out.push_str("$P"),
        Node::CwdBase => {
            return Err(err!(
                "cmd.exe: <cwdbase/> не поддержан — у PROMPT нет способа взять только последний компонент пути без выполнения внешней команды"
            ))
        }
        // cmd не различает root/обычного пользователя в PROMPT — это
        // просто литеральный символ, как и было бы у $G.
        Node::Symbol => out.push('>'),
        Node::Jobs => {
            return Err(err!(
                "cmd.exe: <jobs/> не поддержан — у cmd.exe нет управления job'ами"
            ))
        }
        Node::ExitCode => out.push_str("%ERRORLEVEL%"),
        Node::Reset => out.push_str("$E[0m"),
        Node::Time(mode) => match mode {
            TimeMode::H24 => out.push_str("$T"),
            _ => {
                return Err(err!(
                    "cmd.exe: <time mode=...> поддерживает только mode=\"24\" — у $T фиксированный локальный формат, 12-часовой/AM-PM формат недостижим без внешней команды"
                ))
            }
        },
        Node::Date(fmt) => match fmt {
            None => out.push_str("$D"),
            Some(_) => {
                return Err(err!(
                    "cmd.exe: <date fmt=...> не поддержан — у $D фиксированный локальный формат, свой strftime-формат недостижим без внешней команды"
                ))
            }
        },
        Node::Git { .. } => {
            return Err(err!(
                "cmd.exe: <git/> не поддержан — PROMPT в cmd.exe это статическая строка, она не может выполнять команды при каждой перерисовке"
            ))
        }
        Node::Cmd { .. } => {
            return Err(err!(
                "cmd.exe: <cmd run=...> не поддержан — PROMPT в cmd.exe это статическая строка, она не может выполнять произвольные команды при каждой перерисовке"
            ))
        }
        Node::Bold(children) => emit_pair(children, "1", "22", out)?,
        Node::Underline(children) => emit_pair(children, "4", "24", out)?,
        Node::Italic(children) => emit_pair(children, "3", "23", out)?,
        Node::Color { fg, bg, children } => {
            emit_color(fg.as_deref(), bg.as_deref(), children, out)?
        }
    }
    Ok(())
}

fn emit_pair(children: &[Node], on: &str, off: &str, out: &mut String) -> Result<(), PsmlError> {
    out.push_str(&format!("$E[{}m", on));
    render_nodes(children, out)?;
    out.push_str(&format!("$E[{}m", off));
    Ok(())
}

fn emit_color(
    fg: Option<&str>,
    bg: Option<&str>,
    children: &[Node],
    out: &mut String,
) -> Result<(), PsmlError> {
    let mut open_codes = Vec::new();
    if let Some(f) = fg {
        open_codes.push(resolve_sgr_color(f, false)?);
    }
    if let Some(b) = bg {
        open_codes.push(resolve_sgr_color(b, true)?);
    }
    out.push_str(&format!("$E[{}m", open_codes.join(";")));
    render_nodes(children, out)?;
    let mut close_codes = Vec::new();
    if fg.is_some() {
        close_codes.push("39");
    }
    if bg.is_some() {
        close_codes.push("49");
    }
    out.push_str(&format!("$E[{}m", close_codes.join(";")));
    Ok(())
}

/// Единственный по-настоящему опасный символ в тексте `PROMPT` — `$`
/// (запускает `$`-код); литеральный `$` экранируется документированным
/// кодом `$$`. Буквальный `%` теоретически тоже может случайно сложиться
/// в имя существующей переменной (`%...%`), но это безопасно нейтрализовать
/// без потери смысла невозможно — `%`-подстановка нужна нам самим для
/// `%USERNAME%`/`%ERRORLEVEL%` и т.п., так что это документированное,
/// а не тихое ограничение.
fn escape_literal(s: &str) -> String {
    s.replace('$', "$$")
}

fn finalize(title: &str, body: &str, raw: bool) -> String {
    if raw {
        return body.to_string();
    }
    let mut script = String::new();
    if !title.is_empty() {
        script.push_str(&format!("title {}\r\n", title));
    }
    script.push_str(&format!("prompt {}\r\n", body));
    script
}
