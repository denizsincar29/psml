//! Интеграционные тесты.
//!
//! `test.psml` — фикстура со всеми эджкейсами языка. Эталонные снапшоты
//! (`test*.ps1o`) — замороженный вывод самого rust-бинарника для разных
//! комбинаций `--shell`/`--raw`, проверены при заморозке (вручную и сверкой
//! с оригинальным python-эталоном, который как раз поэтому больше не нужен
//! держать в репозитории как живую зависимость теста) — дальше тест просто
//! следит, что вывод не разойдётся с зафиксированным при будущих правках.

use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(manifest_dir().join(name))
        .unwrap_or_else(|e| panic!("не удалось прочитать {}: {}", name, e))
}

fn rust_convert_test_psml(shell: Option<&str>, raw: bool) -> String {
    let text = read_fixture("test.psml");
    psml::convert(&text, shell, raw).expect("rust-конвертер не должен падать на test.psml")
}

fn assert_matches_fixture(fixture: &str, shell: Option<&str>, raw: bool) {
    let expected = read_fixture(fixture);
    let expected = expected.trim_end_matches('\n');
    let actual = rust_convert_test_psml(shell, raw);
    assert_eq!(actual, expected, "вывод rust расходится с замороженным {}", fixture);
}

// ---------------------------------------------------------------------------
// Сверка со статическими эталонами test*.ps1o
// ---------------------------------------------------------------------------

#[test]
fn matches_frozen_snapshot_default_bash() {
    assert_matches_fixture("test.ps1o", None, false);
}

#[test]
fn matches_frozen_snapshot_raw_bash() {
    assert_matches_fixture("test_bash_raw.ps1o", None, true);
}

#[test]
fn matches_frozen_snapshot_zsh() {
    assert_matches_fixture("test_zsh.ps1o", Some("zsh"), false);
}

#[test]
fn matches_frozen_snapshot_raw_zsh() {
    assert_matches_fixture("test_zsh_raw.ps1o", Some("zsh"), true);
}

// ---------------------------------------------------------------------------
// Точечные эджкейсы (зафиксированные ожидания)
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
// Эджкейсы-ошибки на уровне PSML/IR (не зависят от шелла)
// ---------------------------------------------------------------------------

