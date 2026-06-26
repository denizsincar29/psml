#!/usr/bin/env python3
"""
psml.py — конвертер PSML (Prompt String Markup Language) в PS1 (bash)
или PROMPT (zsh).

PSML — это псевдо-HTML для описания строки приглашения шелла.
Полная документация — в README.md рядом со скриптом. Краткая шпаргалка
по тегам — ниже.

Спецификация тегов
===================

Корень:
    <psml shell="bash|zsh"> ... </psml>
        атрибут shell необязателен (по умолчанию "bash"), его можно
        переопределить флагом --shell в командной строке (флаг главнее).

<!doctype psml> в начале файла необязателен, но допускается — парсер
просто проигнорирует любую <!DOCTYPE ...> декларацию.

<head>
    <title>...</title>   — заголовок окна терминала (просто текст)

<body>
    Самозакрывающиеся ("переменные") теги:
        <user/>                  — имя пользователя
        <host/>                  — короткое имя хоста
        <hostfull/>               — полное имя хоста (FQDN)
        <cwd/>                   — текущая директория (с заменой $HOME на ~)
        <cwdbase/>                — только последний компонент пути
        <symbol/>                 — $ для пользователя / # для root
        <jobs/>                   — число фоновых job'ов
        <exitcode/>                — код возврата последней команды
        <time mode="24|12|ampm|24short"/>  — текущее время
        <date fmt="%Y-%m-%d"/>     — дата (fmt необязателен)
        <git prefix=" (" suffix=")"/>      — имя текущей git-ветки
        <cmd run="shell-команда"/>          — вывод ЛЮБОЙ shell-команды
        <reset/>                  — сброс всех цветов/стилей
        <br>  или  <br/>          — перевод строки

    Контейнерные (стилевые) теги — оборачивают содержимое:
        <bold>...</bold>  (псевдоним <b>)
        <underline>...</underline>  (псевдоним <u>)
        <italic>...</italic>  (псевдоним <i>)
        <color fg="имя|0-255|#rrggbb" bg="имя|0-255|#rrggbb">...</color>

    Имена цветов: black, red, green, yellow, blue, magenta, cyan, white,
    а также bright-варианты: brightblack, brightred, ... brightwhite
    (синонимы gray/grey == brightblack).

Стилевые теги можно вкладывать друг в друга, например:
    <color fg="green"><bold><user/></bold></color>

Использование
=============
    python3 psml.py prompt.psml                  # bash, печатает "PS1='...'"
    python3 psml.py prompt.psml --shell zsh       # zsh, печатает "PROMPT='...'"
    python3 psml.py prompt.psml --raw             # печатает только саму строку
    python3 psml.py -                             # читать psml из stdin
    python3 psml.py                                # без аргумента — берёт ~/ps1.psml

Результат можно положить в ~/.bashrc или ~/.zshrc, например:
    eval "$(python3 ~/psml.py)"

Если используется <git/> или <cmd/>, для zsh обязательно нужен
`setopt PROMPT_SUBST` (скрипт сам допишет эту строку при --shell zsh).
Для bash ничего дополнительно настраивать не нужно — command substitution
в PS1 работает "из коробки" (опция promptvars включена по умолчанию).
"""

import os
import sys
import argparse
from html.parser import HTMLParser

DEFAULT_PATH = "~/ps1.psml"


class PsmlError(Exception):
    """Ошибка разбора PSML-документа."""


# --- таблица именованных цветов (значение = код SGR для foreground, 30-37/90-97) ---
FG_NAMES = {
    "black": 30, "red": 31, "green": 32, "yellow": 33,
    "blue": 34, "magenta": 35, "cyan": 36, "white": 37,
    "brightblack": 90, "brightred": 91, "brightgreen": 92, "brightyellow": 93,
    "brightblue": 94, "brightmagenta": 95, "brightcyan": 96, "brightwhite": 97,
    "gray": 90, "grey": 90,
}

# для zsh используем числовые коды 0-15 (0-7 обычные, 8-15 "яркие")
ZSH_BRIGHT_NUMS = {
    "brightblack": 8, "brightred": 9, "brightgreen": 10, "brightyellow": 11,
    "brightblue": 12, "brightmagenta": 13, "brightcyan": 14, "brightwhite": 15,
    "gray": 8, "grey": 8,
}
ZSH_PLAIN_NAMES = {"black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"}


