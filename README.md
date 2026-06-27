# psml

**PSML** (Prompt String Markup Language) is a small HTML-like markup
language for describing a shell prompt. Write semantic markup once, get a
ready-to-use prompt for whichever shell you actually use.

```html
<psml>
  <head><title>my terminal</title></head>
  <body>
    <color fg="green"><bold><user/>@<host/></bold></color>
    <color fg="blue"><cwd/></color>
    <git prefix=" (" suffix=")"/>
    <symbol/>
  </body>
</psml>
```

```bash
$ psml prompt.psml --shell bash
PS1='\[\033]0;my terminal\007\]\[\033[32m\]\[\033[1m\]\u@\h\[\033[22m\]\[\033[39m\]\[\033[34m\]\w\[\033[39m\]`b=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null); [ -n "$b" ] && printf "%s%s%s" " (" "$b" ")"`\$'
```

Same file, different shell (the real output has a literal ESC byte where
this shows `\x1b` — written out as text here so it's actually readable):

```bash
$ psml prompt.psml --shell fish
function fish_prompt
    printf '%s' '\x1b[32m\x1b[1m' $USER '@' $hostname '\x1b[22m\x1b[39m\x1b[34m' (prompt_pwd) '\x1b[39m' (set -l b (git symbolic-ref --short HEAD 2>/dev/null; or git rev-parse --short HEAD 2>/dev/null); test -n "$b"; and printf '%s%s%s' ' (' "$b" ')') (if fish_is_root_user; echo '#'; else; echo '$'; end)
end

function fish_title
    echo 'my terminal'
end
```

## Why

Prompt configs tend to turn into an unreadable wall of escape codes that
only make sense to whoever wrote them, and have to be rewritten from
scratch for every shell. PSML lets you describe a prompt once, in a
structure that's actually readable, and generates correct output for
whatever shell — or shells — you need.

## Supported shells

| Shell | `--shell` | Live `<git/>` / `<cmd run>` | Notes |
|---|---|---|---|
| bash | `bash` (default) | yes (via backticks, see below) | |
| zsh | `zsh` | yes (`$(...)` + `setopt PROMPT_SUBST`) | |
| fish | `fish` | yes, native git / `bash -c` for `<cmd>` | prompt is a `fish_prompt`/`fish_title` function |
| PowerShell | `powershell` (alias `pwsh`) | yes, native git / `bash -c` for `<cmd>` | targets PowerShell 7+; prompt is a `prompt {}` function |
| cmd.exe | `cmd` | **no** — `PROMPT` is a static string, it can't run a command on every redraw | no `<jobs/>`, `<cwdbase/>`, 12-hour `<time/>`, or custom `<date fmt>` either, for the same reason |
| Nushell | `nu` (alias `nushell`) | yes, native git / `^bash -c` for `<cmd>` | bonus shell, best-effort — see `src/render/nu.rs` |

Run `psml --list-shells` to print this list straight from the binary.

If a tag genuinely can't be supported on a given shell (e.g. `<git/>` on
`cmd`), the converter fails with a clear explanation instead of silently
emitting a prompt that's subtly broken.

## Install

```bash
git clone https://github.com/denizsincar29/psml.git
cd psml
cargo build --release
```

This gives you `target/release/psml`.

## Usage

```bash
psml prompt.psml                     # bash by default, prints PS1='...'
psml prompt.psml --shell zsh         # prints PROMPT='...'
psml prompt.psml --shell fish        # prints a fish_prompt function
psml prompt.psml --shell powershell  # prints a prompt {} function
psml prompt.psml --shell cmd         # prints a `prompt ...` command
psml prompt.psml --raw               # just the prompt value, no wrapper
psml prompt.psml --preview           # actually run it and print a real preview right now
psml --list-shells
echo '<psml>...</psml>' | psml -     # read from stdin
psml                                  # no argument -> reads ~/ps1.psml
```

Hook it into your shell config:

```bash
# ~/.bashrc / ~/.zshrc
eval "$(~/psml/target/release/psml)"
```

```fish
# ~/.config/fish/config.fish
~/psml/target/release/psml --shell fish ~/ps1.psml | source
```

```powershell
# $PROFILE
& "$HOME\psml\target\release\psml.exe" --shell powershell "$HOME\ps1.psml" | Out-String | Invoke-Expression
```

