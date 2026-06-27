//! Генерация-time проверки `<git check="..">`/`<cmd check="..">`.
//!
//! Это отдельный шаг между парсингом и рендером (см. `convert()` в
//! `lib.rs`) — не часть парсера (парсер не должен иметь побочных эффектов
//! типа запуска процессов) и не часть какого-то одного `ShellBackend`
//! (проверка "работает ли эта команда на этой машине" не зависит от того,
//! под какой шелл мы генерируем промпт).
//!
//! `--preview` (`preview.rs`) этот модуль не использует: там `<git/>`/
//! `<cmd run>` в любом случае выполняются по-настоящему, чтобы показать
//! живое значение — отдельная проверка "а сработает ли" там избыточна.

use std::process::{Command, Output};

use crate::ir::{CheckLevel, Document, ExecCheck, Node, PsmlError};
use crate::sysprobe::command_available;

pub fn run_checks(doc: &Document) -> Result<(), PsmlError> {
    check_nodes(&doc.body)
}

fn check_nodes(nodes: &[Node]) -> Result<(), PsmlError> {
    for n in nodes {
        check_node(n)?;
    }
    Ok(())
}

fn check_node(node: &Node) -> Result<(), PsmlError> {
    match node {
        Node::Git { check, .. } => check_git(check),
        Node::Cmd { run, check } => check_cmd(run, check),
        Node::Bold(c) | Node::Underline(c) | Node::Italic(c) => check_nodes(c),
        Node::Color { children, .. } => check_nodes(children),
        _ => Ok(()),
    }
}

fn check_git(check: &ExecCheck) -> Result<(), PsmlError> {
    if check.level.is_off() {
        return Ok(());
    }
    if check.check_path && !command_available("git", check.path.as_deref()) {
        return report(check.level, format!("<git/>: команда git не найдена{}", path_hint(check)), None);
    }
    // Проверяем именно ту команду, что попадёт в промпт (symbolic-ref с
    // фоллбэком на rev-parse) — это значит, что запуск "не в git-репозитории"
    // тоже будет считаться неуспехом. Если это нежелательно — используйте
    // check="0" (по умолчанию) или check="2" (предупреждение, не ошибка).
    let out = Command::new("sh")
        .arg("-c")
        .arg("git symbolic-ref --short HEAD || git rev-parse --short HEAD")
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => report(
            check.level,
            "<git/>: проверочный запуск завершился с ошибкой (возможно, текущая директория — не git-репозиторий)".to_string(),
            Some(&o),
        ),
        Err(e) => report(check.level, format!("<git/>: не удалось запустить проверку: {}", e), None),
    }
}

fn check_cmd(run: &str, check: &ExecCheck) -> Result<(), PsmlError> {
    if check.level.is_off() {
        return Ok(());
    }
    if check.check_path {
        let name = first_token(run);
        if !name.is_empty() && !command_available(name, check.path.as_deref()) {
            return report(
                check.level,
                format!("<cmd run={:?}>: команда {:?} не найдена{}", run, name, path_hint(check)),
                None,
            );
        }
    }
    let out = Command::new("sh").arg("-c").arg(run).output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => report(
            check.level,
            format!("<cmd run={:?}>: завершилась с кодом {:?}", run, o.status.code()),
            Some(&o),
        ),
        Err(e) => report(check.level, format!("<cmd run={:?}>: не удалось запустить: {}", run, e), None),
    }
}

fn first_token(run: &str) -> &str {
    run.split_whitespace().next().unwrap_or("")
}

fn path_hint(check: &ExecCheck) -> String {
    match &check.path {
        Some(p) => format!(" (искали по пути {:?})", p),
        None => " (искали в PATH)".to_string(),
    }
}

fn report(level: CheckLevel, message: String, output: Option<&Output>) -> Result<(), PsmlError> {
    let mut full = message;
    if let Some(o) = output {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let stderr = String::from_utf8_lossy(&o.stderr);
        if !stdout.trim().is_empty() {
            full.push_str(&format!("\n--- stdout ---\n{}", stdout.trim_end()));
        }
        if !stderr.trim().is_empty() {
            full.push_str(&format!("\n--- stderr ---\n{}", stderr.trim_end()));
        }
    }
    match level {
        CheckLevel::Error => Err(PsmlError(full)),
        CheckLevel::Warn => {
            eprintln!("предупреждение psml: {}", full);
            Ok(())
        }
        CheckLevel::Off => Ok(()),
    }
}
