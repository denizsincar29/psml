//! Парсер PSML: `&str` -> [`Document`] (IR). Это единственное место в крейте,
//! которое знает про синтаксис PSML/XML. Оно понятия не имеет, что такое
//! bash, zsh или fish, рендерится ли результат в shell-код или в превью —
//! этим занимаются `render/` и `preview.rs`.
//!
//! Единственное исключение — `target_shell`, который парсер ДОЛЖЕН знать
//! заранее (передаётся аргументом, не вычисляется по ходу разбора): именно
//! на этом этапе вычисляются preprocessing-теги `<if shell="...">`, а узнать
//! "для какого шелла мы сейчас генерируем" нужно ДО того, как до них дойдёт
//! очередь — раньше, чем закончится разбор всего документа. Имя `target_shell`
//! всегда конкретное (результат `render::resolve_shell`), а не "может быть
//! None" — даже `--preview` его получает (см. `main.rs`).
//!
//! Используется `quick-xml` как лексер тегов/атрибутов/сущностей (как
//! `html.parser` в питон-версии); вложенность стилевых тегов и построение
//! дерева — наша логика, через стек "рамок" [`Frame`].

use std::collections::HashMap;

use crate::err;
use crate::ir::{Document, ExecCheck, Node, PsmlError, TimeMode};
use crate::sysprobe::command_in_path;

type Attrs = HashMap<String, String>;

/// Канонический вид контейнерного (вкладываемого) тега — алиасы
/// (`b`=`bold`, `u`=`underline`, `i`=`italic`) сворачиваются в одно и то же.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Bold,
    Underline,
    Italic,
    Color,
}

impl ContainerKind {
    fn name(self) -> &'static str {
        match self {
            ContainerKind::Bold => "bold",
            ContainerKind::Underline => "underline",
            ContainerKind::Italic => "italic",
            ContainerKind::Color => "color",
        }
    }
}

struct Frame {
    kind: ContainerKind,
    fg: Option<String>,
    bg: Option<String>,
    children: Vec<Node>,
}

/// Что именно мы сейчас пропускаем (см. `skip_depth`) — нужно, чтобы по
/// выходу из пропуска правильно выставить `last_if_result` для `<else>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CondKind {
    If,
    Else,
}

struct Parser {
    target_shell: String,
    shell_attr: Option<String>,
    psml_seen: bool,
    psml_depth: i32,
    in_head: bool,
    in_title: bool,
    body_seen: bool,
    in_body: bool,
    title_parts: Vec<String>,
    body_root: Vec<Node>,
    frame_stack: Vec<Frame>,
    /// >0 — мы внутри содержимого `<if>`/`<else>`, условие которого false:
    /// все события (любые теги/текст) до соответствующего закрывающего тега
    /// просто отбрасываются, без какой-либо валидации содержимого —
    /// благодаря этому `<if shell="cmd">` может содержать что угодно,
    /// нерелевантное для остальных шеллов, и это не помешает их генерации.
    skip_depth: u32,
    skipping_kind: Option<CondKind>,
    /// Результат последнего ЗАКРЫВШЕГОСЯ `<if>` — если следующий значимый
    /// тег это `<else>`, он его читает (и инвертирует). Любой другой тег
    /// (или текст) сбрасывает это в `None` — `<else>` обязан идти сразу
    /// следом за `</if>`, не через произвольный контент.
    last_if_result: Option<bool>,
}

impl Parser {
    fn new(target_shell: String) -> Self {
        Parser {
            target_shell,
            shell_attr: None,
            psml_seen: false,
            psml_depth: 0,
            in_head: false,
            in_title: false,
            body_seen: false,
            in_body: false,
            title_parts: Vec::new(),
            body_root: Vec::new(),
            frame_stack: Vec::new(),
            skip_depth: 0,
            skipping_kind: None,
            last_if_result: None,
        }
    }

    fn insert_node(&mut self, node: Node) {
        if let Some(top) = self.frame_stack.last_mut() {
            top.children.push(node);
        } else {
            self.body_root.push(node);
        }
    }

    fn open_container(&mut self, kind: ContainerKind, fg: Option<String>, bg: Option<String>) {
        self.frame_stack.push(Frame { kind, fg, bg, children: Vec::new() });
    }

    fn close_container(&mut self, kind: ContainerKind) -> Result<(), PsmlError> {
        match self.frame_stack.last() {
            Some(f) if f.kind == kind => {
                let frame = self.frame_stack.pop().unwrap();
                let node = match kind {
                    ContainerKind::Bold => Node::Bold(frame.children),
                    ContainerKind::Underline => Node::Underline(frame.children),
                    ContainerKind::Italic => Node::Italic(frame.children),
                    ContainerKind::Color => Node::Color {
                        fg: frame.fg,
                        bg: frame.bg,
                        children: frame.children,
                    },
                };
                self.insert_node(node);
                Ok(())
            }
            _ => Err(err!(
                "неправильное вложение тегов: ожидался конец </{}>",
                kind.name()
            )),
        }
    }

