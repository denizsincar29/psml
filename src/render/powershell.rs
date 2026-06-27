//! PowerShell-бэкенд (целимся в PowerShell 7+/`pwsh`: используется оператор
//! `??`, появившийся в 7.0, и автопеременные `$IsWindows`/`$IsLinux`,
//! появившиеся в 6.0 — на Windows PowerShell 5.1 часть выражений работать
//! не будет).
//!
//! Как и в fish, промпт — это функция (`function prompt { ... }`),
//! исполняемая заново при каждой отрисовке: никакого отдельного шага
//! "включить подстановки" (как zsh-овский `setopt PROMPT_SUBST`) не нужно.
//! Всё тело собирается как ОДНА interpolated double-quoted строка с
//! вкраплениями `$(...)` — PowerShell разбирает содержимое `$(...)` как
//! настоящий код независимо от внешнего экранирования, поэтому туда можно
//! без опасений вкладывать собственные кавычки/`{}`/`;`.
//!
//! `<git/>` реализован нативно через сам `git.exe` (он и так обязателен,
//! чтобы тег имел смысл). `<cmd run="...">` — это произвольная POSIX-команда
//! из PSML-файла, транслировать её синтаксис на лету не имеет смысла, так
//! что она выполняется через `bash -c` (на Windows это работает, если рядом
//! с git установлен Git for Windows/WSL; на Linux/macOS bash почти всегда
//! есть).
//!
//! `<date fmt="...">` в PSML — strftime-формат (так и в bash/zsh), а
//! `Get-Date -Format` в PowerShell — .NET custom date format, это два
//! разных языка форматирования. Конвертер `strftime_to_dotnet` переводит
//! между ними точечно для часто встречающихся спецификаторов и явно
//! ошибается на неизвестных — лучше понятная ошибка на этапе генерации,
//! чем молча неверная дата в промпте.

use crate::err;
use crate::ir::{Document, Node, PsmlError, TimeMode};
use crate::render::util::{powershell_escape_double, powershell_quote_single, raw_sgr, resolve_sgr_color};
use crate::render::ShellBackend;

pub struct PowerShell;

impl ShellBackend for PowerShell {
    fn key(&self) -> &'static str {
        "powershell"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["pwsh"]
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

    /// Буквальный текст — экранируем backtick/`"`/`$`, чтобы он не был
    /// принят за начало интерполяции внутри внешней `"..."` строки.
    fn push_lit(&mut self, s: &str) {
        self.out.push_str(&powershell_escape_double(s));
    }

    /// Готовый PowerShell-код (обычно `$(...)`) — вставляется как есть,
    /// PowerShell сам распарсит содержимое скобок как настоящий код.
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
        // ``n — родной escape новой строки PowerShell внутри "..."; не
        // вставляем сырой байт `\n`, чтобы исходник профиля оставался
        // в одну строку и не плодил визуально многострочный литерал.
        Node::Br => b.push_expr("`n"),
        Node::User => b.push_expr("$($env:USERNAME ?? $env:USER)"),
        Node::Host => b.push_expr("$($env:COMPUTERNAME ?? (hostname))"),
        Node::HostFull => b.push_expr("$([System.Net.Dns]::GetHostEntry('').HostName)"),
        Node::Cwd => b.push_expr(
            "$($pwd.Path -replace ('^' + [regex]::Escape($HOME)), '~')",
        ),
        Node::CwdBase => b.push_expr("$(Split-Path $pwd -Leaf)"),
        Node::Symbol => b.push_expr(SYMBOL_EXPR),
        Node::Jobs => b.push_expr("$((Get-Job).Count)"),
        // PowerShell не хранит код возврата процесса в `$?` (это bool
        // "успех/неуспех"), ближайший аналог — $LASTEXITCODE, но он
        // отражает только вызовы внешних .exe, а не каждую команду.
        Node::ExitCode => b.push_expr("$($LASTEXITCODE)"),
        Node::Reset => b.push_lit(&raw_sgr("0")),
        Node::Time(mode) => b.push_expr(&time_expr(*mode)),
        Node::Date(fmt) => b.push_expr(&date_expr(fmt.as_deref())?),
        Node::Git { prefix, suffix, .. } => b.push_expr(&git_expr(prefix, suffix)),
        Node::Cmd { run, .. } => b.push_expr(&format!("$(bash -c {})", powershell_quote_single(run))),
        Node::Bold(children) => emit_pair(children, "1", "22", b)?,
        Node::Underline(children) => emit_pair(children, "4", "24", b)?,
        Node::Italic(children) => emit_pair(children, "3", "23", b)?,
        Node::Color { fg, bg, children } => {
            emit_color(fg.as_deref(), bg.as_deref(), children, b)?
        }
    }
    Ok(())
}

