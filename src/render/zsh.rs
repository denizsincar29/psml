//! zsh-бэкенд. Поведение побайтово совпадает с тем, что было до рефакторинга
//! на IR. В отличие от bash/fish/powershell/nu, zsh не использует голые
//! ANSI SGR-коды для цвета/стиля — у него есть "родные" токены (`%F{}`,
//! `%K{}`, `%B`, `%U`...), поэтому он не делит код с `render::util::resolve_sgr_color`
//! и живёт в собственном модуле целиком.

use crate::err;
use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::posix_quote_single;
use crate::render::ShellBackend;

pub struct Zsh;

impl ShellBackend for Zsh {
    fn key(&self) -> &'static str {
        "zsh"
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
        Node::Br => out.push('\n'),
        Node::User => out.push_str("%n"),
        Node::Host => out.push_str("%m"),
        Node::HostFull => out.push_str("%M"),
        Node::Cwd => out.push_str("%~"),
        Node::CwdBase => out.push_str("%1~"),
        Node::Symbol => out.push_str("%#"),
        Node::Jobs => out.push_str("%j"),
        Node::ExitCode => out.push_str("%?"),
        Node::Reset => out.push_str("%f%k%b%u%{\\e[0m%}"),
        Node::Time(mode) => out.push_str(time_code(*mode)),
        Node::Date(fmt) => match fmt {
            Some(f) => out.push_str(&format!("%D{{{}}}", f)),
            None => out.push_str("%D{%a %b %d}"),
        },
        Node::Git { prefix, suffix } => {
            *uses_subst = true;
            out.push_str(&wrap_subst(&git_cmd(prefix, suffix)));
        }
        Node::Cmd(run) => {
            *uses_subst = true;
            out.push_str(&wrap_subst(run));
        }
        Node::Bold(children) => emit_token_pair(children, "%B", "%b", out, uses_subst)?,
        Node::Underline(children) => emit_token_pair(children, "%U", "%u", out, uses_subst)?,
        Node::Italic(children) => emit_token_pair(
            children,
            "%{\\e[3m%}",
            "%{\\e[23m%}",
            out,
            uses_subst,
        )?,
        Node::Color { fg, bg, children } => emit_color(fg.as_deref(), bg.as_deref(), children, out, uses_subst)?,
    }
    Ok(())
}

fn emit_token_pair(
    children: &[Node],
    on: &str,
    off: &str,
    out: &mut String,
    uses_subst: &mut bool,
) -> Result<(), PsmlError> {
    out.push_str(on);
    for c in children {
        render_node(c, out, uses_subst)?;
    }
    out.push_str(off);
    Ok(())
}

fn emit_color(
    fg: Option<&str>,
    bg: Option<&str>,
    children: &[Node],
    out: &mut String,
    uses_subst: &mut bool,
) -> Result<(), PsmlError> {
    if let Some(f) = fg {
        out.push_str(&format!("%F{{{}}}", resolve_color_zsh(f)?));
    }
    if let Some(b) = bg {
        out.push_str(&format!("%K{{{}}}", resolve_color_zsh(b)?));
    }
    for c in children {
        render_node(c, out, uses_subst)?;
    }
    if fg.is_some() {
        out.push_str("%f");
    }
    if bg.is_some() {
        out.push_str("%k");
    }
    Ok(())
}

fn time_code(mode: TimeMode) -> &'static str {
    match mode {
        TimeMode::H24 => "%D{%H:%M:%S}",
        TimeMode::H12 => "%D{%I:%M:%S}",
        TimeMode::AmPm => "%D{%I:%M %p}",
        TimeMode::H24Short => "%D{%H:%M}",
    }
}

fn git_cmd(prefix: &str, suffix: &str) -> String {
    format!(
        "b=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); [ -n \"$b\" ] && printf \"%s%s%s\" \"{}\" \"$b\" \"{}\"",
        prefix, suffix
    )
}

/// В zsh нет MSYS2-бага bash-сборки (он специфичен для патча самого bash),
/// поэтому подстановка остаётся обычным `$(...)`, завязанным на `setopt
/// PROMPT_SUBST`.
fn wrap_subst(cmd: &str) -> String {
    format!("$({})", cmd)
}

fn zsh_plain_name(name: &str) -> bool {
    matches!(
        name,
        "black" | "red" | "green" | "yellow" | "blue" | "magenta" | "cyan" | "white"
    )
}

fn zsh_bright_num(name: &str) -> Option<i32> {
    Some(match name {
        "brightblack" => 8,
        "brightred" => 9,
        "brightgreen" => 10,
        "brightyellow" => 11,
        "brightblue" => 12,
        "brightmagenta" => 13,
        "brightcyan" => 14,
        "brightwhite" => 15,
        "gray" => 8,
        "grey" => 8,
        _ => return None,
    })
}

fn is_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Возвращает содержимое для `%F{...}` / `%K{...}` в zsh.
fn resolve_color_zsh(value: &str) -> Result<String, PsmlError> {
    if value.starts_with('#') && value.chars().count() == 7 {
        return Ok(value.to_string());
    }
    if is_ascii_digits(value) {
        return Ok(value.to_string());
    }
    let name = value.to_lowercase();
    if zsh_plain_name(&name) {
        return Ok(name);
    }
    if let Some(n) = zsh_bright_num(&name) {
        return Ok(n.to_string());
    }
    Err(err!("неизвестное имя цвета: {:?}", value))
}

fn finalize(title: &str, body: &str, raw: bool, uses_subst: bool) -> String {
    let mut full_prompt = body.to_string();
    if !title.is_empty() {
        full_prompt = format!("%{{\\e]0;{}\\a%}}{}", title, full_prompt);
    }
    if raw {
        return full_prompt;
    }
    let mut lines = Vec::new();
    if uses_subst {
        lines.push(
            "setopt PROMPT_SUBST  # нужно, чтобы $(...) внутри PROMPT вычислялся".to_string(),
        );
    }
    lines.push(format!("PROMPT={}", posix_quote_single(&full_prompt)));
    lines.join("\n")
}
