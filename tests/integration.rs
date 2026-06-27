//! Интеграционные тесты: сравнение Rust-порта с оригинальным `psml.py`.
//!
//! Часть тестов сверяется с "замороженным" эталоном `test.ps1o` (не требует
//! python3 в системе вообще), часть — реально запускает `python_ref/psml.py`
//! через python3 и сравнивает вывод вживую (требует python3 в PATH; если его
//! нет, тест аккуратно skip'ается с сообщением, а не падает).

use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(manifest_dir().join(name))
        .unwrap_or_else(|e| panic!("не удалось прочитать {}: {}", name, e))
}

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Запускает оригинальный python_ref/psml.py с заданными доп. аргументами,
/// подавая test.psml на stdin. Возвращает stdout (или None, если python3 не
/// найден / скрипт не нашёлся — тест в этом случае skip'ается).
fn run_python_on_test_psml(extra_args: &[&str]) -> Option<String> {
    if !python3_available() {
        eprintln!("python3 не найден в PATH — skip динамической сверки с эталоном");
        return None;
    }
    let script = manifest_dir().join("python_ref/psml.py");
    let test_psml = manifest_dir().join("test.psml");
    let mut cmd = Command::new("python3");
    cmd.arg(&script).arg(&test_psml);
    cmd.args(extra_args);
    let output = cmd.output().expect("не удалось запустить python3");
    if !output.status.success() {
        panic!(
            "python_ref/psml.py завершился с ошибкой: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn rust_convert_test_psml(shell: Option<&str>, raw: bool) -> String {
    let text = read_fixture("test.psml");
    psml::convert(&text, shell, raw).expect("rust-конвертер не должен падать на test.psml")
}

/// Обратное преобразование к `wrap_subst()` из lib.rs для bash: конвертирует
/// `` `...` `` обратно в `$(...)`, чтобы можно было сравнить с "сырым"
/// выводом python (тот всегда использует `$(...)`, потому что в нём нет
/// обхода бага MSYS2-bash — см. komментарий у `wrap_subst`).
/// Нужно только тесту, в самой библиотеке этого нет и не должно быть.
fn backticks_to_dollar_paren(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '`' {
            let mut j = i + 1;
            let mut inner = String::new();
            while j < n && chars[j] != '`' {
                if chars[j] == '\\' && j + 1 < n && matches!(chars[j + 1], '`' | '\\') {
                    inner.push(chars[j + 1]);
                    j += 2;
                } else {
                    inner.push(chars[j]);
                    j += 1;
                }
            }
            out.push_str("$(");
            out.push_str(&inner);
            out.push(')');
            i = j + 1; // пропускаем закрывающий backtick
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Сверка со статическим эталоном test.ps1o (без python3 во время теста)
// ---------------------------------------------------------------------------

#[test]
fn matches_frozen_snapshot_default_bash() {
    // ВНИМАНИЕ: test.ps1o с момента фикса MSYS2-бага (см. wrap_subst() в
    // lib.rs) — это не сырой вывод python_ref/psml.py, а снапшот
    // УЖЕ-ПОФИКШЕННОГО rust-поведения (backticks вместо $(...) для
    // <git/>/<cmd/> в bash). Регенерируется через сам rust-бинарник.
    // Параллельная сверка с живым python — ниже, с нормализацией.
    let expected = read_fixture("test.ps1o");
    let expected = expected.trim_end_matches('\n');
    let actual = rust_convert_test_psml(None, false);
    assert_eq!(actual, expected, "вывод rust расходится с замороженным test.ps1o");
}

// ---------------------------------------------------------------------------
// Динамическая сверка "живого" python с rust по всем комбинациям флагов
// ---------------------------------------------------------------------------

#[test]
fn matches_live_python_default_bash() {
    // bash сознательно отличается от python: <git/>/<cmd/> заворачиваются в
    // backticks, а не в $(...) — обход бага MSYS2-bash (см. wrap_subst() в
    // lib.rs и https://github.com/msys2/MSYS2-packages/issues/1839).
    // Поэтому сравниваем после обратной нормализации backticks -> $(...).
    let Some(py_out) = run_python_on_test_psml(&[]) else { return };
    let py_out = py_out.trim_end_matches('\n');
    let rs_out = rust_convert_test_psml(None, false);
    assert_eq!(backticks_to_dollar_paren(&rs_out), py_out);
}

#[test]
fn matches_live_python_zsh() {
    let Some(py_out) = run_python_on_test_psml(&["--shell", "zsh"]) else { return };
    let py_out = py_out.trim_end_matches('\n');
    let rs_out = rust_convert_test_psml(Some("zsh"), false);
    assert_eq!(rs_out, py_out);
}

#[test]
fn matches_live_python_raw_bash() {
    // см. комментарий в matches_live_python_default_bash про backticks vs $(...)
    let Some(py_out) = run_python_on_test_psml(&["--raw"]) else { return };
    let py_out = py_out.trim_end_matches('\n');
    let rs_out = rust_convert_test_psml(None, true);
    assert_eq!(backticks_to_dollar_paren(&rs_out), py_out);
}

#[test]
fn matches_live_python_raw_zsh() {
    let Some(py_out) = run_python_on_test_psml(&["--shell", "zsh", "--raw"]) else { return };
    let py_out = py_out.trim_end_matches('\n');
    let rs_out = rust_convert_test_psml(Some("zsh"), true);
    assert_eq!(rs_out, py_out);
}

// ---------------------------------------------------------------------------
// Точечные эджкейсы, не требующие python3 (зафиксированные ожидания)
// ---------------------------------------------------------------------------

#[test]
fn minimal_doc() {
    let out = psml::convert("<psml><body><user/></body></psml>", None, false).unwrap();
    assert_eq!(out, "PS1='\\u'");
}

#[test]
fn whitespace_only_with_newline_is_dropped() {
    // Перевод строки + отступ между тегами — выбрасывается целиком.
    let out = psml::convert(
        "<psml><body><user/>\n    <host/></body></psml>",
        None,
        true,
    )
    .unwrap();
    assert_eq!(out, "\\u\\h");
}

#[test]
fn single_line_space_is_preserved() {
    // Пробел в пределах одной строки между тегами — сохраняется.
    let out = psml::convert("<psml><body><user/> <host/></body></psml>", None, true).unwrap();
    assert_eq!(out, "\\u \\h");
}

#[test]
fn entities_in_cmd_attr_are_decoded() {
    let out = psml::convert(
        r#"<psml><body><cmd run="a &amp;&amp; b &gt; c &lt; d &quot;q&quot;"/></body></psml>"#,
        None,
        true,
    )
    .unwrap();
    assert_eq!(out, "`a && b > c < d \"q\"`");
}

#[test]
fn bash_cmd_and_git_use_backticks_not_dollar_paren() {
    // Обход бага MSYS2-bash (git-bash/Git for Windows): $(...), за которым в
    // той же строке PS1 где-то дальше встречается "\n", ломает их парсер.
    // https://github.com/msys2/MSYS2-packages/issues/1839
    // backticks этот баг не задевают — так же, как `__git_ps1` в дефолтном
    // PS1 самого git-bash вызывается через backticks, а не $(...).
    //
    // Внутри самой команды git-индикатора $(...) всё равно остаётся (нужен
    // для "git symbolic-ref || git rev-parse" как обычная подстановка внутри
    // тела backtick-команды) — проверяем именно ВНЕШНЮЮ обёртку.
    let out = psml::convert(
        r#"<psml><body><git/><cmd run="echo hi"/><br/></body></psml>"#,
        None,
        true,
    )
    .unwrap();
    assert!(
        out.starts_with("`b=$(git symbolic-ref"),
        "<git/> должен быть обёрнут во внешние backticks: {}",
        out
    );
    assert!(
        out.contains("`echo hi`"),
        "<cmd run=\"echo hi\"/> должен быть обёрнут в backticks: {}",
        out
    );

    // а в zsh баг отсутствует (он специфичен для MSYS2-патча bash) — там
    // как и раньше используется $(...), завязанное на setopt PROMPT_SUBST.
    let out_zsh = psml::convert(
        r#"<psml shell="zsh"><body><git/><cmd run="echo hi"/><br/></body></psml>"#,
        None,
        true,
    )
    .unwrap();
    assert!(
        out_zsh.starts_with("$(b=$(git symbolic-ref"),
        "в zsh <git/> остаётся на $(...): {}",
        out_zsh
    );
    assert!(
        out_zsh.contains("$(echo hi)"),
        "в zsh <cmd run=\"echo hi\"/> остаётся на $(...): {}",
        out_zsh
    );
}

#[test]
fn self_closing_style_tag_auto_closes() {
    // <bold/> сразу открывает и закрывает стиль, не оставляя его "висящим".
    let out = psml::convert("<psml><body><bold/>x</body></psml>", None, true).unwrap();
    assert_eq!(out, "\\[\\033[1m\\]\\[\\033[22m\\]x");
}

#[test]
fn nested_color_and_bold() {
    let out = psml::convert(
        r#"<psml><body><color fg="green"><bold><user/></bold></color></body></psml>"#,
        None,
        true,
    )
    .unwrap();
    assert_eq!(
        out,
        "\\[\\033[32m\\]\\[\\033[1m\\]\\u\\[\\033[22m\\]\\[\\033[39m\\]"
    );
}

#[test]
fn zsh_uses_prompt_subst_when_git_used() {
    let out = psml::convert(r#"<psml shell="zsh"><body><git/></body></psml>"#, None, false)
        .unwrap();
    assert!(out.starts_with("setopt PROMPT_SUBST"));
}

#[test]
fn bash_does_not_need_prompt_subst() {
    let out = psml::convert(r#"<psml><body><git/></body></psml>"#, None, false).unwrap();
    assert!(!out.contains("setopt"));
}

// ---------------------------------------------------------------------------
// Эджкейсы-ошибки: достаточно того, что rust тоже считает их ошибкой
// ---------------------------------------------------------------------------

#[test]
fn errors_are_detected_like_in_python() {
    let bad_cases = [
        "<body><user/></body>",                                   // нет <psml>
        "<psml><body><foo/></body></psml>",                       // неизвестный тег
        "<psml><body><bold>x</italic></bold></body></psml>",      // неправильное вложение
        "<psml><body><color>x</color></body></psml>",             // нет fg/bg
        "<psml><body><color fg=\"#zzzzzz\">x</color></body></psml>", // битый hex
        "<psml><body><color fg=\"999\">x</color></body></psml>",  // вне диапазона
        "<psml><body><cmd/></body></psml>",                       // нет run
        "<psml><title>x</title><body></body></psml>",             // title вне head
        "<user/><psml><body></body></psml>",                      // тег до <psml>
        "<psml><body><bold>x</body></psml>",                      // незакрытый стиль
    ];
    for case in bad_cases {
        let res = psml::convert(case, None, false);
        assert!(res.is_err(), "ожидалась ошибка для: {}", case);
    }
}

// ---------------------------------------------------------------------------
// Реестр шеллов
// ---------------------------------------------------------------------------

#[test]
fn shell_keys_includes_all_backends() {
    let keys = psml::shell_keys();
    for expected in ["bash", "zsh", "fish", "powershell", "cmd", "nu"] {
        assert!(keys.contains(&expected), "в реестре нет {:?}: {:?}", expected, keys);
    }
}

#[test]
fn unknown_shell_errors() {
    let res = psml::convert("<psml><body><user/></body></psml>", Some("xyz"), false);
    assert!(res.is_err());
}

#[test]
fn powershell_alias_pwsh_resolves_to_same_backend() {
    let by_key = psml::find_backend("powershell").unwrap();
    let by_alias = psml::find_backend("pwsh").unwrap();
    assert_eq!(by_key.key(), by_alias.key());
}

#[test]
fn nu_alias_nushell_resolves_to_same_backend() {
    let by_key = psml::find_backend("nu").unwrap();
    let by_alias = psml::find_backend("nushell").unwrap();
    assert_eq!(by_key.key(), by_alias.key());
}

#[test]
fn cli_override_wins_over_doc_attribute() {
    // <psml shell="bash">, но явно просим zsh — должен победить явный аргумент.
    let out = psml::convert(
        r#"<psml shell="bash"><body><user/></body></psml>"#,
        Some("zsh"),
        true,
    )
    .unwrap();
    assert_eq!(out, "%n");
}

// ---------------------------------------------------------------------------
// fish: структурные проверки + (если fish установлен) реальная проверка
// синтаксиса через `fish -n` и реальное исполнение `fish_prompt`/`fish_title`.
// ---------------------------------------------------------------------------

fn fish_available() -> bool {
    Command::new("fish")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn fish_raw_is_just_printf_no_function_wrapper() {
    let out = psml::convert(
        r#"<psml><body><color fg="green"><user/></color></body></psml>"#,
        Some("fish"),
        true,
    )
    .unwrap();
    assert!(!out.contains("function"), "raw не должен включать обёртку: {}", out);
    assert!(out.starts_with("printf '%s'"));
    assert!(out.contains("$USER"));
}

#[test]
fn fish_non_raw_defines_fish_prompt_and_fish_title() {
    let out = psml::convert(
        r#"<psml><head><title>hi</title></head><body><user/></body></psml>"#,
        Some("fish"),
        false,
    )
    .unwrap();
    assert!(out.contains("function fish_prompt"));
    assert!(out.contains("function fish_title"));
    assert!(out.contains("echo 'hi'"));
}

#[test]
fn fish_cmd_run_shells_out_to_bash() {
    let out = psml::convert(
        r#"<psml><body><cmd run="echo hi"/></body></psml>"#,
        Some("fish"),
        true,
    )
    .unwrap();
    assert!(out.contains("(bash -c 'echo hi')"));
}

#[test]
fn fish_syntax_is_valid_for_full_fixture() {
    if !fish_available() {
        eprintln!("fish не найден в PATH — skip проверки синтаксиса");
        return;
    }
    let out = rust_convert_test_psml(Some("fish"), false);
    let tmp = std::env::temp_dir().join("psml_fish_syntax_check.fish");
    std::fs::write(&tmp, &out).unwrap();
    let status = Command::new("fish")
        .arg("-n")
        .arg(&tmp)
        .status()
        .expect("не удалось запустить fish -n");
    assert!(status.success(), "fish -n нашёл синтаксическую ошибку в:\n{}", out);
}

#[test]
fn fish_actually_executes_and_produces_text() {
    if !fish_available() {
        eprintln!("fish не найден в PATH — skip реального исполнения");
        return;
    }
    // Детерминированный документ — без <user/>/<host/>/<time/>, чтобы
    // проверить именно ЦВЕТ/СТИЛЬ + статический текст байт-в-байт.
    let psml = r#"<psml><body><color fg="green"><bold>hi</bold></color></body></psml>"#;
    let out = psml::convert(psml, Some("fish"), false).unwrap();
    let tmp = std::env::temp_dir().join("psml_fish_exec_check.fish");
    std::fs::write(&tmp, &out).unwrap();
    let result = Command::new("fish")
        .arg("-c")
        .arg(format!("source {}; fish_prompt", tmp.display()))
        .output()
        .expect("не удалось запустить fish -c");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout, "\u{1b}[32m\u{1b}[1mhi\u{1b}[22m\u{1b}[39m");
}

// ---------------------------------------------------------------------------
// PowerShell: только структурные проверки — в этой песочнице нет pwsh,
// чтобы реально исполнить (нет доступа к репозиториям Microsoft); код
// выверен вручную по документации, см. комментарии в render/powershell.rs.
// ---------------------------------------------------------------------------

#[test]
fn powershell_raw_is_just_the_string_literal() {
    let out = psml::convert(
        r#"<psml><body><color fg="green"><user/></color></body></psml>"#,
        Some("powershell"),
        true,
    )
    .unwrap();
    assert!(!out.contains("function prompt"));
    assert!(out.starts_with('"') && out.ends_with('"'));
    assert!(out.contains("$env:USERNAME"));
}

#[test]
fn powershell_non_raw_defines_prompt_function_and_sets_title() {
    let out = psml::convert(
        r#"<psml><head><title>hi</title></head><body><user/></body></psml>"#,
        Some("powershell"),
        false,
    )
    .unwrap();
    assert!(out.starts_with("function prompt {"));
    assert!(out.contains("$Host.UI.RawUI.WindowTitle = 'hi'"));
}

#[test]
fn powershell_cmd_run_shells_out_to_bash() {
    let out = psml::convert(
        r#"<psml><body><cmd run="echo hi"/></body></psml>"#,
        Some("powershell"),
        true,
    )
    .unwrap();
    assert!(out.contains("$(bash -c 'echo hi')"));
}

#[test]
fn powershell_date_fmt_converts_strftime_to_dotnet() {
    let out = psml::convert(
        r#"<psml><body><date fmt="%d.%m.%Y"/></body></psml>"#,
        Some("powershell"),
        true,
    )
    .unwrap();
    assert!(out.contains("dd\\.MM\\.yyyy"), "{}", out);
}

#[test]
fn powershell_date_fmt_unknown_specifier_errors() {
    let res = psml::convert(
        r#"<psml><body><date fmt="%Q"/></body></psml>"#,
        Some("powershell"),
        true,
    );
    assert!(res.is_err());
}

// ---------------------------------------------------------------------------
// cmd.exe: то, что поддержано — детерминированная проверка байт-в-байт;
// то, что архитектурно невозможно (живое выполнение команд) — явная ошибка.
// ---------------------------------------------------------------------------

#[test]
fn cmd_supported_features_render_exactly() {
    let out = psml::convert(
        r#"<psml><head><title>hi</title></head><body><color fg="green"><user/></color></body></psml>"#,
        Some("cmd"),
        false,
    )
    .unwrap();
    assert_eq!(out, "title hi\r\nprompt $E[32m%USERNAME%$E[39m\r\n");
}

#[test]
fn cmd_raw_has_no_title_or_prompt_wrapper() {
    let out = psml::convert(
        r#"<psml><head><title>hi</title></head><body><cwd/></body></psml>"#,
        Some("cmd"),
        true,
    )
    .unwrap();
    assert_eq!(out, "$P");
}

#[test]
fn cmd_cannot_run_live_commands() {
    for psml in [
        r#"<psml><body><git/></body></psml>"#,
        r#"<psml><body><cmd run="echo hi"/></body></psml>"#,
        r#"<psml><body><jobs/></body></psml>"#,
        r#"<psml><body><cwdbase/></body></psml>"#,
        r#"<psml><body><time mode="12"/></body></psml>"#,
        r#"<psml><body><date fmt="%Y"/></body></psml>"#,
    ] {
        let res = psml::convert(psml, Some("cmd"), true);
        assert!(res.is_err(), "ожидалась ошибка cmd.exe для: {}", psml);
    }
}

// ---------------------------------------------------------------------------
// nu: бонусный шелл, структурные smoke-проверки (см. оговорку
// "best-effort" в render/nu.rs).
// ---------------------------------------------------------------------------

#[test]
fn nu_non_raw_sets_prompt_command_closure() {
    let out = psml::convert(
        r#"<psml><body><user/></body></psml>"#,
        Some("nu"),
        false,
    )
    .unwrap();
    assert!(out.contains("$env.PROMPT_COMMAND = {|| "));
}

#[test]
fn nu_jobs_is_unsupported() {
    let res = psml::convert(r#"<psml><body><jobs/></body></psml>"#, Some("nu"), true);
    assert!(res.is_err());
}

#[test]
fn nu_cmd_run_shells_out_to_bash() {
    let out = psml::convert(
        r#"<psml><body><cmd run="echo hi"/></body></psml>"#,
        Some("nu"),
        true,
    )
    .unwrap();
    assert!(out.contains("(^bash -c r#'echo hi'#)"));
}