const SYMBOL_EXPR: &str = "$(if($IsWindows){if(([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)){'#'}else{'$'}}else{if((id -u) -eq '0'){'#'}else{'$'}})";

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
    let fmt = match mode {
        TimeMode::H24 => "HH:mm:ss",
        TimeMode::H12 => "hh:mm:ss",
        TimeMode::AmPm => "hh:mm tt",
        TimeMode::H24Short => "HH:mm",
    };
    format!("$(Get-Date -Format {})", powershell_quote_single(fmt))
}

fn date_expr(fmt: Option<&str>) -> Result<String, PsmlError> {
    let dotnet = match fmt {
        Some(f) => strftime_to_dotnet(f)?,
        None => "ddd MMM dd".to_string(),
    };
    Ok(format!(
        "$(Get-Date -Format {})",
        powershell_quote_single(&dotnet)
    ))
}

/// `<git/>` — нативно, без посторонних зависимостей кроме самого git.
fn git_expr(prefix: &str, suffix: &str) -> String {
    format!(
        "$($b = git symbolic-ref --short HEAD 2>$null; if (-not $b) {{ $b = git rev-parse --short HEAD 2>$null }}; if ($b) {{ \"{}$b{}\" }})",
        powershell_escape_double(prefix),
        powershell_escape_double(suffix)
    )
}

/// Переводит strftime-формат (как в `<date fmt="...">`, документированном
/// в README_PSML.md) в формат `Get-Date -Format` (.NET custom date format).
/// Любой нераспознанный `%X` — явная ошибка, а не тихая неверная дата.
fn strftime_to_dotnet(fmt: &str) -> Result<String, PsmlError> {
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y') => out.push_str("yyyy"),
                Some('y') => out.push_str("yy"),
                Some('m') => out.push_str("MM"),
                Some('d') => out.push_str("dd"),
                Some('H') => out.push_str("HH"),
                Some('I') => out.push_str("hh"),
                Some('M') => out.push_str("mm"),
                Some('S') => out.push_str("ss"),
                Some('p') => out.push_str("tt"),
                Some('a') => out.push_str("ddd"),
                Some('A') => out.push_str("dddd"),
                Some('b') => out.push_str("MMM"),
                Some('B') => out.push_str("MMMM"),
                Some('%') => out.push_str("\\%"),
                Some(other) => {
                    return Err(err!(
                        "PowerShell: спецификатор даты %{} не поддержан конвертером strftime->.NET (доступны: Y y m d H I M S p a A b B %)",
                        other
                    ))
                }
                None => return Err(err!("PowerShell: формат даты заканчивается на одиночный '%'")),
            }
        } else if c == '\\' {
            out.push_str("\\\\");
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    Ok(out)
}

fn finalize(title: &str, body_inner: &str, raw: bool) -> String {
    let body_str = format!("\"{}\"", body_inner);
    if raw {
        return body_str;
    }
    let mut script = String::from("function prompt {\n");
    if !title.is_empty() {
        script.push_str(&format!(
            "    $Host.UI.RawUI.WindowTitle = {}\n",
            powershell_quote_single(title)
        ));
    }
    script.push_str(&format!("    {}\n", body_str));
    script.push_str("}\n");
    script
}