def resolve_color_bash(value, is_bg):
    """Возвращает SGR-параметр (часть для \\033[...m) для bash/ANSI."""
    if value.startswith("#") and len(value) == 7:
        try:
            r = int(value[1:3], 16)
            g = int(value[3:5], 16)
            b = int(value[5:7], 16)
        except ValueError:
            raise PsmlError(f"некорректный hex-цвет: {value!r}")
        return f"{48 if is_bg else 38};2;{r};{g};{b}"
    if value.isdigit():
        n = int(value)
        if not (0 <= n <= 255):
            raise PsmlError(f"номер цвета должен быть 0-255, получено {value!r}")
        return f"{48 if is_bg else 38};5;{n}"
    name = value.lower()
    if name not in FG_NAMES:
        raise PsmlError(f"неизвестное имя цвета: {value!r}")
    code = FG_NAMES[name]
    return str(code + 10) if is_bg else str(code)


def resolve_color_zsh(value):
    """Возвращает содержимое для %F{...} / %K{...} в zsh."""
    if value.startswith("#") and len(value) == 7:
        return value  # zsh умеет %F{#rrggbb} напрямую
    if value.isdigit():
        return value
    name = value.lower()
    if name in ZSH_PLAIN_NAMES:
        return name
    if name in ZSH_BRIGHT_NUMS:
        return str(ZSH_BRIGHT_NUMS[name])
    raise PsmlError(f"неизвестное имя цвета: {value!r}")


# простые ("самозакрывающиеся") теги -> готовые escape-последовательности
BASH_VOID = {
    "user": "\\u", "host": "\\h", "hostfull": "\\H",
    "cwd": "\\w", "cwdbase": "\\W",
    "symbol": "\\$", "jobs": "\\j", "exitcode": "$?",
}
ZSH_VOID = {
    "user": "%n", "host": "%m", "hostfull": "%M",
    "cwd": "%~", "cwdbase": "%1~",
    "symbol": "%#", "jobs": "%j", "exitcode": "%?",
}

BASH_ATTR_ON = {"bold": "1", "underline": "4", "italic": "3"}
BASH_ATTR_OFF = {"bold": "22", "underline": "24", "italic": "23"}
ZSH_ATTR_ON = {"bold": "%B", "underline": "%U", "italic": "%{\\e[3m%}"}
ZSH_ATTR_OFF = {"bold": "%b", "underline": "%u", "italic": "%{\\e[23m%}"}


