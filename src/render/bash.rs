//! bash-бэкенд. Выход побайтово совпадает с тем, что было до рефакторинга
//! на IR (см. `tests/integration.rs::matches_frozen_snapshot_default_bash`
//! и сверки с `python_ref/psml.py`) — меняется только то, ОТКУДА берутся
//! данные (дерево [`Node`], а не поток событий парсера).

use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::{posix_quote_single, resolve_sgr_color};
use crate::render::ShellBackend;

pub struct Bash;

impl ShellBackend for Bash {
    fn key(&self) -> &'static str {
        "bash"
    }

    fn render_document(&self, doc: &Document, raw: bool) -> Result<String, PsmlError> {
        let mut uses_subst = false;
        let body = render_nodes(&doc.body, &mut uses_subst)?;
        Ok(finalize(&doc.title, &body, raw, uses_subst))
    }
}

fn render_nodes(nodes: &[Node], uses_subst: &mut bool) -> Result<String, PsmlError> {
    let mut out = String::new();
    for n in nodes {
        render_node(n, &mut out, uses_subst)?;
    }
    Ok(out)
}

fn render_node(node: &Node, out: &mut String, uses_subst: &mut bool) -> Result<(), PsmlError> {
    match node {
        Node::Text(s) => out.push_str(s),
        Node::Br => out.push_str("\\n"),
        Node::User => out.push_str("\\u"),
        Node::Host => out.push_str("\\h"),
        Node::HostFull => out.push_str("\\H"),
        Node::Cwd => out.push_str("\\w"),
        Node::CwdBase => out.push_str("\\W"),
        Node::Symbol => out.push_str("\\$"),
        Node::Jobs => out.push_str("\\j"),
        Node::ExitCode => out.push_str("$?"),
        Node::Reset => out.push_str("\\[\\033[0m\\]"),
        Node::Time(mode) => out.push_str(time_code(*mode)),
        Node::Date(fmt) => match fmt {
            Some(f) => out.push_str(&format!("\\D{{{}}}", f)),
            None => out.push_str("\\d"),
        },
        Node::Git { prefix, suffix } => {
            *uses_subst = true;
            out.push_str(&wrap_subst(&git_cmd(prefix, suffix)));
        }
        Node::Cmd(run) => {
            *uses_subst = true;
            out.push_str(&wrap_subst(run));
        }
        Node::Bold(children) => emit_attr(children, "1", "22", out, uses_subst)?,
        Node::Underline(children) => emit_attr(children, "4", "24", out, uses_subst)?,
        Node::Italic(children) => emit_attr(children, "3", "23", out, uses_subst)?,
        Node::Color { fg, bg, children } => {
            emit_color(fg.as_deref(), bg.as_deref(), children, out, uses_subst)?
        }
    }
    Ok(())
}

fn emit_attr(
    children: &[Node],
    on: &str,
    off: &str,
    out: &mut String,
    uses_subst: &mut bool,
) -> Result<(), PsmlError> {
    out.push_str(&format!("\\[\\033[{}m\\]", on));
    for c in children {
        render_node(c, out, uses_subst)?;
    }
    out.push_str(&format!("\\[\\033[{}m\\]", off));
    Ok(())
}

fn emit_color(
    fg: Option<&str>,
    bg: Option<&str>,
    children: &[Node],
    out: &mut String,
    uses_subst: &mut bool,
) -> Result<(), PsmlError> {
    let mut open_codes = Vec::new();
    if let Some(f) = fg {
        open_codes.push(resolve_sgr_color(f, false)?);
    }
    if let Some(b) = bg {
        open_codes.push(resolve_sgr_color(b, true)?);
    }
    out.push_str(&format!("\\[\\033[{}m\\]", open_codes.join(";")));
    for c in children {
        render_node(c, out, uses_subst)?;
    }
    let mut close_codes = Vec::new();
    if fg.is_some() {
        close_codes.push("39");
    }
    if bg.is_some() {
        close_codes.push("49");
    }
    out.push_str(&format!("\\[\\033[{}m\\]", close_codes.join(";")));
    Ok(())
}

fn time_code(mode: TimeMode) -> &'static str {
    match mode {
        TimeMode::H24 => "\\t",
        TimeMode::H12 => "\\T",
        TimeMode::AmPm => "\\@",
        TimeMode::H24Short => "\\A",
    }
}

fn git_cmd(prefix: &str, suffix: &str) -> String {
    format!(
        "b=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); [ -n \"$b\" ] && printf \"%s%s%s\" \"{}\" \"$b\" \"{}\"",
        prefix, suffix
    )
}

/// Оборачивает кусок shell-кода в подстановку для bash через backticks, а не
/// `$(...)`. Это обход давнего (с 2014, до сих пор не исправленного) бага
/// именно в сборке MSYS2 (на которой держится git-bash/Git for Windows):
/// если в PS1 после `$(...)`-подстановки где-то дальше в той же строке
/// встречается "\n", парсер ломается с "syntax error near unexpected
/// token `)'". См. <https://github.com/msys2/MSYS2-packages/issues/1839>.
/// Backticks этот баг не задевают — именно поэтому в самом git-bash
/// `__git_ps1` в дефолтном PS1 вызывается через `` `__git_ps1` ``, а не
/// `$(__git_ps1)`.
fn wrap_subst(cmd: &str) -> String {
    // внутри backticks один уровень '\' перед `\`, '`' или '$' экранирует
    // символ — экранируем буквальные '\' и '`', чтобы произвольная команда
    // пользователя (`<cmd run="...">`) не сломала обёртку.
    let escaped = cmd.replace('\\', "\\\\").replace('`', "\\`");
    format!("`{}`", escaped)
}

fn finalize(title: &str, body: &str, raw: bool, _uses_subst: bool) -> String {
    // bash не нуждается в доп. строках типа zsh-овского `setopt
    // PROMPT_SUBST` — `promptvars` включён по умолчанию.
    let mut full_prompt = body.to_string();
    if !title.is_empty() {
        full_prompt = format!("\\[\\033]0;{}\\007\\]{}", title, full_prompt);
    }
    if raw {
        return full_prompt;
    }
    format!("PS1={}", posix_quote_single(&full_prompt))
}
