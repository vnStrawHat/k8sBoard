# Screenshot script (`--script`)

A screenshot build (`--features screenshot`) can walk a flow instead of taking one picture.
`--script <file>` needs `--screenshot <path>`; the shots land in the folder of that path.

```
k8sboard --kubeconfig kind-lab.yml --context kind-k8sboard-lab --screen pods --namespace lab-shop \
  --script walk.txt --screenshot .tmp/walk/final.png
```

## File format

Plain text, one step per line. Blank lines and lines starting with `#` are ignored.

| Step | Effect |
|---|---|
| `wait settled` | Waits until the launch screen settles (the rule of the final capture, same 60 s timeout; a timeout fails the run) |
| `wait <ms>` | Fixed delay |
| `key <keystroke>` | Key down and up to the focused control, in `Keystroke::parse` syntax: `j`, `enter`, `escape`, `ctrl-k`, `shift-j`, `space`, `alt-left` |
| `type <text>` | Types the characters into the focused text input (the rest of the line, spaces kept) |
| `click <x>,<y>` / `rclick <x>,<y>` | Left / right button down and up at window pixels |
| `hover <x>,<y>` | Moves the pointer there and waits for a tooltip |
| `scroll <x>,<y> <dy>` | Turns the mouse wheel at that point; positive `dy` scrolls the content down, in window pixels |
| `shot <name>` | Saves the window to `<dir>/<name>.png` (`name`: letters, digits, `-`, `_`) |
| `expect <text>` | Fails unless one of the shell's reported texts contains `<text>` |

Each step logs `step N: <line>` to stderr. After an input step the player waits 200 ms.
The row menu and other pop-ups with no key open with `rclick` (find the pixels in a `shot` first).

## `expect` limit

Rendered text is not readable, so `expect` checks what `AppShell::reported_texts` reports: `screen <Screen>`,
`cursor <ResourceKey>`, `drawer <ResourceKey>` (drawer open), the notices, and `dialog open` while a dialog is
shown. Dialog titles, menu items, and table cells are not reported; check them in the PNG.

## Exit codes

- 2: the file cannot be read or a line does not parse (the message names the line); nothing runs.
- 3: an `expect` failed (after saving `failed-step-N.png`; the final capture is skipped), or the final capture timed out.
- 1: any other failure (a `wait settled` timeout, a missing CRD).
- 0: all steps ran and the final capture was saved.

## Writes

The screenshot build never writes to a cluster, whatever `K8SBOARD_ALLOW_WRITES` says (spec 0030): a script can open a
write dialog and see its failed dry-run, but cannot commit it.
A build with `--features screenshot,lab-writes` is the one exception (spec 0030 decision 24b): it writes only on a
`kind-*` context under `K8SBOARD_ALLOW_WRITES=1`, so scripts can commit on the kind lab. Never run it against UAT.

`K8SBOARD_SCREENSHOT_HOVER` still applies to the final capture.