    /// Парсит атрибуты `check`/`check-path`/`path`, общие для `<git>`/`<cmd>`.
    fn parse_exec_check(&self, attrs: &Attrs) -> Result<ExecCheck, PsmlError> {
        let level = match attrs.get("check") {
            Some(v) => crate::ir::CheckLevel::parse(v)?,
            None => crate::ir::CheckLevel::Off,
        };
        let check_path = match attrs.get("check-path") {
            Some(v) => parse_bool(v).ok_or_else(|| {
                err!("атрибут check-path: ожидалось true/false, получено {:?}", v)
            })?,
            None => true,
        };
        let path = attrs.get("path").cloned();
        Ok(ExecCheck { level, check_path, path })
    }

    /// Вычисляет условие `<if shell="..." command="...">` (оба
    /// необязательны, но хотя бы один должен быть, комбинируются через И).
    fn eval_if_attrs(&self, attrs: &Attrs) -> Result<bool, PsmlError> {
        let shell_attr = attrs.get("shell");
        let command_attr = attrs.get("command");
        if shell_attr.is_none() && command_attr.is_none() {
            return Err(err!(
                "<if> должен иметь хотя бы один из атрибутов: shell, command"
            ));
        }
        let mut result = true;
        if let Some(v) = shell_attr {
            result &= self.eval_shell_cond(v)?;
        }
        if let Some(v) = command_attr {
            result &= eval_command_cond(v)?;
        }
        Ok(result)
    }

    fn eval_shell_cond(&self, value: &str) -> Result<bool, PsmlError> {
        let (negated, entries) = parse_polarity_list(value)?;
        let matches = entries.iter().any(|e| e == &self.target_shell);
        Ok(if negated { !matches } else { matches })
    }

