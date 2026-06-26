//! psml — конвертер PSML (Prompt String Markup Language) в PS1 (bash) / PROMPT (zsh).
//!
//! Это прямой порт `psml.py` на Rust. Логика разбора и генерации escape-строк
//! воспроизведена максимально близко к оригиналу (вплоть до сохранения тех же
//! "странностей" поведения, например того, что текстовые узлы попадают в
//! выходную строку независимо от того, находятся ли они внутри `<body>`).

use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Ошибка
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsmlError(pub String);

impl fmt::Display for PsmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PsmlError {}

macro_rules! err {
    ($($arg:tt)*) => { PsmlError(format!($($arg)*)) };
}

// ---------------------------------------------------------------------------
// Таблицы цветов
// ---------------------------------------------------------------------------

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

/// Возвращает SGR-параметр (часть для `\033[...m`) для bash/ANSI.
fn resolve_color_bash(value: &str, is_bg: bool) -> Result<String, PsmlError> {
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
    let code =
        fg_name_code(&name).ok_or_else(|| err!("неизвестное имя цвета: {:?}", value))?;
    Ok(if is_bg {
        (code + 10).to_string()
    } else {
        code.to_string()
    })
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

// ---------------------------------------------------------------------------
// Таблицы простых ("самозакрывающихся") тегов
// ---------------------------------------------------------------------------

fn bash_void(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "user" => "\\u",
        "host" => "\\h",
        "hostfull" => "\\H",
        "cwd" => "\\w",
        "cwdbase" => "\\W",
        "symbol" => "\\$",
        "jobs" => "\\j",
        "exitcode" => "$?",
        _ => return None,
    })
}

fn zsh_void(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "user" => "%n",
        "host" => "%m",
        "hostfull" => "%M",
        "cwd" => "%~",
        "cwdbase" => "%1~",
        "symbol" => "%#",
        "jobs" => "%j",
        "exitcode" => "%?",
        _ => return None,
    })
}

fn bash_attr_on(kind: &str) -> &'static str {
    match kind {
        "bold" => "1",
        "underline" => "4",
        "italic" => "3",
        _ => unreachable!(),
    }
}
fn bash_attr_off(kind: &str) -> &'static str {
    match kind {
        "bold" => "22",
        "underline" => "24",
        "italic" => "23",
        _ => unreachable!(),
    }
}
fn zsh_attr_on(kind: &str) -> &'static str {
    match kind {
        "bold" => "%B",
        "underline" => "%U",
        "italic" => "%{\\e[3m%}",
        _ => unreachable!(),
    }
}
fn zsh_attr_off(kind: &str) -> &'static str {
    match kind {
        "bold" => "%b",
        "underline" => "%u",
        "italic" => "%{\\e[23m%}",
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Разбор PSML через quick-xml
//
// quick-xml сам делает то, что в python-версии делал html.parser: лексит
// теги/атрибуты/текст и раскрывает сущности (&lt; &gt; &amp; &quot; &apos;,
// числовые &#NN;/&#xHH;). Отключаем `check_end_names`, потому что PSML (как
// и html.parser) толерантен к несовпадающим/пропущенным закрывающим тегам —
// например, `<br>` никогда не закрывается, а `<b>...</bold>` валиден, если
// смысловой алиас совпадает. Эту "смысловую" проверку вложенности стилей
// делает сам PsmlConverter (style_stack), а не парсер.
// ---------------------------------------------------------------------------

type Attrs = HashMap<String, String>;
const AUTO_CLOSE_STYLE_TAGS: &[&str] = &["bold", "b", "underline", "u", "italic", "i", "color"];

fn tag_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn read_attrs(start: &quick_xml::events::BytesStart) -> Result<Attrs, PsmlError> {
    let mut attrs = Attrs::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| err!("некорректный атрибут: {}", e))?;
        let name = tag_name(attr.key.as_ref()).to_lowercase();
        let value = attr
            .unescape_value()
            .map_err(|e| err!("некорректное значение атрибута: {}", e))?
            .into_owned();
        attrs.insert(name, value);
    }
    Ok(attrs)
}