#[test]
fn malformed_documents_are_rejected() {
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

// ---------------------------------------------------------------------------
// <if shell="...">/<else> — preprocessing-теги для многошелловых промптов
// ---------------------------------------------------------------------------

fn convert_raw(psml: &str, shell: &str) -> Result<String, psml::PsmlError> {
    psml::convert(psml, Some(shell), true)
}

#[test]
fn if_shell_picks_matching_branch_only() {
    let doc = r#"<psml><body><if shell="bash">A</if><if shell="zsh">B</if></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "A");
    assert_eq!(convert_raw(doc, "zsh").unwrap(), "B");
    assert_eq!(convert_raw(doc, "fish").unwrap(), "printf '%s'");
}

#[test]
fn if_shell_negation() {
    let doc = r#"<psml><body><if shell="!cmd">A</if></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "A");
    assert_eq!(convert_raw(doc, "cmd").unwrap(), "");
}

#[test]
fn if_shell_or_list() {
    let doc = r#"<psml><body><if shell="bash,zsh">A</if></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "A");
    assert_eq!(convert_raw(doc, "zsh").unwrap(), "A");
    assert_eq!(convert_raw(doc, "fish").unwrap(), "printf '%s'");
}

#[test]
fn if_shell_mixed_polarity_errors() {
    let doc = r#"<psml><body><if shell="bash,!zsh">A</if></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn if_without_shell_or_command_errors() {
    let doc = r#"<psml><body><if>A</if></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn else_renders_when_if_is_false() {
    let doc = r#"<psml><body><if shell="bash">A</if><else>B</else></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "A");
    assert_eq!(convert_raw(doc, "zsh").unwrap(), "B");
}

#[test]
fn else_without_preceding_if_errors() {
    let doc = r#"<psml><body><else>B</else></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn else_with_attributes_errors() {
    let doc = r#"<psml><body><if shell="zsh">A</if><else shell="bash">B</else></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn else_not_immediately_after_if_errors() {
    // между </if> и <else> затесался непробельный текст — цепочка рвётся.
    let doc = r#"<psml><body><if shell="zsh">A</if>X<else>B</else></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn self_closing_if_still_feeds_else() {
    let doc = r#"<psml><body><if shell="zsh"/><else>B</else></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "B");
    assert_eq!(convert_raw(doc, "zsh").unwrap(), "");
}

#[test]
fn skipped_if_branch_content_is_never_validated() {
    // <nonexistenttag/> внутри ветки для другого шелла не должен мешать
    // генерации для bash — он попросту никогда не разбирается.
    let doc = r#"<psml><body><if shell="cmd"><nonexistenttag/></if>ok</body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "ok");
    // а для cmd (где ветка реально раскрывается) это всё равно ошибка
    assert!(convert_raw(doc, "cmd").is_err());
}

#[test]
fn if_command_condition() {
    // sh почти наверняка есть везде, где это будет собрано и запущено
    let doc_has = r#"<psml><body><if command="sh">A</if></body></psml>"#;
    assert_eq!(convert_raw(doc_has, "bash").unwrap(), "A");
    let doc_missing =
        r#"<psml><body><if command="this-cmd-does-not-exist-zzz">A</if></body></psml>"#;
    assert_eq!(convert_raw(doc_missing, "bash").unwrap(), "");
}

#[test]
fn if_command_and_shell_combine_with_and() {
    let doc = r#"<psml><body><if shell="bash" command="this-cmd-does-not-exist-zzz">A</if></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "");
}

#[test]
fn nested_if_inside_true_branch_works_normally() {
    let doc = r#"<psml><body><if shell="bash"><if shell="bash">A</if></if></body></psml>"#;
    assert_eq!(convert_raw(doc, "bash").unwrap(), "A");
}

// ---------------------------------------------------------------------------
// <git>/<cmd> check/check-path/path — generation-time валидация
// ---------------------------------------------------------------------------

#[test]
fn check_off_by_default_never_runs_anything() {
    // команда не существует, но check не указан (по умолчанию "0") — не ошибка.
    let doc = r#"<psml><body><cmd run="this-cmd-does-not-exist-zzz"/></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_ok());
}

#[test]
fn check_path_catches_missing_command_with_level_1() {
    let doc =
        r#"<psml><body><cmd run="this-cmd-does-not-exist-zzz" check="1"/></body></psml>"#;
    let err = convert_raw(doc, "bash").unwrap_err();
    assert!(err.0.contains("не найдена"), "{}", err.0);
}

#[test]
fn check_level_2_warns_but_still_generates() {
    let doc =
        r#"<psml><body><cmd run="this-cmd-does-not-exist-zzz" check="2"/></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_ok());
}

#[test]
fn check_path_false_skips_path_lookup_and_runs_command_directly() {
    // "echo" — шелл-builtin sh, которого не найдёшь поиском по PATH как
    // отдельный исполняемый файл на некоторых системах; check-path=false
    // пропускает дорогую/бессмысленную здесь проверку и просто запускает.
    let doc = r#"<psml><body><cmd run="echo hi" check="1" check-path="false"/></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_ok());
}

#[test]
fn check_invalid_level_errors_at_parse_time() {
    let doc = r#"<psml><body><cmd run="echo hi" check="3"/></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_err());
}

#[test]
fn check_path_explicit_path_checked_instead_of_path_env() {
    let doc = r#"<psml><body><cmd run="echo hi" check="1" path="/definitely/not/a/real/binary"/></body></psml>"#;
    let err = convert_raw(doc, "bash").unwrap_err();
    assert!(err.0.contains("не найдена"), "{}", err.0);
}

#[test]
fn git_check_runs_real_git_version_check() {
    // git точно есть (используется тестами в других местах файла) —
    // check-path должен пройти; реальный результат symbolic-ref/rev-parse
    // зависит от того, git-репозиторий ли текущая директория тестового
    // процесса (то есть сам пакет psml) — а это так, значит check=1 пройдёт.
    let doc = r#"<psml><body><git check="1"/></body></psml>"#;
    assert!(convert_raw(doc, "bash").is_ok());
}

// ---------------------------------------------------------------------------
// --preview / render_preview: интерпретатор IR, а не shell-бэкенд — реально
// выполняет <git/>/<cmd run>/`date`, печатает готовый ANSI-текст.
// ---------------------------------------------------------------------------

fn render_preview(psml: &str) -> Result<String, psml::PsmlError> {
    let doc = psml::parse_psml(psml, "bash").expect("doc должен парситься");
    psml::render_preview(&doc)
}

#[test]
fn preview_renders_static_content_exactly() {
    let out = render_preview(
        r#"<psml><body><color fg="green"><bold>hi</bold></color></body></psml>"#,
    )
    .unwrap();
    assert_eq!(out, "\u{1b}[32m\u{1b}[1mhi\u{1b}[22m\u{1b}[39m");
}

#[test]
fn preview_shows_window_title_line() {
    let out = render_preview(r#"<psml><head><title>hi</title></head><body>x</body></psml>"#)
        .unwrap();
    assert!(out.starts_with("Заголовок окна: hi\n"));
}

#[test]
fn preview_actually_runs_cmd_run() {
    let out = render_preview(r#"<psml><body><cmd run="echo hi-from-shell"/></body></psml>"#)
        .unwrap();
    assert_eq!(out, "hi-from-shell");
}

#[test]
fn preview_notes_jobs_and_exitcode_are_placeholders() {
    let out = render_preview(r#"<psml><body><jobs/></body></psml>"#).unwrap();
    assert!(out.contains("jobs/"), "{}", out);
    assert!(out.contains("пример"), "{}", out);

    // а если этих тегов в документе нет — пометки быть не должно
    let out2 = render_preview(r#"<psml><body><user/></body></psml>"#).unwrap();
    assert!(!out2.contains("пример"));
}

#[test]
fn preview_errors_on_invalid_color_same_as_any_backend() {
    let res = render_preview(r#"<psml><body><color fg="999">x</color></body></psml>"#);
    assert!(res.is_err());
}

#[test]
fn preview_ignores_shell_capability_limits() {
    // <git/> и <cwdbase/> под --shell cmd должны падать у реального
    // cmd-бэкенда — но превью не бэкенд, оно не привязано к --shell вообще
    // и просто исполняет/вычисляет всё напрямую.
    let out = render_preview(r#"<psml shell="cmd"><body><cwdbase/></body></psml>"#);
    assert!(out.is_ok(), "{:?}", out);
}
