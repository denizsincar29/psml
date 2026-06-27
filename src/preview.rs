//! `--preview`: рендерит [`Document`] не в shell-скрипт, а прямо в ANSI-текст,
//! который можно напечатать в терминал и сразу увидеть, как выглядит промпт —
//! без правки `.bashrc`/`config.fish`/профиля и перезапуска шелла.
//!
//! Это НЕ седьмой `ShellBackend` — у него принципиально другая роль: вместо
//! того чтобы генерировать код, который шелл выполнит сам при каждой
//! отрисовке, preview выполняет всё ПРЯМО СЕЙЧАС, один раз, и печатает
//! результат. Отсюда два честных компромисса:
//!
//! - `<user/>`, `<host/>`, `<hostfull/>`, `<cwd/>`, `<cwdbase/>`, `<time/>`,
//!   `<date/>`, `<git/>`, `<cmd run="...">` — настоящие живые значения
//!   (текущий пользователь/хост/директория/время и реальный вывод `git`/
//!   `<cmd run>` — он же ваш собственный shell-код из самого PSML-файла,
//!   выполнить его не более рискованно, чем подключить файл в `.bashrc`).
//! - `<jobs/>` и `<exitcode/>` — это вопрос к ИНТЕРАКТИВНОЙ сессии шелла
//!   (сколько у НЕЁ фоновых job'ов, каким был код возврата ЕЁ последней
//!   команды), а не к этому процессу — у процесса `psml --preview` своей
//!   таблицы job'ов и истории команд просто нет. Подставляется фиксированный
//!   пример (`0`) — без inline-пометки прямо в строке (это испортило бы
//!   именно ту визуальную точность, ради которой превью существует), а
//!   отдельной строкой после всего вывода, и только если эти теги
//!   реально встретились в документе.

use std::process::Command;

use crate::ir::{Document, Node, PsmlError};
use crate::render::util::{raw_sgr, resolve_sgr_color};

/// Значение-пример для `<jobs/>`/`<exitcode/>` — см. пометку, которую
/// [`render_preview`] добавляет в конец вывода, если они встретились.
const PLACEHOLDER_VALUE: &str = "0";

pub fn render_preview(doc: &Document) -> Result<String, PsmlError> {
    let mut out = String::new();
    if !doc.title.is_empty() {
        out.push_str(&format!("Заголовок окна: {}\n", doc.title));
    }
    let mut used_placeholder = false;
    render_nodes(&doc.body, &mut out, &mut used_placeholder)?;
    if used_placeholder {
        out.push_str(
            "\n(значения <jobs/>/<exitcode/> выше — пример (0): они известны только \
             живой интерактивной сессии шелла, а не этому процессу)",
        );
    }
    Ok(out)
}

fn render_nodes(nodes: &[Node], out: &mut String, used_placeholder: &mut bool) -> Result<(), PsmlError> {
    for n in nodes {
        render_node(n, out, used_placeholder)?;
    }
    Ok(())
}

fn render_node(node: &Node, out: &mut String, used_placeholder: &mut bool) -> Result<(), PsmlError> {
    match node {
        Node::Text(s) => out.push_str(s),
        Node::Br => out.push('\n'),
        Node::User => out.push_str(&current_user()),
        Node::Host => out.push_str(&run_capture("hostname", &[]).unwrap_or_else(|| "host".to_string())),
        Node::HostFull => out.push_str(
            &run_capture("hostname", &["-f"])
                .or_else(|| run_capture("hostname", &[]))
                .unwrap_or_else(|| "host".to_string()),
        ),
        Node::Cwd => out.push_str(&cwd_collapsed()),
        Node::CwdBase => out.push_str(&cwd_base()),
        Node::Symbol => out.push_str(if is_root() { "#" } else { "$" }),
        Node::Jobs => {
            *used_placeholder = true;
            out.push_str(PLACEHOLDER_VALUE);
        }
        Node::ExitCode => {
            *used_placeholder = true;
            out.push_str(PLACEHOLDER_VALUE);
        }
        Node::Reset => out.push_str(&raw_sgr("0")),
        Node::Time(mode) => out.push_str(&date_command(mode.strftime_fmt())),
        Node::Date(fmt) => {
            out.push_str(&date_command(fmt.as_deref().unwrap_or(crate::ir::DEFAULT_DATE_FMT)))
        }
        Node::Git { prefix, suffix, .. } => {
            if let Some(b) = git_branch() {
                out.push_str(prefix);
                out.push_str(&b);
                out.push_str(suffix);
            }
        }
        Node::Cmd { run, .. } => out.push_str(&run_shell_unconditional(run)),
        Node::Bold(children) => emit_pair(children, "1", "22", out, used_placeholder)?,
        Node::Underline(children) => emit_pair(children, "4", "24", out, used_placeholder)?,
        Node::Italic(children) => emit_pair(children, "3", "23", out, used_placeholder)?,
        Node::Color { fg, bg, children } => {
            emit_color(fg.as_deref(), bg.as_deref(), children, out, used_placeholder)?
        }
    }
    Ok(())
}

