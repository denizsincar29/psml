//! fish-бэкенд.
//!
//! Принципиальное отличие fish от bash/zsh: промпт там не "строка с
//! escape-кодами, которую шелл сам разворачивает", а ФУНКЦИЯ
//! (`function fish_prompt ... end`), которая исполняется заново при каждой
//! отрисовке — поэтому никакого `\033[...]`-style текстового кодирования
//! переменных не нужно: `$USER`, `(git ...)`, `$status` — это просто
//! настоящий код, который fish выполнит сам.
//!
//! Реализация собирает тело промпта как список "фрагментов" —
//! либо буквальный текст (попадёт в одиночные кавычки), либо готовый
//! fish-токен (переменная или `(подстановка)`, вставляется без кавычек) —
//! и в конце склеивает их одним вызовом `printf '%s' ...` (POSIX `printf`
//! зацикливает формат на остаток аргументов, так что `'%s'` без проблем
//! "сцепляет" сколько угодно частей и не добавляет лишнего перевода строки
//! в конце, в отличие от `string join`, который иногда используют для
//! этого же в fish-сообществе).
//!
//! Заголовок окна — отдельная fish-функция `fish_title` (это нативный
//! механизм fish для заголовка, в отличие от bash/zsh, где он "вшит"
//! прямо в текст самого промпта через OSC-последовательность).
//!
//! `<git/>` реализован НАТИВНО (без внешних зависимостей кроме самого git).
//! `<cmd run="...">` — это произвольная POSIX shell-команда из PSML-файла
//! (так и задумано в спеке PSML), для неё нет смысла транслировать
//! синтаксис на лету, поэтому она выполняется через `bash -c` (Git for
//! Windows ставит git-bash в PATH рядом с git, так что на практике это не
//! лишняя зависимость; на Linux/macOS bash есть почти всегда).

use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::{fish_quote_single, raw_sgr, resolve_sgr_color};
use crate::render::ShellBackend;

pub struct Fish;

impl ShellBackend for Fish {
    fn key(&self) -> &'static str {
        "fish"
    }

    fn render_document(&self, doc: &Document, raw: bool) -> Result<String, PsmlError> {
        let mut b = Builder::new();
        render_nodes(&doc.body, &mut b)?;
        let body_cmd = b.into_printf_command();
        Ok(finalize(&doc.title, &body_cmd, raw))
    }
}

/// Один кусочек промпта: либо буквальный текст (нужно квотить), либо уже
/// готовый кусок fish-кода (переменная/подстановка — вставляется как есть).
enum Frag {
    Lit(String),
    Expr(String),
}

struct Builder {
    frags: Vec<Frag>,
}

impl Builder {
    fn new() -> Self {
        Builder { frags: Vec::new() }
    }

    fn push_lit(&mut self, s: &str) {
        if let Some(Frag::Lit(last)) = self.frags.last_mut() {
            last.push_str(s);
        } else {
            self.frags.push(Frag::Lit(s.to_string()));
        }
    }

    fn push_expr(&mut self, s: String) {
        self.frags.push(Frag::Expr(s));
    }

    /// `printf '%s' arg1 arg2 ...` — литералы квотятся, выражения идут как есть.
    fn into_printf_command(self) -> String {
        if self.frags.is_empty() {
            return "printf '%s'".to_string();
        }
        let args: Vec<String> = self
            .frags
            .into_iter()
            .map(|f| match f {
                Frag::Lit(s) => fish_quote_single(&s),
                Frag::Expr(s) => s,
            })
            .collect();
        format!("printf '%s' {}", args.join(" "))
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
        Node::User => b.push_expr("$USER".to_string()),
        Node::Host => b.push_expr("$hostname".to_string()),
        Node::HostFull => b.push_expr("(hostname -f 2>/dev/null; or hostname)".to_string()),
        Node::Cwd => b.push_expr("(prompt_pwd)".to_string()),
        Node::CwdBase => b.push_expr("(basename $PWD)".to_string()),
        Node::Symbol => {
            b.push_expr("(if fish_is_root_user; echo '#'; else; echo '$'; end)".to_string())
        }
        Node::Jobs => b.push_expr("(jobs | count)".to_string()),
        Node::ExitCode => b.push_expr("$status".to_string()),
        Node::Reset => b.push_lit(&raw_sgr("0")),
        Node::Time(mode) => b.push_expr(time_expr(*mode)),
        Node::Date(fmt) => b.push_expr(date_expr(fmt.as_deref())),
        Node::Git { prefix, suffix } => b.push_expr(git_expr(prefix, suffix)),
        Node::Cmd(run) => b.push_expr(format!("(bash -c {})", fish_quote_single(run))),
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
    format!("(date {})", fish_quote_single(&format!("+{}", mode.strftime_fmt())))
}

fn date_expr(fmt: Option<&str>) -> String {
    let f = fmt.unwrap_or(crate::ir::DEFAULT_DATE_FMT);
    format!("(date {})", fish_quote_single(&format!("+{}", f)))
}

/// Нативная (без внешних зависимостей) реализация `<git/>` на fish.
fn git_expr(prefix: &str, suffix: &str) -> String {
    format!(
        "(set -l b (git symbolic-ref --short HEAD 2>/dev/null; or git rev-parse --short HEAD 2>/dev/null); test -n \"$b\"; and printf '%s%s%s' {} \"$b\" {})",
        fish_quote_single(prefix),
        fish_quote_single(suffix)
    )
}

fn finalize(title: &str, body_cmd: &str, raw: bool) -> String {
    if raw {
        return body_cmd.to_string();
    }
    let mut script = format!("function fish_prompt\n    {}\nend\n", body_cmd);
    if !title.is_empty() {
        script.push_str(&format!(
            "\nfunction fish_title\n    echo {}\nend\n",
            fish_quote_single(title)
        ));
    }
    script
}
