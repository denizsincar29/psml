use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::exit;

use psml::{convert, shell_keys, PsmlError};

const DEFAULT_PATH: &str = "~/ps1.psml";

fn expand_home(path: &str) -> String {
    if path == "~" {
        return env::var("HOME").unwrap_or_default();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

fn print_help() {
    println!("Конвертер PSML (Prompt String Markup Language) в готовый промпт для разных шеллов.");
    println!();
    println!("Использование:");
    println!("  psml [file] [--shell <шелл>] [--raw]");
    println!("  psml --list-shells");
    println!();
    println!(
        "  file           путь к .psml, '-' для stdin, или ничего (тогда {})",
        DEFAULT_PATH
    );
    println!(
        "  --shell <шелл> один из: {} (если не указан — берётся из <psml shell=\"..\">, иначе bash)",
        shell_keys().join(", ")
    );
    println!("  --raw          печатать только саму строку приглашения, без обвязки (PS1=.../function prompt {{...}}/...)");
    println!("  --list-shells  показать поддерживаемые шеллы и выйти");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut file_arg: Option<String> = None;
    let mut shell_arg: Option<String> = None;
    let mut raw = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--shell" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--shell требует значение ({})", shell_keys().join(", "));
                    exit(2);
                }
                shell_arg = Some(args[i].clone());
            }
            "--raw" => raw = true,
            "--list-shells" => {
                for key in shell_keys() {
                    println!("{}", key);
                }
                exit(0);
            }
            "-h" | "--help" => {
                print_help();
                exit(0);
            }
            other => {
                if file_arg.is_none() {
                    file_arg = Some(other.to_string());
                } else {
                    eprintln!("неожиданный аргумент: {}", other);
                    exit(2);
                }
            }
        }
        i += 1;
    }

    let (text, src_desc) = if file_arg.as_deref() == Some("-") {
        let mut s = String::new();
        if let Err(e) = io::stdin().read_to_string(&mut s) {
            eprintln!("не удалось прочитать stdin: {}", e);
            exit(1);
        }
        (s, "<stdin>".to_string())
    } else {
        let path = match &file_arg {
            Some(p) => p.clone(),
            None => expand_home(DEFAULT_PATH),
        };
        if !Path::new(&path).is_file() {
            let hint = if file_arg.is_none() {
                format!(
                    " (путь по умолчанию, передай файл явно или создай {})",
                    DEFAULT_PATH
                )
            } else {
                String::new()
            };
            eprintln!("Файл не найден: {}{}", path, hint);
            exit(1);
        }
        match fs::read_to_string(&path) {
            Ok(s) => (s, path),
            Err(e) => {
                eprintln!("не удалось прочитать файл {}: {}", path, e);
                exit(1);
            }
        }
    };

    match convert(&text, shell_arg.as_deref(), raw) {
        Ok(output) => println!("{}", output),
        Err(PsmlError(msg)) => {
            eprintln!("Ошибка PSML ({}): {}", src_desc, msg);
            exit(1);
        }
    }
}
