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

## Writing one file for multiple shells

Some markup only makes sense for certain shells, or you want a different
`<cmd run="...">` per shell instead of relying on the automatic `bash -c`
fallback (see below). `<if>`/`<else>` are preprocessing tags: they're
resolved while parsing, before anything is rendered, and content in a
branch that doesn't apply is dropped *completely* — it's never even checked
for valid syntax, so it's fine to put shell-specific things in there that
wouldn't make sense for other shells.

```html
<if shell="bash,zsh,fish">
  <cmd run="git status --porcelain 2&gt;/dev/null | grep -q . &amp;&amp; printf ' *'"/>
</if>
<else>
  <git prefix=" (" suffix=")"/>
</else>
```

- `shell="bash,zsh"` — true if the target shell (`--shell`, or `<psml
  shell="...">`) is any of the listed ones. Prefix every entry with `!` to
  invert (`shell="!cmd"` — true for everything except cmd.exe; you can't mix
  `!a,b` in the same list, that's ambiguous, pick one).
- `command="docker"` — true if `docker` is found on `PATH` right now, on the
  machine generating the prompt (handy for dotfiles synced across machines
  that don't all have the same tools installed). Same comma-list/`!`
  negation rules as `shell`.
- Give both attributes and they're combined with AND.
- `<else>` applies to whatever the immediately preceding `<if>` resolved to
  (inverted) — nothing else allowed in between, not even unrelated tags;
  it doesn't take its own attributes.

This is also the answer to "why does `<cmd run>` need bash on PowerShell at
all" — it doesn't, if you don't want it to: write the bash-only version
inside `<if shell="bash,zsh,fish">` and a native PowerShell one elsewhere.
`<git/>` itself never needs this — it's already implemented natively on
every shell that can run it at all (everything except cmd.exe).

## Sanity-checking `<git>`/`<cmd run>` while generating

`<git>` and `<cmd run="...">` both accept three more attributes, useful for
catching a typo or a missing dependency immediately instead of discovering
it the next time you open a terminal:

```html
<cmd run="kubectl config current-context" check="1"/>
```

- `check="0"` (default) — don't check anything, just generate.
- `check="1"` — actually run the command **right now, while generating**;
  if it exits non-zero, print its stdout/stderr and abort generation
  entirely (no output at all, non-zero exit for `psml` itself).
- `check="2"` — same, but only a warning on failure; generation still
  succeeds.
- `check-path="true"` (default) — before running anything, cheaply check
  that the command actually exists (on `PATH`, or at `path="..."` if you
  give one) — catches "not installed on this machine" without the cost or
  side effects of actually running it.

Two things worth knowing before reaching for `check="1"`: it runs in
whatever directory you happen to run `psml` from (typically your dotfiles
repo) — for `<git>` that means "not currently in a git repo" *also* counts
as a failure, which is usually not what you want for a tag whose entire
point is "show nothing outside a repo". And patterns like
`<cmd run="[ $? -ne 0 ] &amp;&amp; printf ...">` exit non-zero *on purpose*
when there's nothing to print — `check="1"`/`check="2"` aren't a good fit
for those either. `check` is best suited to commands that are either
genuinely broken or genuinely fine, like the `kubectl` example above.

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

See the **Usage** and **Preview** sections in the top-level
[README](./README.md) for the exact snippets for bash/zsh/fish/PowerShell/
cmd, and for `--preview` — a flag that actually runs `<git/>`/`<cmd run>`
and prints the resulting prompt right now, without touching your shell
config at all.

The short version: `psml` itself runs **once**, when your shell starts up
(when `.bashrc`/`.zshrc`/etc. is sourced). What it outputs is then read
by the shell itself via `eval`/`source` — after that it's just a regular
prompt template (`\u`, `\h`, `\w`, command substitutions, ...) that the
shell expands on its own, on every redraw, with no `psml` process involved
at all.