fn emit_pair(
    children: &[Node],
    on: &str,
    off: &str,
    out: &mut String,
    used_placeholder: &mut bool,
) -> Result<(), PsmlError> {
    out.push_str(&raw_sgr(on));
    render_nodes(children, out, used_placeholder)?;
    out.push_str(&raw_sgr(off));
    Ok(())
}

fn emit_color(
    fg: Option<&str>,
    bg: Option<&str>,
    children: &[Node],
    out: &mut String,
    used_placeholder: &mut bool,
) -> Result<(), PsmlError> {
    let mut open_codes = Vec::new();
    if let Some(f) = fg {
        open_codes.push(resolve_sgr_color(f, false)?);
    }
    if let Some(b) = bg {
        open_codes.push(resolve_sgr_color(b, true)?);
    }
    out.push_str(&raw_sgr(&open_codes.join(";")));
    render_nodes(children, out, used_placeholder)?;
    let mut close_codes = Vec::new();
    if fg.is_some() {
        close_codes.push("39");
    }
    if bg.is_some() {
        close_codes.push("49");
    }
    out.push_str(&raw_sgr(&close_codes.join(";")));
    Ok(())
}

/// Запускает `cmd args...`, возвращает stdout (без хвостового `\n`), если
/// процесс реально запустился и что-то напечатал. `None` — и "команды нет",
/// и "команда ничего не напечатала" неразличимы и не должны быть различимы
/// здесь: в обоих случаях честно показать, что показать нечего.
fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `<cmd run="...">` — в отличие от `run_capture`, статус выхода НЕ
/// проверяется: ровно как настоящая `$(...)`-подстановка в живом шелле,
/// захватывается stdout независимо от кода возврата (классический паттерн
/// `<cmd run="[ $? -ne 0 ] &amp;&amp; printf ...">` специально завершается
/// ненулевым кодом, когда ничего печатать не нужно — это не ошибка).
fn run_shell_unconditional(run: &str) -> String {
    Command::new("sh")
        .arg("-c")
        .arg(run)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string())
        .unwrap_or_default()
}

fn git_branch() -> Option<String> {
    run_capture("git", &["symbolic-ref", "--short", "HEAD"])
        .or_else(|| run_capture("git", &["rev-parse", "--short", "HEAD"]))
}

fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| run_capture("whoami", &[]))
        .unwrap_or_else(|| "user".to_string())
}

fn is_root() -> bool {
    run_capture("id", &["-u"]).map(|s| s == "0").unwrap_or(false)
}

fn cwd_collapsed() -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()) {
        if !home.is_empty() {
            if let Some(rest) = cwd.strip_prefix(&home) {
                return format!("~{}", rest);
            }
        }
    }
    cwd
}

fn cwd_base() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "/".to_string())
}

/// `date "+FMT"` — тот же приём, что в fish/nu-бэкендах. Если `date` не
/// нашлась (на голом Windows без git-bash/WSL) — честный плейсхолдер, а не
/// падение всего превью.
fn date_command(fmt: &str) -> String {
    run_capture("date", &[&format!("+{}", fmt)]).unwrap_or_else(|| format!("<date +{}>", fmt))
}
