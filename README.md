# psml (Rust)

Rust-порт `psml.py` — конвертера PSML (Prompt String Markup Language) в
готовый промпт для разных шеллов. Полное описание самого языка — в
[`README_PSML.md`](./README_PSML.md) (взято из оригинального проекта,
синтаксис PSML не менялся).

Разбор тегов/атрибутов/сущностей делает [`quick-xml`](https://docs.rs/quick-xml) —
своего лексера тут нет, вся "своя" логика — это семантика самого PSML и
генерация под конкретный шелл.

## Поддерживаемые шеллы

| Шелл | `--shell` | Живые `<git/>`/`<cmd run>` | Примечания |
|---|---|---|---|
| bash | `bash` (по умолчанию) | да (через backticks, см. ниже) | |
| zsh | `zsh` | да (`$(...)` + `setopt PROMPT_SUBST`) | |
| fish | `fish` | да, нативно (git) / через `bash -c` (`<cmd>`) | промпт — функция `fish_prompt`/`fish_title` |
| PowerShell | `powershell` (алиас `pwsh`) | да, нативно (git) / через `bash -c` (`<cmd>`) | целимся в PowerShell 7+; промпт — функция `prompt {}` |
| cmd.exe | `cmd` | **нет** — `PROMPT` в cmd.exe статическая строка, выполнять команды при отрисовке не может | нет `<jobs/>`, `<cwdbase/>`, 12-часового `<time/>`, кастомного `<date fmt>` — см. `render/cmd.rs` |
| Nushell | `nu` (алиас `nushell`) | да, нативно (git) / через `^bash -c` (`<cmd>`) | бонусный шелл, best-effort (см. `render/nu.rs`), нет `<jobs/>` |

`psml --list-shells` печатает этот список из самого бинарника.

Каждый шелл — независимый бэкенд (см. "Архитектура" ниже); если в каком-то
PSML-файле используется тег, который выбранный шелл принципиально не может
поддержать (например `<git/>` под `cmd`), конвертер явно падает с понятным
объяснением — никогда не молчит и не генерирует промпт, который "почти
работает".

## Сборка и использование

```bash
cargo build --release
./target/release/psml prompt.psml                     # bash по умолчанию, печатает "PS1='...'"
./target/release/psml prompt.psml --shell zsh          # печатает "PROMPT='...'"
./target/release/psml prompt.psml --shell fish         # печатает "function fish_prompt ... end"
./target/release/psml prompt.psml --shell powershell   # печатает "function prompt { ... }"
./target/release/psml prompt.psml --shell cmd           # печатает "prompt $E[...]..."
./target/release/psml prompt.psml --raw                # только сама строка/выражение, без обвязки
./target/release/psml --list-shells                     # список поддержанных шеллов
echo '<psml>...</psml>' | ./target/release/psml -
./target/release/psml                                   # без аргумента — берёт ~/ps1.psml
```

В `~/.bashrc` / `~/.zshrc`:

```bash
eval "$(~/psml/target/release/psml)"
```

В `~/.config/fish/config.fish`:

```fish
~/psml/target/release/psml --shell fish ~/ps1.psml | source
```

В профиле PowerShell (`$PROFILE`):

```powershell
& ~/psml/target/release/psml --shell powershell ~/ps1.psml | Out-String | Invoke-Expression
```

В `autorun`-команде cmd.exe (например, через `HKCU\Software\Microsoft\Command Processor\AutoRun`)
или просто в начале `.bat`-обёртки:

```bat
for /f "delims=" %%i in ('psml.exe ps1.psml --shell cmd') do %%i
```
(построчно выполняет вывод — `title ...` и `prompt ...`).

## Архитектура

Три слоя, каждый знает только про соседний:

```
PSML-текст --[parser.rs]--> IR (ir.rs: Document/Node) --[render/*.rs]--> готовый скрипт
```

- **`src/ir.rs`** — дерево [`Node`]: `Text`, `User`, `Host`, `Color{fg,bg,children}`,
  `Git{prefix,suffix}`, `Cmd(run)`, и т.д. Контейнерные теги (`Bold`/`Underline`/
  `Italic`/`Color`) хранят уже вложенные дочерние узлы — никакого стека стилей
  на этапе рендера не нужно, вложенность зафиксирована структурой дерева.
- **`src/parser.rs`** — `parse_psml(&str) -> Result<Document, PsmlError>`. Цикл
  по событиям `quick-xml` (`Start`/`Empty`/`End`/`Text`), который строит дерево
  через стек "рамок" (`Frame`) для контейнерных тегов. Это единственное место,
  которое знает про синтаксис PSML/XML — оно понятия не имеет, что такое bash
  или fish.
- **`src/render/`** — трейт `ShellBackend` (`render_document(&Document, raw) ->
  Result<String, PsmlError>`) и по одному модулю на шелл: `bash.rs`, `zsh.rs`,
  `fish.rs`, `powershell.rs`, `cmd.rs`, `nu.rs`. `render/util.rs` — общие куски:
  кавычки разных шеллов (`posix_quote_single`, `fish_quote_single`,
  `powershell_quote_single`, ...) и общий резолвер ANSI/SGR-цвета
  (`resolve_sgr_color`) для всех бэкендов, которые в итоге пишут голые
  SGR-коды (bash/fish/powershell/nu/cmd — отличаются только текстовым
  обозначением самого ESC-байта). `render/mod.rs` — реестр (`BACKENDS`) и
  резолвинг имени шелла (`--shell` важнее `<psml shell="...">`, по умолчанию
  bash).
- **`src/lib.rs`** — тонкая склейка: `convert(text, shell, raw)` = `parse_psml`
  + `resolve_shell` + `backend.render_document`. Это весь публичный API,
  которым пользуются `main.rs` (CLI) и тесты.

### Как добавить ещё один шелл

1. Написать `src/render/<имя>.rs` с `impl ShellBackend for <Тип>` (по образцу
   `fish.rs` — самый "обычный" из новых; `cmd.rs` — пример, как явно
   отказываться от того, что шелл не может).
2. Добавить `&<имя>::<Тип>` в `render::BACKENDS` (`src/render/mod.rs`).

Ни парсер, ни IR, ни другие бэкенды трогать не нужно — `ir.rs`/`parser.rs`
уже умеют всё, что нужно любому шеллу.

### Почему `<cmd run="...">`/`<git/>` на fish/PowerShell/Nushell иногда зовут `bash -c`

`<cmd run="...">` — это произвольная POSIX shell-команда из самого PSML-файла
(см. примеры в `README_PSML.md` с `[ -n "$VAR" ] && ...`). Транслировать
произвольный bash-синтаксис в fish/PowerShell/nu-синтаксис на лету не имеет
смысла и было бы ненадёжно — поэтому для этих трёх шеллов такая команда
выполняется через `bash -c` (на Windows это работает, если рядом с git
установлен Git for Windows/Git Bash — он почти всегда есть, если есть сам
git; на Linux/macOS bash есть почти всегда). `<git/>` — наоборот, это
встроенная в PSML фича с чётко определённым смыслом, поэтому она
реализована НАТИВНО на каждом шелле (без зависимости от bash) — см.
`git_expr()` в соответствующем `render/*.rs`.

## Тесты

```bash
cargo test
```

- сверка rust-вывода для bash с замороженным `test.ps1o`;
- живая сверка rust vs `python3 python_ref/psml.py` для bash/zsh × `--raw`
  (если `python3` не найден в `PATH` — тест аккуратно skip'ается, а не падает);
- точечные тесты на эджкейсы PSML (whitespace-правило, сущности,
  самозакрывающиеся стили, вложенность, `PROMPT_SUBST`, реестр шеллов,
  алиасы `pwsh`/`nushell`);
- **fish**: если `fish` найден в `PATH` — реальная проверка синтаксиса
  (`fish -n`) на полном `test.psml` И реальное исполнение `fish_prompt`
  с проверкой точного байтового вывода;
- **cmd.exe**: детерминированные byte-exact проверки того, что поддержано,
  плюс проверка, что `<git/>`/`<cmd run>`/`<jobs/>`/`<cwdbase/>`/12-часовой
  `<time/>`/кастомный `<date fmt>` явно (а не молча) считаются ошибкой;
  исполнить реальный `cmd.exe` негде (Linux-песочница), но синтаксис
  `$`-кодов простой и хорошо документирован (`prompt /?`);
- **PowerShell/nu**: структурные smoke-тесты (нет доступа ни к репозиториям
  Microsoft, ни к боту, способному собрать `nu` за разумное время в этой
  песочнице) — код выверен вручную по документации, см. комментарии в
  `render/powershell.rs`/`render/nu.rs`. Если что-то не взлетит на вашей
  версии PowerShell/Nushell — это ожидаемо для самых "молодых" бэкендов,
  присылайте issue/PR.

## Фикс под git-bash/MSYS2: backticks вместо $(...) для `<git/>`/`<cmd/>` в bash

В bash-сборке MSYS2 (на которой держится git-bash / Git for Windows) есть
давний баг: если в `PS1`, заданном через `$(...)`-подстановку, где-то дальше
в той же строке встречается `\n`, парсер ломается с
`syntax error near unexpected token `)'`. Тикет открыт с 2014-го и до сих пор
не пофикшен в самом MSYS2: <https://github.com/msys2/MSYS2-packages/issues/1839>.
Воспроизводится в любом PSML-файле, где `<git/>` или `<cmd/>` стоят раньше
`<br/>` в bash-режиме (то есть почти всегда).

Поэтому для **bash** `<git/>`/`<cmd/>` оборачиваются в backticks, а не в
`$(...)` — ровно так же, как в дефолтном `PS1` самого git-bash вызывается
`` `__git_ps1` `` (а не `$(__git_ps1)`) специально по этой причине. Для
**zsh** этот баг не актуален (он специфичен для патча MSYS2 именно к bash),
там всё осталось на `$(...)` + `setopt PROMPT_SUBST`, как и в python.
fish/PowerShell/cmd/nu этот баг тоже не касается — он специфичен именно для
bash-сборки MSYS2.

Из-за этого rust-вывод для bash **намеренно** отличается от
`python_ref/psml.py` именно в этом месте — `python_ref/psml.py` оставлен
нетронутым (это твой реальный скрипт, трогать его без обратной связи не дело),
а тесты сверяют bash-вывод после обратной нормализации backticks → `$(...)`
(см. `backticks_to_dollar_paren()` в `tests/integration.rs`). zsh-вывод
сверяется как и раньше, байт-в-байт.

## Заметки о `quick-xml` vs `html.parser`

- `html.parser` в Python принимает практически любой "грязный" HTML, в том
  числе несовпадающие закрывающие теги (`<b>...</bold>`) и незакрытые
  void-теги (`<br>` без `/>` и без `</br>`). По умолчанию `quick-xml` такое
  бы зарубил как XML-ошибку — поэтому в `parser.rs` явно выставлено
  `reader.config_mut().check_end_names = false`: это снимает проверку имён
  закрывающих тегов на уровне лексера. Собственная проверка вложенности
  (нужна ли она вообще и совпадает ли *смысловой* алиас — `b`/`bold`,
  `i`/`italic` и т.п.) всё равно делается через стек "рамок" в `parser.rs`,
  как и в оригинале. Проверено тестом `errors_are_detected_like_in_python` +
  ручной сверкой с живым питоном.
- **Единственное оставшееся отличие**: `quick-xml` требует валидного
  XML-экранирования голого `&` даже в обычном тексте (не только в атрибутах) —
  `<body>Tom & Jerry</body>` он считает ошибкой, а python тихо пропускает
  как литеральный `&`. Сам PSML и так требует экранировать `&`/`<`/`"` в
  атрибутах (`<cmd run="...">`), так что на практике это не проблема, но
  если где-то в тексте промпта затесался "голый" `&` — экранируй его как
  `&amp;`.
- **Новое в этой версии**: `<title>` теперь поддерживает только текст (без
  вложенных тегов) — в оригинале тег внутри `<title>` тихо обрабатывался как
  стиль/escape-код, но результат утекал не в заголовок окна, а в тело
  промпта (баг в `in_title`-роутинге). Так это никогда осознанно не
  использовалось, так что это сознательное уточнение языка, а не потеря
  функциональности.
