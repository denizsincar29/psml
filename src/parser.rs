//! Парсер PSML: `&str` -> [`Document`] (IR). Это единственное место в крейте,
//! которое знает про синтаксис PSML/XML. Оно понятия не имеет, что такое
//! bash, zsh или fish — этим занимаются модули в `render/`.
//!
//! Используется `quick-xml` как лексер тегов/атрибутов/сущностей (как
//! `html.parser` в питон-версии); вложенность стилевых тегов и построение
//! дерева — наша логика, через стек "рамок" [`Frame`].

use std::collections::HashMap;

use crate::err;
use crate::ir::{Document, Node, PsmlError, TimeMode};

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

struct Parser {
    shell: Option<String>,
    psml_seen: bool,
    psml_depth: i32,
    in_head: bool,
    in_title: bool,
    body_seen: bool,
    in_body: bool,
    title_parts: Vec<String>,
    body_root: Vec<Node>,
    frame_stack: Vec<Frame>,
}

impl Parser {
    fn new() -> Self {
        Parser {
            shell: None,
            psml_seen: false,
            psml_depth: 0,
            in_head: false,
            in_title: false,
            body_seen: false,
            in_body: false,
            title_parts: Vec::new(),
            body_root: Vec::new(),
            frame_stack: Vec::new(),
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

    fn open(&mut self, tag_raw: &str, attrs: &Attrs) -> Result<(), PsmlError> {
        let tag = tag_raw.to_lowercase();

        if tag == "psml" {
            if self.psml_depth == 0 && !self.psml_seen {
                self.psml_seen = true;
                let tag_shell = attrs.get("shell").cloned();
                self.shell = tag_shell;
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
                self.insert_node(Node::Git { prefix, suffix });
            }
            "cmd" => {
                let run = attrs.get("run").cloned();
                let run = match run {
                    Some(r) if !r.is_empty() => r,
                    _ => return Err(err!("<cmd> должен иметь атрибут run с shell-командой")),
                };
                self.insert_node(Node::Cmd(run));
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
        Ok(Document {
            shell: self.shell,
            title: self.title_parts.concat(),
            body: self.body_root,
        })
    }
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

/// Разбирает PSML-текст в [`Document`] (IR). Ничего не знает про конкретные
/// шеллы — `doc.shell` это просто сырое значение атрибута `<psml shell="...">`
/// (или `None`), интерпретация/валидация имени шелла — на стороне `render::resolve`.
pub fn parse_psml(text: &str) -> Result<Document, PsmlError> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = false;
    reader.config_mut().trim_text(false);

    let mut p = Parser::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| err!("ошибка разбора PSML: {}", e))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = tag_name(e.name().as_ref());
                let attrs = read_attrs(&e)?;
                p.open(&name, &attrs)?;
            }
            Event::Empty(e) => {
                // Самозакрывающийся тег — open+close атомарно: для
                // контейнерных тегов это даёт узел с пустыми children
                // (см. `self_closing_style_tag_auto_closes` в тестах),
                // для остальных — просто эквивалентно одиночному узлу.
                let name = tag_name(e.name().as_ref());
                let attrs = read_attrs(&e)?;
                p.open(&name, &attrs)?;
                p.close(&name)?;
            }
            Event::End(e) => {
                p.close(&tag_name(e.name().as_ref()))?;
            }
            Event::Text(t) => {
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