class PsmlConverter(HTMLParser):
    def __init__(self, shell=None):
        super().__init__(convert_charrefs=True)
        if shell is not None and shell not in ("bash", "zsh"):
            raise PsmlError("shell должен быть 'bash' или 'zsh'")
        self.shell = shell  # может быть None до встречи <psml shell="...">
        self.psml_seen = False
        self.psml_depth = 0
        self.in_head = False
        self.in_title = False
        self.body_seen = False
        self.in_body = False
        self.title_parts = []
        self.body_parts = []
        self.style_stack = []
        # True, если использован <git/> или <cmd/> — то есть в строке
        # появилась "живая" $(...) command substitution, которую шелл должен
        # пересчитывать заново при каждой отрисовке промпта.
        self.uses_subst = False

    @property
    def out(self):
        return self.title_parts if self.in_title else self.body_parts

    # --- точки входа HTMLParser ---
    def handle_starttag(self, tag, attrs):
        self._open(tag, dict(attrs))

    def handle_startendtag(self, tag, attrs):
        attrs = dict(attrs)
        self._open(tag, attrs)
        # на случай если кто-то напишет стилевой тег как самозакрывающийся,
        # типа <bold/> — сразу же закрываем, чтобы не оставлять "висящий" стиль
        if tag.lower() in ("bold", "b", "underline", "u", "italic", "i", "color"):
            self._close(tag)

    def handle_endtag(self, tag):
        self._close(tag)

    def handle_decl(self, decl):
        pass  # игнорируем <!doctype psml> и любые другие декларации

    def handle_comment(self, data):
        pass

    def handle_data(self, data):
        if not data:
            return
        # Текстовый узел, который целиком состоит из пробельных символов И
        # содержит перевод строки — это просто отступ/форматирование исходного
        # .psml файла (как лишние пробелы в HTML), а не осознанный пробел.
        # Его выбрасываем. А вот пробел внутри одной строки (например, между
        # </color> и <symbol/> на одной строке) — это осознанный пробел,
        # его сохраняем как есть.
        if "\n" in data and all(ch in " \t\r\n\f\v" for ch in data):
            return
        self.out.append(data)

    # --- открытие тегов ---
    def _open(self, tag, attrs, self_closing=False):
        tag = tag.lower()

        if tag == "psml":
            if self.psml_depth == 0 and not self.psml_seen:
                self.psml_seen = True
                tag_shell = attrs.get("shell")
                if self.shell is None:
                    self.shell = tag_shell if tag_shell else "bash"
                    if self.shell not in ("bash", "zsh"):
                        raise PsmlError(f"<psml shell=...> неизвестный shell: {self.shell!r}")
            self.psml_depth += 1
            return

        if not self.psml_seen:
            raise PsmlError(f"тег <{tag}> встречен до <psml>")

        if tag == "head":
            self.in_head = True
            return
        if tag == "title":
            if not self.in_head:
                raise PsmlError("<title> допустим только внутри <head>")
            self.in_title = True
            return
        if tag == "body":
            self.in_body = True
            self.body_seen = True
            return

        if not (self.in_head or self.in_body):
            raise PsmlError(f"тег <{tag}> вне <head>/<body>")

        if tag == "br":
            self.out.append("\\n" if self.shell == "bash" else "\n")
            return

        void_table = BASH_VOID if self.shell == "bash" else ZSH_VOID
        if tag in void_table:
            self.out.append(void_table[tag])
            return

        if tag == "time":
            self._emit_time(attrs)
            return
        if tag == "date":
            self._emit_date(attrs)
            return
        if tag == "git":
            self._emit_git(attrs)
            return
        if tag == "cmd":
            self._emit_cmd(attrs)
            return
        if tag == "reset":
            self._emit_reset()
            return

        if tag in ("bold", "b"):
            self._open_attr("bold")
            return
        if tag in ("underline", "u"):
            self._open_attr("underline")
            return
        if tag in ("italic", "i"):
            self._open_attr("italic")
            return
        if tag == "color":
            self._open_color(attrs)
            return

        raise PsmlError(f"неизвестный тег: <{tag}>")

    # --- закрытие тегов ---
    def _close(self, tag):
        tag = tag.lower()
        if tag == "psml":
            self.psml_depth -= 1
            return
        if tag == "head":
            self.in_head = False
            return
        if tag == "title":
            self.in_title = False
            return
        if tag == "body":
            self.in_body = False
            return
        if tag in ("bold", "b"):
            self._expect_and_pop("bold")
            self._close_attr("bold")
            return
        if tag in ("underline", "u"):
            self._expect_and_pop("underline")
            self._close_attr("underline")
            return
        if tag in ("italic", "i"):
            self._expect_and_pop("italic")
            self._close_attr("italic")
            return
        if tag == "color":
            fg, bg = self._pop_color()
            self._close_color(fg, bg)
            return
        # у остальных тегов (br, user, host, time, date, git, cmd, reset...)
        # закрывающих тегов не бывает — игнорируем, если вдруг написали

    # --- стиль: bold/underline/italic ---
    def _open_attr(self, kind):
        self.style_stack.append(kind)
        if self.shell == "bash":
            self.out.append(f"\\[\\033[{BASH_ATTR_ON[kind]}m\\]")
        else:
            self.out.append(ZSH_ATTR_ON[kind])

    def _close_attr(self, kind):
        if self.shell == "bash":
            self.out.append(f"\\[\\033[{BASH_ATTR_OFF[kind]}m\\]")
        else:
            self.out.append(ZSH_ATTR_OFF[kind])

    def _expect_and_pop(self, kind):
        if not self.style_stack or self.style_stack[-1] != kind:
            raise PsmlError(f"неправильное вложение тегов: ожидался конец </{kind}>")
        self.style_stack.pop()

    # --- стиль: color ---
    def _open_color(self, attrs):
        fg = attrs.get("fg")
        bg = attrs.get("bg")
        if not fg and not bg:
            raise PsmlError("<color> должен иметь атрибут fg и/или bg")
        self.style_stack.append(("color", fg, bg))
        if self.shell == "bash":
            codes = []
            if fg:
                codes.append(resolve_color_bash(fg, is_bg=False))
            if bg:
                codes.append(resolve_color_bash(bg, is_bg=True))
            self.out.append(f"\\[\\033[{';'.join(codes)}m\\]")
        else:
            if fg:
                self.out.append(f"%F{{{resolve_color_zsh(fg)}}}")
            if bg:
                self.out.append(f"%K{{{resolve_color_zsh(bg)}}}")

    def _pop_color(self):
        if not self.style_stack or self.style_stack[-1][0] != "color":
            raise PsmlError("неправильное вложение тега </color>")
        _, fg, bg = self.style_stack.pop()
        return fg, bg

    def _close_color(self, fg, bg):
        if self.shell == "bash":
            codes = []
            if fg:
                codes.append("39")
            if bg:
                codes.append("49")
            self.out.append(f"\\[\\033[{';'.join(codes)}m\\]")
        else:
            if fg:
                self.out.append("%f")
            if bg:
                self.out.append("%k")

    # --- время / дата ---
    def _emit_time(self, attrs):
        mode = attrs.get("mode", "24")
        if self.shell == "bash":
            table = {"24": "\\t", "12": "\\T", "ampm": "\\@", "24short": "\\A"}
        else:
            table = {
                "24": "%D{%H:%M:%S}", "12": "%D{%I:%M:%S}",
                "ampm": "%D{%I:%M %p}", "24short": "%D{%H:%M}",
            }
        if mode not in table:
            raise PsmlError(f"<time mode=...>: неизвестный режим {mode!r}")
        self.out.append(table[mode])

    def _emit_date(self, attrs):
        fmt = attrs.get("fmt")
        if self.shell == "bash":
            self.out.append(f"\\D{{{fmt}}}" if fmt else "\\d")
        else:
            self.out.append(f"%D{{{fmt}}}" if fmt else "%D{%a %b %d}")

    # --- git-ветка (готовый "шорткат" поверх <cmd/>) ---
    def _emit_git(self, attrs):
        self.uses_subst = True
        prefix = attrs.get("prefix", " (")
        suffix = attrs.get("suffix", ")")
        cmd = (
            "b=$(git symbolic-ref --short HEAD 2>/dev/null || "
            "git rev-parse --short HEAD 2>/dev/null); "
            f'[ -n "$b" ] && printf "%s%s%s" "{prefix}" "$b" "{suffix}"'
        )
        self.out.append(f"$({cmd})")

    # --- произвольная shell-команда ---
    def _emit_cmd(self, attrs):
        run = attrs.get("run")
        if not run:
            raise PsmlError("<cmd> должен иметь атрибут run с shell-командой")
        self.uses_subst = True
        # вставляем команду как есть внутрь $( ... ) — пользователь сам
        # отвечает за то, что в ней (включая редиректы вида 2>/dev/null)
        self.out.append(f"$({run})")

    # --- сброс стилей ---
    def _emit_reset(self):
        if self.shell == "bash":
            self.out.append("\\[\\033[0m\\]")
        else:
            self.out.append("%f%k%b%u%{\\e[0m%}")


