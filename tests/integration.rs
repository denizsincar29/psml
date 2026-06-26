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
