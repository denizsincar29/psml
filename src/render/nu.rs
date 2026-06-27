//! nu (Nushell)-бэкенд.
//!
//! Это шестой, "сверх" запрошенного списка бэкенд — конкретно чтобы
//! показать, что архитектура (IR + `ShellBackend`) реально позволяет
//! добавлять новые шеллы без единой правки в парсере или в других
//! бэкендах. Nushell — самый юный и быстро меняющийся из всех шеллов
//! здесь, поэтому: считайте этот файл best-effort (написан по документации
//! Nushell ~0.9x), а не настолько же обкатанным, как bash/zsh/fish/
//! powershell/cmd. Если синтаксис разойдётся с вашей версией — это
//! ожидаемо, правьте смело.
//!
//! Механика — как у fish/PowerShell: промпт это замыкание
//! (`$env.PROMPT_COMMAND = {|| "..." }`), исполняемое заново при каждой
//! отрисовке, тело собирается одной interpolated-строкой `$"..."` с
//! вкраплениями `(expr)`.
//!
//! `<git/>` — нативно через сам `git`. `<jobs/>` не поддержан: в отличие
//! от bash/zsh/fish, у Nushell нет классического Unix job-control.
//! `<cmd run="...">` (произвольная POSIX-команда из PSML) выполняется
//! через `^bash -c` — как и в fish/PowerShell, транслировать
//! произвольный bash-синтаксис в нативный nu-код не имеет смысла.

use crate::err;
use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::{nu_escape_interp, nu_quote_raw, raw_sgr, resolve_sgr_color};
use crate::render::ShellBackend;

pub struct Nu;

impl ShellBackend for Nu {
    fn key(&self) -> &'static str {
        "nu"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["nushell"]
    }

    fn render_document(&self, doc: &Document, raw: bool) -> Result<String, PsmlError> {
        let mut b = Builder::new();
        render_nodes(&doc.body, &mut b)?;
        Ok(finalize(&doc.title, &b.into_inner(), raw))
    }
}

struct Builder {
    out: String,
}

impl Builder {
    fn new() -> Self {
        Builder { out: String::new() }
    }

    fn push_lit(&mut self, s: &str) {
        self.out.push_str(&nu_escape_interp(s));
    }

    fn push_expr(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn into_inner(self) -> String {
        self.out
    }
}

fn render_nodes(nodes: &[Node], b: &mut Builder) -> Result<(), PsmlError> {
    for n in nodes {
        render_node(n, b)?;
    }
    Ok(())
}

fn render_node(node: &Node, b: &mut Builder) -> Result<(), PsmlError> {
    match node {
        Node::Text(s) => b.push_lit(s),
        Node::Br => b.push_lit("\n"),
        Node::User => b.push_expr(
            "(($env.USER? | default '') | default ($env.USERNAME? | default ''))",
        ),
        Node::Host => b.push_expr("(^hostname)"),
        Node::HostFull => b.push_expr("(try { ^hostname -f } catch { ^hostname })"),
        // без сворачивания домашней папки в `~`, в отличие от bash/zsh/fish —
        // сознательное упрощение этого бонусного бэкенда.
        Node::Cwd => b.push_expr("(pwd)"),
        Node::CwdBase => b.push_expr("(pwd | path basename)"),
        Node::Symbol => b.push_expr("(if (is-admin) { '#' } else { '$' })"),
        Node::Jobs => {
            return Err(err!(
                "nu: <jobs/> не поддержан — у Nushell нет классического Unix job-control"
            ))
        }
        Node::ExitCode => b.push_expr("$env.LAST_EXIT_CODE"),
        Node::Reset => b.push_lit(&raw_sgr("0")),
        Node::Time(mode) => b.push_expr(&time_expr(*mode)),
        Node::Date(fmt) => b.push_expr(&date_expr(fmt.as_deref())),
        Node::Git { prefix, suffix } => b.push_expr(&git_expr(prefix, suffix)),
        Node::Cmd(run) => b.push_expr(&format!("(^bash -c {})", nu_quote_raw(run))),
        Node::Bold(children) => emit_pair(children, "1", "22", b)?,
        Node::Underline(children) => emit_pair(children, "4", "24", b)?,
        Node::Italic(children) => emit_pair(children, "3", "23", b)?,
        Node::Color { fg, bg, children } => {
            emit_color(fg.as_deref(), bg.as_deref(), children, b)?
        }
    }
    Ok(())
}

fn emit_pair(children: &[Node], on: &str, off: &str, b: &mut Builder) -> Result<(), PsmlError> {
    b.push_lit(&raw_sgr(on));
    render_nodes(children, b)?;
    b.push_lit(&raw_sgr(off));
    Ok(())
}

fn emit_color(
    fg: Option<&str>,
    bg: Option<&str>,
    children: &[Node],
    b: &mut Builder,
) -> Result<(), PsmlError> {
    let mut open_codes = Vec::new();
    if let Some(f) = fg {
        open_codes.push(resolve_sgr_color(f, false)?);
    }
    if let Some(bgv) = bg {
        open_codes.push(resolve_sgr_color(bgv, true)?);
    }
    b.push_lit(&raw_sgr(&open_codes.join(";")));
    render_nodes(children, b)?;
    let mut close_codes = Vec::new();
    if fg.is_some() {
        close_codes.push("39");
    }
    if bg.is_some() {
        close_codes.push("49");
    }
    b.push_lit(&raw_sgr(&close_codes.join(";")));
    Ok(())
}

fn time_expr(mode: TimeMode) -> String {
    format!("(date now | format date \"{}\")", mode.strftime_fmt())
}

fn date_expr(fmt: Option<&str>) -> String {
    let f = fmt.unwrap_or(crate::ir::DEFAULT_DATE_FMT);
    format!("(date now | format date \"{}\")", f)
}

/// `<git/>` — нативно через сам `git`.
fn git_expr(prefix: &str, suffix: &str) -> String {
    format!(
        "(let b = (try {{ git symbolic-ref --short HEAD }} catch {{ try {{ git rev-parse --short HEAD }} catch {{ '' }} }}); if ($b | str length) > 0 {{ $\"{}($b){}\" }} else {{ '' }})",
        nu_escape_interp(prefix),
        nu_escape_interp(suffix)
    )
}

fn finalize(title: &str, body_inner: &str, raw: bool) -> String {
    let body_str = format!("$\"{}\"", body_inner);
    if raw {
        return body_str;
    }
    let mut script = String::new();
    if !title.is_empty() {
        // nu умеет менять заголовок терминала через ту же OSC-последовательность,
        // что и bash/zsh — просто печатаем её отдельной командой при старте.
        script.push_str(&format!(
            "print -n \"\\u{{1b}}]0;{}\\u{{07}}\"\n",
            nu_escape_interp(title)
        ));
    }
    script.push_str(&format!("$env.PROMPT_COMMAND = {{|| {} }}\n", body_str));
    script
}