```bat
:: cmd.exe — e.g. inside a .bat wrapper or an AutoRun key
for /f "delims=" %i in ('psml.exe ps1.psml --shell cmd') do %i
```

## Preview

Editing PSML by hand and reloading your shell every time to see if it looks
right gets old fast. `--preview` skips that loop entirely: it evaluates the
markup directly (not through any shell backend at all) and prints the
actual resulting prompt right into your terminal.

```bash
$ psml prompt.psml --preview
root@vm/home/you/code (main)$
```

It does this with **real, live values** wherever that's cheap and
meaningful: your actual username/hostname/cwd, the real current time/date,
and yes — `<git/>` and `<cmd run="...">` are genuinely executed (the same
way they're written to run inside a real prompt; running your own
`<cmd run>` here is no riskier than sourcing the same file into `.bashrc`).
The only exceptions are `<jobs/>` and `<exitcode/>`: those describe a
property of an *interactive shell session* (its job table, its last exit
status), not of the `psml` process itself, so there's nothing real to read
— they're shown as `0` with a one-line note appended if the document
actually uses them, rather than silently pretending it's a live value.

`--preview` ignores `--shell` and `--raw` — it isn't tied to any particular
shell's capabilities (or limitations: a `<git/>` that would fail to compile
for `cmd.exe` previews just fine, since preview never goes through the
`cmd` backend at all).

## The language

Full tag reference, attributes, and more examples (git status, venv
indicator, exit-code coloring, etc.) live in
[PSML.md](./PSML.md).

A quick taste of the available tags:

| Tag | Meaning |
|---|---|
| `<user/>` `<host/>` `<hostfull/>` | username / hostname / FQDN |
| `<cwd/>` `<cwdbase/>` | current directory, full or just the last component |
| `<symbol/>` `<jobs/>` `<exitcode/>` | `$`/`#` prompt char, background job count, last exit code |
| `<time mode="24\|12\|ampm\|24short"/>` `<date fmt="...">` | time / date |
| `<git prefix=".." suffix="..">` | current git branch |
| `<cmd run="...">` | output of an arbitrary shell command |
| `<bold>` `<underline>` `<italic>` `<color fg=".." bg="..">` | style, nestable |

## How it works

```
PSML text --[src/parser.rs]--> IR tree (src/ir.rs) --[src/render/*.rs]--> shell script
```

- **`src/ir.rs`** — a shell-agnostic tree (`Node`: `Text`, `User`, `Host`,
  `Color { fg, bg, children }`, `Git`, `Cmd`, ...). Container tags already
  hold their nested children, so there's no style stack to manage at
  render time.
- **`src/parser.rs`** — turns PSML text into that tree. This is the only
  place that knows anything about PSML/XML syntax; it has no idea what a
  shell even is.
- **`src/render/`** — one module per shell, each implementing the
  `ShellBackend` trait. `render/util.rs` holds the bits shared by several
  backends (quoting rules, the ANSI/SGR color resolver used by bash, fish,
  PowerShell, nu, and cmd).

Adding a new shell means writing a new `src/render/<name>.rs` and
registering it in `render::BACKENDS` — no changes to the parser, the IR, or
any other backend required.

`src/preview.rs` (the `--preview` flag) sits next to `render/`, not inside
it — it isn't a `ShellBackend`. Instead of generating code for a shell to
run later, it walks the same IR tree and evaluates it right now (real
`$USER`/cwd/git/`<cmd run>`, see the **Preview** section above), printing
ANSI text straight to the terminal.

## Testing

```bash
cargo test
```

- bash/zsh output is checked byte-for-byte against frozen snapshots for
  every `--shell`/`--raw` combination (`test*.ps1o`).
- fish output is checked with a real `fish -n` syntax check and by
  actually running `fish_prompt`, if `fish` is installed.
- cmd.exe has deterministic exact-output tests for what it supports, and
  tests asserting that the unsupported tags fail with a clear error.
- PowerShell/nu have structural smoke tests (no sandboxed access to a real
  `pwsh`/`nu` to execute against — the code has been reviewed by hand
  instead, see the comments in `src/render/powershell.rs` /
  `src/render/nu.rs`).
- `--preview` has its own tests, including one that actually shells out and
  checks the real output of a `<cmd run="...">`.

## License

MIT — see [LICENSE](./LICENSE).