fn parse(text: &str, conv: &mut PsmlConverter) -> Result<(), PsmlError> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = false;
    reader.config_mut().trim_text(false);

    loop {
        let event = reader
            .read_event()
            .map_err(|e| err!("ошибка разбора PSML: {}", e))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = tag_name(e.name().as_ref());
                let attrs = read_attrs(&e)?;
                conv.open(&name, &attrs)?;
            }
            Event::Empty(e) => {
                let name = tag_name(e.name().as_ref());
                let attrs = read_attrs(&e)?;
                conv.open(&name, &attrs)?;
                if AUTO_CLOSE_STYLE_TAGS.contains(&name.to_lowercase().as_str()) {
                    conv.close(&name)?;
                }
            }
            Event::End(e) => {
                conv.close(&tag_name(e.name().as_ref()))?;
            }
            Event::Text(t) => {
                let decoded = t.unescape().map_err(|e| err!("ошибка текста PSML: {}", e))?;
                conv.data(&decoded);
            }
            // комментарии, doctype, processing instructions, CDATA — игнорируем,
            // как и оригинальный html.parser (handle_comment/handle_decl и т.п.)
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Конвертер
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum StyleItem {
    Attr(&'static str),
    Color(Option<String>, Option<String>),
}

pub struct PsmlConverter {
    shell: Option<String>,
    psml_seen: bool,
    psml_depth: i32,
    in_head: bool,
    in_title: bool,
    body_seen: bool,
    in_body: bool,
    title_parts: Vec<String>,
    body_parts: Vec<String>,
    style_stack: Vec<StyleItem>,
    pub uses_subst: bool,
}

impl PsmlConverter {
    pub fn new(shell: Option<String>) -> Result<Self, PsmlError> {
        if let Some(s) = &shell {
            if s != "bash" && s != "zsh" {
                return Err(err!("shell должен быть 'bash' или 'zsh'"));
            }
        }
        Ok(PsmlConverter {
            shell,
            psml_seen: false,
            psml_depth: 0,
            in_head: false,
            in_title: false,
            body_seen: false,
            in_body: false,
            title_parts: Vec::new(),
            body_parts: Vec::new(),
            style_stack: Vec::new(),
            uses_subst: false,
        })
    }

    fn is_bash(&self) -> bool {
        self.shell.as_deref() == Some("bash")
    }

    fn push(&mut self, s: String) {
        if self.in_title {
            self.title_parts.push(s);
        } else {
            self.body_parts.push(s);
        }
    }

    fn data(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        if data.contains('\n') && data.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b')) {
            return;
        }
        self.push(data.to_string());
    }

    fn open(&mut self, tag_raw: &str, attrs: &Attrs) -> Result<(), PsmlError> {
        let tag = tag_raw.to_lowercase();

        if tag == "psml" {
            if self.psml_depth == 0 && !self.psml_seen {
                self.psml_seen = true;
                if self.shell.is_none() {
                    let tag_shell = attrs.get("shell").cloned();
                    let sh = tag_shell.unwrap_or_else(|| "bash".to_string());
                    if sh != "bash" && sh != "zsh" {
                        return Err(err!("<psml shell=...> неизвестный shell: {:?}", sh));
                    }
                    self.shell = Some(sh);
                }
            }
            self.psml_depth += 1;
            return Ok(());
        }

        if !self.psml_seen {
            return Err(err!("тег <{}> встречен до <psml>", tag));
        }

        if tag == "head" {
            self.in_head = true;
            return Ok(());
        }
        if tag == "title" {
            if !self.in_head {
                return Err(err!("<title> допустим только внутри <head>"));
            }
            self.in_title = true;
            return Ok(());
        }
        if tag == "body" {
            self.in_body = true;
            self.body_seen = true;
            return Ok(());
        }

        if !(self.in_head || self.in_body) {
            return Err(err!("тег <{}> вне <head>/<body>", tag));
        }

        if tag == "br" {
            self.push(if self.is_bash() { "\\n".to_string() } else { "\n".to_string() });
            return Ok(());
        }

        let void_val = if self.is_bash() {
            bash_void(&tag)
        } else {
            zsh_void(&tag)
        };
        if let Some(v) = void_val {
            self.push(v.to_string());
            return Ok(());
        }

        match tag.as_str() {
            "time" => {
                self.emit_time(attrs)?;
                return Ok(());
            }
            "date" => {
                self.emit_date(attrs);
                return Ok(());
            }
            "git" => {
                self.emit_git(attrs);
                return Ok(());
            }
            "cmd" => {
                self.emit_cmd(attrs)?;
                return Ok(());
            }
            "reset" => {
                self.emit_reset();
                return Ok(());
            }
            "bold" | "b" => {
                self.open_attr("bold");
                return Ok(());
            }
            "underline" | "u" => {
                self.open_attr("underline");
                return Ok(());
            }
            "italic" | "i" => {
                self.open_attr("italic");
                return Ok(());
            }
            "color" => {
                self.open_color(attrs)?;
                return Ok(());
            }
            _ => {}
        }

        Err(err!("неизвестный тег: <{}>", tag))
    }

    fn close(&mut self, tag_raw: &str) -> Result<(), PsmlError> {
        let tag = tag_raw.to_lowercase();
        match tag.as_str() {
            "psml" => self.psml_depth -= 1,
            "head" => self.in_head = false,
            "title" => self.in_title = false,
            "body" => self.in_body = false,
            "bold" | "b" => {
                self.expect_and_pop("bold")?;
                self.close_attr("bold");
            }
            "underline" | "u" => {
                self.expect_and_pop("underline")?;
                self.close_attr("underline");
            }
            "italic" | "i" => {
                self.expect_and_pop("italic")?;
                self.close_attr("italic");
            }
            "color" => {
                let (fg, bg) = self.pop_color()?;
                self.close_color(fg, bg);
            }
            // у остальных тегов (br, user, host, time, date, git, cmd, reset...)
            // закрывающих тегов не бывает — игнорируем, если вдруг написали
            _ => {}
        }
        Ok(())
    }

    fn expect_and_pop(&mut self, kind: &'static str) -> Result<(), PsmlError> {
        match self.style_stack.last() {
            Some(StyleItem::Attr(k)) if *k == kind => {
                self.style_stack.pop();
                Ok(())
            }
            _ => Err(err!("неправильное вложение тегов: ожидался конец </{}>", kind)),
        }
    }

    fn pop_color(&mut self) -> Result<(Option<String>, Option<String>), PsmlError> {
        match self.style_stack.last() {
            Some(StyleItem::Color(_, _)) => {
                if let Some(StyleItem::Color(fg, bg)) = self.style_stack.pop() {
                    Ok((fg, bg))
                } else {
                    unreachable!()
                }
            }
            _ => Err(err!("неправильное вложение тега </color>")),
        }
    }

    fn open_attr(&mut self, kind: &'static str) {
        self.style_stack.push(StyleItem::Attr(kind));
        if self.is_bash() {
            self.push(format!("\\[\\033[{}m\\]", bash_attr_on(kind)));
        } else {
            self.push(zsh_attr_on(kind).to_string());
        }
    }

    fn close_attr(&mut self, kind: &str) {
        if self.is_bash() {
            self.push(format!("\\[\\033[{}m\\]", bash_attr_off(kind)));
        } else {
            self.push(zsh_attr_off(kind).to_string());
        }
    }

    fn open_color(&mut self, attrs: &Attrs) -> Result<(), PsmlError> {
        let fg = attrs.get("fg").cloned();
        let bg = attrs.get("bg").cloned();
        if fg.is_none() && bg.is_none() {
            return Err(err!("<color> должен иметь атрибут fg и/или bg"));
        }
        self.style_stack.push(StyleItem::Color(fg.clone(), bg.clone()));
        if self.is_bash() {
            let mut codes = Vec::new();
            if let Some(ref f) = fg {
                codes.push(resolve_color_bash(f, false)?);
            }
            if let Some(ref b) = bg {
                codes.push(resolve_color_bash(b, true)?);
            }
            self.push(format!("\\[\\033[{}m\\]", codes.join(";")));
        } else {
            if let Some(ref f) = fg {
                self.push(format!("%F{{{}}}", resolve_color_zsh(f)?));
            }
            if let Some(ref b) = bg {
                self.push(format!("%K{{{}}}", resolve_color_zsh(b)?));
            }
        }
        Ok(())
    }

    fn close_color(&mut self, fg: Option<String>, bg: Option<String>) {
        if self.is_bash() {
            let mut codes = Vec::new();
            if fg.is_some() {
                codes.push("39".to_string());
            }
            if bg.is_some() {
                codes.push("49".to_string());
            }
            self.push(format!("\\[\\033[{}m\\]", codes.join(";")));
        } else {
            if fg.is_some() {
                self.push("%f".to_string());
            }
            if bg.is_some() {
                self.push("%k".to_string());
            }
        }
    }

    fn emit_time(&mut self, attrs: &Attrs) -> Result<(), PsmlError> {
        let mode = attrs.get("mode").cloned().unwrap_or_else(|| "24".to_string());
        let val = if self.is_bash() {
            match mode.as_str() {
                "24" => "\\t",
                "12" => "\\T",
                "ampm" => "\\@",
                "24short" => "\\A",
                _ => return Err(err!("<time mode=...>: неизвестный режим {:?}", mode)),
            }
        } else {
            match mode.as_str() {
                "24" => "%D{%H:%M:%S}",
                "12" => "%D{%I:%M:%S}",
                "ampm" => "%D{%I:%M %p}",
                "24short" => "%D{%H:%M}",
                _ => return Err(err!("<time mode=...>: неизвестный режим {:?}", mode)),
            }
        };
        self.push(val.to_string());
        Ok(())
    }

    fn emit_date(&mut self, attrs: &Attrs) {
        let fmt = attrs.get("fmt").cloned();
        if self.is_bash() {
            match fmt {
                Some(f) => self.push(format!("\\D{{{}}}", f)),
                None => self.push("\\d".to_string()),
            }
        } else {
            match fmt {
                Some(f) => self.push(format!("%D{{{}}}", f)),
                None => self.push("%D{%a %b %d}".to_string()),
            }
        }
    }

    fn emit_git(&mut self, attrs: &Attrs) {
        self.uses_subst = true;
        let prefix = attrs.get("prefix").cloned().unwrap_or_else(|| " (".to_string());
        let suffix = attrs.get("suffix").cloned().unwrap_or_else(|| ")".to_string());
        let cmd = format!(
            "b=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); [ -n \"$b\" ] && printf \"%s%s%s\" \"{}\" \"$b\" \"{}\"",
            prefix, suffix
        );
        self.push(self.wrap_subst(&cmd));
    }

    fn emit_cmd(&mut self, attrs: &Attrs) -> Result<(), PsmlError> {
        let run = attrs.get("run").cloned();
        let run = match run {
            Some(r) if !r.is_empty() => r,
            _ => return Err(err!("<cmd> должен иметь атрибут run с shell-командой")),
        };
        self.uses_subst = true;
        self.push(self.wrap_subst(&run));
        Ok(())
    }

    /// Оборачивает кусок shell-кода в подстановку для текущего шелла.
    ///
    /// Для bash используются backticks, а не `$(...)`. Это не стилистика —
    /// это обход давнего (с 2014, до сих пор не исправленного) бага bash
    /// именно в сборке MSYS2 (на которой держится git-bash/Git for Windows):
    /// если в PS1 после `$(...)`-подстановки где-то дальше в той же строке
    /// встречается "\n", парсер ломается с
    /// "syntax error near unexpected token `)'".
    /// См. <https://github.com/msys2/MSYS2-packages/issues/1839>.
    /// Backticks этот баг не задевают — именно поэтому в самом git-bash
    /// `__git_ps1` в дефолтном PS1 вызывается через `` `__git_ps1` ``,
    /// а не `$(__git_ps1)`. zsh этим багом не страдает, там оставляем `$(...)`.
    fn wrap_subst(&self, cmd: &str) -> String {
        if self.is_bash() {
            // внутри backticks один levels '\' перед `\`, '`' или '$'
            // экранирует символ — экранируем буквальные '\' и '`', чтобы
            // произвольная команда пользователя (`<cmd run="...">`) не
            // сломала обёртку и не самоэкранировалась незапланированно.
            let escaped = cmd.replace('\\', "\\\\").replace('`', "\\`");
            format!("`{}`", escaped)
        } else {
            format!("$({})", cmd)
        }
    }

    fn emit_reset(&mut self) {
        if self.is_bash() {
            self.push("\\[\\033[0m\\]".to_string());
        } else {
            self.push("%f%k%b%u%{\\e[0m%}".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Публичный API
// ---------------------------------------------------------------------------

/// Парсит PSML-текст, возвращает (title, body, shell, uses_subst).
pub fn psml_to_prompt(
    psml_text: &str,
    shell: Option<&str>,
) -> Result<(String, String, String, bool), PsmlError> {
    let mut conv = PsmlConverter::new(shell.map(|s| s.to_string()))?;
    parse(psml_text, &mut conv)?;

    if !conv.psml_seen {
        return Err(err!("не найден корневой тег <psml>"));
    }
    if conv.psml_depth != 0 {
        return Err(err!("тег <psml> не закрыт"));
    }
    if !conv.body_seen {
        return Err(err!("не найден тег <body>"));
    }
    if !conv.style_stack.is_empty() {
        return Err(err!("остались незакрытые стилевые теги"));
    }

    let title = conv.title_parts.concat();
    let body = conv.body_parts.concat();
    let shell_final = conv.shell.unwrap_or_else(|| "bash".to_string());
    Ok((title, body, shell_final, conv.uses_subst))
}

pub fn shell_quote_single(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn build_output(title: &str, body: &str, shell: &str, raw: bool, uses_subst: bool) -> String {
    let mut full_prompt = body.to_string();
    if !title.is_empty() {
        if shell == "bash" {
            full_prompt = format!("\\[\\033]0;{}\\007\\]{}", title, full_prompt);
        } else {
            full_prompt = format!("%{{\\e]0;{}\\a%}}{}", title, full_prompt);
        }
    }
    if raw {
        return full_prompt;
    }
    let var = if shell == "bash" { "PS1" } else { "PROMPT" };
    let mut lines = Vec::new();
    if shell == "zsh" && uses_subst {
        lines.push("setopt PROMPT_SUBST  # нужно, чтобы $(...) внутри PROMPT вычислялся".to_string());
    }
    lines.push(format!("{}={}", var, shell_quote_single(&full_prompt)));
    lines.join("\n")
}

/// Удобная обёртка: psml-текст -> готовая строка вывода (как stdout psml.py).
pub fn convert(psml_text: &str, shell: Option<&str>, raw: bool) -> Result<String, PsmlError> {
    let (title, body, shell_final, uses_subst) = psml_to_prompt(psml_text, shell)?;
    Ok(build_output(&title, &body, &shell_final, raw, uses_subst))
}
