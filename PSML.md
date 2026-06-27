# PSML — Prompt String Markup Language

A small HTML-like markup language for describing a shell prompt. You write
markup, the converter turns it into a ready-to-use prompt (a string, a
function, or a closure, depending on the target shell).

```bash
psml example.psml --shell bash
# PS1='...'
```

See the top-level [README](./README.md) for the full list of supported
shells and their quirks/limitations (cmd.exe in particular can't do
everything the others can — that's covered there, not here).

## Document structure

```html
<!doctype psml>                 <!-- optional, just ignored -->
<psml shell="bash">             <!-- shell is optional, defaults to bash -->
  <head>
    <title>Terminal window title</title>
  </head>
  <body>
    ... the actual prompt markup ...
  </body>
</psml>
```

`shell` can be left out of the file and passed via `--shell` instead — the
command-line flag wins over the file attribute. Run `psml --list-shells`
to see the full set of accepted values.

`<title>` only supports plain text — no nested tags. There wouldn't be much
point anyway: a terminal window title doesn't render color or style.

## Tags

### Variables (self-closing)

| Tag | Meaning |
|---|---|
| `<user/>` | username |
| `<host/>` | short hostname |
| `<hostfull/>` | full hostname (FQDN) |
| `<cwd/>` | current directory (`$HOME` collapsed to `~`) |
| `<cwdbase/>` | just the last path component |
| `<symbol/>` | `$` for a regular user, `#` for root |
| `<jobs/>` | number of background jobs |
| `<exitcode/>` | exit code of the last command |
| `<time mode="24\|12\|ampm\|24short"/>` | current time |
| `<date fmt="%Y-%m-%d"/>` | date (`fmt` is optional, strftime-style) |
| `<git prefix=".." suffix="..">` | current git branch name |
| `<cmd run="command"/>` | output of **any** shell command (see below) |
| `<reset/>` | resets all color/style |
| `<br>` / `<br/>` | line break |

### Style tags (containers, nestable)

| Tag | Meaning |
|---|---|
| `<bold>...</bold>` (= `<b>`) | bold |
| `<underline>...</underline>` (= `<u>`) | underline |
| `<italic>...</italic>` (= `<i>`) | italic (not every terminal renders this) |
| `<color fg=".." bg="..">...</color>` | text / background color |

A color value is a name (`red`, `brightblue`, `gray`, ...), a number `0-255`
(256-color mode), or `#rrggbb` (24-bit truecolor).

Nesting example:

```html
<color fg="green"><bold><user/>@<host/></bold></color>
```

## Showing the output of an arbitrary command (e.g. git status)

The language has a built-in shortcut for showing the current git branch
(`<git/>`), but for anything else — dirty-tree status, an active venv, a
kubectl context, battery level — use `<cmd run="...">`. The `run` attribute
is a shell command; its output gets inserted right into the prompt:

```html
<color fg="red"><cmd run="git status --porcelain 2&gt;/dev/null | grep -q . &amp;&amp; printf '*'"/></color>
```

This shows a red `*` whenever the repo has uncommitted changes, and nothing
otherwise. You can hook up anything this way, for example:

```html
<!-- active python venv -->
<cmd run="[ -n &quot;$VIRTUAL_ENV&quot; ] &amp;&amp; printf '(%s) ' &quot;$(basename $VIRTUAL_ENV)&quot;"/>

<!-- color the prompt red if the last command failed -->
<cmd run="[ $? -ne 0 ] &amp;&amp; printf '\033[31m'"/>
```

**A note on `&lt;`, `&gt;`, `&amp;`, `&quot;`:** the `run` attribute lives
inside an HTML tag, so `<`, `&`, and whichever quote character wraps the
attribute need to be HTML-escaped (`&lt;` `&gt;` `&amp;` `&quot;`), same as
in regular HTML. `>` doesn't strictly need escaping, but escaping it too
(`&gt;`) keeps things symmetric and readable.

> **cmd.exe:** `<git/>` and `<cmd run="...">` under `--shell cmd` produce a
> clear error instead of a prompt that's almost-but-not-quite working —
> cmd.exe's `PROMPT` has no mechanism to re-run anything on every redraw
> (it's a static string), so these tags are architecturally impossible
> there. fish/PowerShell/Nushell all support them normally.

### Why this works live, not just once

`<git/>` and `<cmd/>` don't turn into pre-computed text — they turn into a
command substitution (e.g. `$(command)` in bash/zsh, or the shell's native
equivalent), embedded **directly inside** the generated prompt. `psml`
itself runs once, when you generate the prompt config — but the command
inside that substitution gets re-run by the shell on **every single
redraw**. That's the same mechanism every `__git_ps1`/powerline-style
prompt relies on, and it's why the git status shown is always current, not
frozen at the moment the shell started.

On bash this needs no extra setup (`promptvars` is on by default). On
**zsh** it requires `setopt PROMPT_SUBST` — if your file uses `<git/>` or
`<cmd/>`, `psml --shell zsh` adds that line to the output automatically.

## Whitespace and line breaks

Just like in HTML, indentation and line breaks in the source `.psml` file
are formatting, not part of the prompt. A text node that's **entirely**
whitespace — spaces/tabs/newlines, i.e. the indentation between tags on
different lines — gets dropped completely. A space within a single line
(say, between `</color>` and `<symbol/>`) is intentional and kept.

If you need a space right before a closing tag, keep it on the same line,
without a line break after it:

```html
<!-- the space after $ is kept -->
<symbol/> </body>

<!-- this one isn't — the line break after the space eats the whole node -->
<symbol/>
</body>
```

## Hooking it into your shell config

See the **Usage** section in the top-level [README](./README.md) for the
exact snippets for bash/zsh/fish/PowerShell/cmd.

The short version: `psml` itself runs **once**, when your shell starts up
(when `.bashrc`/`.zshrc`/etc. is sourced). What it outputs is then read
by the shell itself via `eval`/`source` — after that it's just a regular
prompt template (`\u`, `\h`, `\w`, command substitutions, ...) that the
shell expands on its own, on every redraw, with no `psml` process involved
at all.