    fn open(&mut self, tag_raw: &str, attrs: &Attrs) -> Result<(), PsmlError> {
        let tag = tag_raw.to_lowercase();

        // <else> обязан идти сразу за закрывшимся <if> — любой другой тег
        // (включая новый <if>) рвёт эту цепочку.
        if tag != "else" {
            self.last_if_result = None;
        }

        if tag == "psml" {
            if self.psml_depth == 0 && !self.psml_seen {
                self.psml_seen = true;
                self.shell_attr = attrs.get("shell").cloned();
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

        if self.in_title {
            return Err(err!(
                "тег <{}> недопустим внутри <title>: <title> поддерживает только текст",
                tag
            ));
        }
        if self.in_head {
            return Err(err!(
                "тег <{}> внутри <head> допустим только в виде <title>",
                tag
            ));
        }
        if !self.in_body {
            return Err(err!("тег <{}> вне <body>", tag));
        }

        // <if>/<else> — preprocessing-теги: они НЕ попадают в дерево (ни
        // как узел, ни как контейнер) — либо их содержимое обрабатывается
        // совершенно прозрачно (как если бы обёртки не было), либо целиком
        // пропускается. См. doc-комментарий `skip_depth`.
        if tag == "if" {
            let cond = self.eval_if_attrs(attrs)?;
            if !cond {
                self.skip_depth = 1;
                self.skipping_kind = Some(CondKind::If);
            }
            return Ok(());
        }
        if tag == "else" {
            if !attrs.is_empty() {
                return Err(err!(
                    "<else> не принимает атрибуты — условие уже задано предшествующим <if>"
                ));
            }
            let prev = self
                .last_if_result
                .ok_or_else(|| err!("<else> без непосредственно предшествующего <if>"))?;
            self.last_if_result = None; // потребили — следующий <else> уже не пройдёт
            if prev {
                // if был true => else пропускаем целиком
                self.skip_depth = 1;
                self.skipping_kind = Some(CondKind::Else);
            }
            return Ok(());
        }

        match tag.as_str() {
            "br" => self.insert_node(Node::Br),
            "user" => self.insert_node(Node::User),
            "host" => self.insert_node(Node::Host),
            "hostfull" => self.insert_node(Node::HostFull),
            "cwd" => self.insert_node(Node::Cwd),
            "cwdbase" => self.insert_node(Node::CwdBase),
            "symbol" => self.insert_node(Node::Symbol),
            "jobs" => self.insert_node(Node::Jobs),
            "exitcode" => self.insert_node(Node::ExitCode),
            "reset" => self.insert_node(Node::Reset),
            "time" => {
                let mode_str = attrs.get("mode").cloned().unwrap_or_else(|| "24".to_string());
                let mode = TimeMode::parse(&mode_str)?;
                self.insert_node(Node::Time(mode));
            }
            "date" => {
                let fmt = attrs.get("fmt").cloned();
                self.insert_node(Node::Date(fmt));
            }
            "git" => {
                let prefix = attrs.get("prefix").cloned().unwrap_or_else(|| " (".to_string());
                let suffix = attrs.get("suffix").cloned().unwrap_or_else(|| ")".to_string());
                let check = self.parse_exec_check(attrs)?;
                self.insert_node(Node::Git { prefix, suffix, check });
            }
            "cmd" => {
                let run = attrs.get("run").cloned();
                let run = match run {
                    Some(r) if !r.is_empty() => r,
                    _ => return Err(err!("<cmd> должен иметь атрибут run с shell-командой")),
                };
                let check = self.parse_exec_check(attrs)?;
                self.insert_node(Node::Cmd { run, check });
            }
            "bold" | "b" => self.open_container(ContainerKind::Bold, None, None),
            "underline" | "u" => self.open_container(ContainerKind::Underline, None, None),
            "italic" | "i" => self.open_container(ContainerKind::Italic, None, None),
            "color" => {
                let fg = attrs.get("fg").cloned();
                let bg = attrs.get("bg").cloned();
                if fg.is_none() && bg.is_none() {
                    return Err(err!("<color> должен иметь атрибут fg и/или bg"));
                }
                self.open_container(ContainerKind::Color, fg, bg);
            }
            _ => return Err(err!("неизвестный тег: <{}>", tag)),
        }
        Ok(())
    }

    fn close(&mut self, tag_raw: &str) -> Result<(), PsmlError> {
        let tag = tag_raw.to_lowercase();
        match tag.as_str() {
            "psml" => self.psml_depth -= 1,
            "head" => self.in_head = false,
            "title" => self.in_title = false,
            "body" => self.in_body = false,
            // мы дошли сюда (а не через skip_depth-ветку в главном цикле) —
            // значит условие было true и контент обработан как обычно.
            "if" => self.last_if_result = Some(true),
            "else" => self.last_if_result = None,
            "bold" | "b" => self.close_container(ContainerKind::Bold)?,
            "underline" | "u" => self.close_container(ContainerKind::Underline)?,
            "italic" | "i" => self.close_container(ContainerKind::Italic)?,
            "color" => self.close_container(ContainerKind::Color)?,
            // у остальных тегов (br, user, host, time, date, git, cmd, reset...)
            // закрывающих тегов не бывает — игнорируем, если вдруг написали.
            _ => {}
        }
        Ok(())
    }

    /// Самозакрывающийся `<if/>`/`<else/>` — содержимого нет (значит, нечего
    /// ни включать, ни пропускать), но результат всё равно нужно
    /// зафиксировать на случай следующего `<else>`.
    fn handle_empty_if_else(&mut self, tag: &str, attrs: &Attrs) -> Result<(), PsmlError> {
        match tag {
            "if" => {
                let cond = self.eval_if_attrs(attrs)?;
                self.last_if_result = Some(cond);
            }
            "else" => {
                if !attrs.is_empty() {
                    return Err(err!(
                        "<else> не принимает атрибуты — условие уже задано предшествующим <if>"
                    ));
                }
                self.last_if_result
                    .ok_or_else(|| err!("<else> без непосредственно предшествующего <if>"))?;
                self.last_if_result = None;
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Вызывается, когда `skip_depth` дошёл обратно до 0 — то есть мы нашли
    /// закрывающий тег того самого `<if>`/`<else>`, с которого начался пропуск.
    fn finish_skip(&mut self) {
        match self.skipping_kind.take() {
            Some(CondKind::If) => self.last_if_result = Some(false),
            Some(CondKind::Else) => self.last_if_result = None,
            None => {}
        }
    }

    /// Текстовый узел. Как и в HTML, чисто форматирующий текст (отступ +
    /// перевод строки между тегами на разных строках исходника) выбрасывается
    /// целиком; пробел в пределах одной строки — осознанный, сохраняется.
    fn text(&mut self, data: &str) -> Result<(), PsmlError> {
        if data.is_empty() {
            return Ok(());
        }
        let whitespace_only_with_newline = data.contains('\n')
            && data.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b'));
        if whitespace_only_with_newline {
            return Ok(());
        }
        // непробельный текст рвёт цепочку <if>...</if><else> точно так же,
        // как и любой другой тег (см. open()).
        self.last_if_result = None;
        if self.in_title {
            self.title_parts.push(data.to_string());
            return Ok(());
        }
        if self.in_body {
            self.insert_node(Node::Text(data.to_string()));
            return Ok(());
        }
        if self.in_head {
            return Err(err!("текст вне <title> внутри <head>"));
        }
        Err(err!("текст вне <body>/<title>"))
    }

    fn finish(self) -> Result<Document, PsmlError> {
        if !self.psml_seen {
            return Err(err!("не найден корневой тег <psml>"));
        }
        if self.psml_depth != 0 {
            return Err(err!("тег <psml> не закрыт"));
        }
        if !self.body_seen {
            return Err(err!("не найден тег <body>"));
        }
        if !self.frame_stack.is_empty() {
            return Err(err!("остались незакрытые стилевые теги"));
        }
        if self.skip_depth != 0 {
            return Err(err!("не закрыт тег <if>/<else>"));
        }
        Ok(Document {
            shell: self.shell_attr,
            title: self.title_parts.concat(),
            body: self.body_root,
        })
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Разбирает список вида `a,b,c` или `!a,!b` (отрицание — либо у всех
/// элементов, либо ни у одного; смешивать нельзя, это неоднозначно).
/// Возвращает `(negated, entries)`.
fn parse_polarity_list(value: &str) -> Result<(bool, Vec<String>), PsmlError> {
    let parts: Vec<&str> = value.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Err(err!("пустой список значений в условии <if>"));
    }
    let negs: Vec<bool> = parts.iter().map(|p| p.starts_with('!')).collect();
    let all_neg = negs.iter().all(|&n| n);
    let none_neg = negs.iter().all(|&n| !n);
    if !all_neg && !none_neg {
        return Err(err!(
            "нельзя смешивать отрицание (!значение) и обычные значения в одном списке условия <if>"
        ));
    }
    let entries = parts
        .into_iter()
        .map(|p| p.strip_prefix('!').unwrap_or(p).trim().to_lowercase())
        .collect();
    Ok((all_neg, entries))
}

fn eval_command_cond(value: &str) -> Result<bool, PsmlError> {
    let (negated, entries) = parse_polarity_list(value)?;
    let any_available = entries.iter().any(|e| command_in_path(e));
    Ok(if negated { !any_available } else { any_available })
}

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

/// Заглядывает в самый первый тег документа (должен быть `<psml>`) и
/// возвращает его атрибут `shell`, если есть, ничего больше не разбирая.
/// Нужно, чтобы определить итоговый шелл (`render::resolve_shell`) ДО
/// полного разбора — а полный разбор (`parse_psml`) этот шелл как раз
/// требует на входе, чтобы вычислить `<if shell="...">`.
pub fn peek_shell_attr(text: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = false;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return None,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if !tag_name(e.name().as_ref()).eq_ignore_ascii_case("psml") {
                    return None;
                }
                return read_attrs(&e).ok()?.get("shell").cloned();
            }
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
}

/// Разбирает PSML-текст в [`Document`] (IR) для конкретного `target_shell`
/// (нужен только для вычисления `<if shell="...">` по ходу разбора — см.
/// doc-комментарий в начале файла). `doc.shell` в результате — это просто
/// сырое значение атрибута `<psml shell="...">` (или `None`) для справки;
/// само разрешение "какой шелл в итоге используем" уже произошло раньше,
/// через [`peek_shell_attr`] + `render::resolve_shell`.
pub fn parse_psml(text: &str, target_shell: &str) -> Result<Document, PsmlError> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = false;
    reader.config_mut().trim_text(false);

    let mut p = Parser::new(target_shell.to_lowercase());

    loop {
        let event = reader
            .read_event()
            .map_err(|e| err!("ошибка разбора PSML: {}", e))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = tag_name(e.name().as_ref());
                if p.skip_depth > 0 {
                    p.skip_depth += 1;
                    continue;
                }
                let attrs = read_attrs(&e)?;
                p.open(&name, &attrs)?;
            }
            Event::Empty(e) => {
                let name = tag_name(e.name().as_ref());
                if p.skip_depth > 0 {
                    // самозакрывающийся тег внутри пропуска — глубина не
                    // меняется (это Start+End в одном событии).
                    continue;
                }
                let attrs = read_attrs(&e)?;
                let lname = name.to_lowercase();
                if lname == "if" || lname == "else" {
                    p.handle_empty_if_else(&lname, &attrs)?;
                } else {
                    p.open(&name, &attrs)?;
                    p.close(&name)?;
                }
            }
            Event::End(e) => {
                if p.skip_depth > 0 {
                    p.skip_depth -= 1;
                    if p.skip_depth == 0 {
                        p.finish_skip();
                    }
                    continue;
                }
                p.close(&tag_name(e.name().as_ref()))?;
            }
            Event::Text(t) => {
                if p.skip_depth > 0 {
                    continue;
                }
                let decoded = t.unescape().map_err(|e| err!("ошибка текста PSML: {}", e))?;
                p.text(&decoded)?;
            }
            // комментарии, doctype, processing instructions, CDATA — игнорируем,
            // как и оригинальный html.parser (handle_comment/handle_decl и т.п.)
            _ => {}
        }
    }

    p.finish()
}