def psml_to_prompt(psml_text, shell=None):
    """Парсит PSML-текст, возвращает (title, body, shell, uses_subst)."""
    conv = PsmlConverter(shell=shell)
    conv.feed(psml_text)
    conv.close()

    if not conv.psml_seen:
        raise PsmlError("не найден корневой тег <psml>")
    if conv.psml_depth != 0:
        raise PsmlError("тег <psml> не закрыт")
    if not conv.body_seen:
        raise PsmlError("не найден тег <body>")
    if conv.style_stack:
        raise PsmlError(f"остались незакрытые стилевые теги: {conv.style_stack}")

    title = "".join(conv.title_parts)
    body = "".join(conv.body_parts)
    return title, body, conv.shell, conv.uses_subst


def shell_quote_single(value):
    """Безопасно оборачивает строку в одинарные кавычки для shell."""
    return "'" + value.replace("'", "'\\''") + "'"


def build_output(title, body, shell, raw, uses_subst):
    full_prompt = body
    if title:
        if shell == "bash":
            full_prompt = f"\\[\\033]0;{title}\\007\\]" + full_prompt
        else:
            full_prompt = f"%{{\\e]0;{title}\\a%}}" + full_prompt

    if raw:
        return full_prompt

    var = "PS1" if shell == "bash" else "PROMPT"
    lines = []
    if shell == "zsh" and uses_subst:
        lines.append("setopt PROMPT_SUBST  # нужно, чтобы $(...) внутри PROMPT вычислялся")
    lines.append(f"{var}={shell_quote_single(full_prompt)}")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Конвертер PSML (Prompt String Markup Language) в PS1/PROMPT для bash/zsh."
    )
    parser.add_argument(
        "file", nargs="?", default=None,
        help=f"путь к .psml файлу, '-' для stdin, или ничего (берётся {DEFAULT_PATH})",
    )
    parser.add_argument(
        "--shell", choices=["bash", "zsh"], default=None,
        help="целевой шелл (если не указан — берётся из атрибута <psml shell=...>, иначе bash)",
    )
    parser.add_argument(
        "--raw", action="store_true",
        help="вывести только саму строку приглашения, без 'PS1=...'",
    )
    args = parser.parse_args()

    if args.file == "-":
        text = sys.stdin.read()
        src_desc = "<stdin>"
    else:
        path = args.file or os.path.expanduser(DEFAULT_PATH)
        src_desc = path
        if not os.path.isfile(path):
            hint = "" if args.file else f" (путь по умолчанию, передай файл явно или создай {DEFAULT_PATH})"
            print(f"Файл не найден: {path}{hint}", file=sys.stderr)
            sys.exit(1)
        with open(path, encoding="utf-8") as f:
            text = f.read()

    try:
        title, body, shell, uses_subst = psml_to_prompt(text, shell=args.shell)
    except PsmlError as e:
        print(f"Ошибка PSML ({src_desc}): {e}", file=sys.stderr)
        sys.exit(1)

    print(build_output(title, body, shell, args.raw, uses_subst))


if __name__ == "__main__":
    main()
