# 0003 · Dev-only screenshot hook

[Back to index](README.md) · Module: `screenshot.rs` · Cargo feature: `screenshot`

## Mechanism (verified in the vendored sources)

- With `test-support` on, GPUI exposes `Window::render_to_image() -> anyhow::Result<image::RgbaImage>` (`gpui-pre 0.3.7`, `src/window.rs`).
- On Windows it is implemented by `gpui-pre-windows` `DirectXRenderer::render_to_image`. This renders the last scene into the existing render target, copies it to a staging texture, and converts BGRA to RGBA. No screen grab and no OS capture API are involved, so window occlusion and DPI scaling of the desktop do not matter.
- GPUI's `image` dependency has the `png` feature, so `image.save(path)` writes PNG with no new dependency.
- Feature wiring: `[features] screenshot = ["gpui-kit/test-support"]` in `crates/app/Cargo.toml`.
  - `screenshot` must **never** be in the `default` feature set, and release builds never enable it.
  - This build is dev-only and slower. Test-support turns on GPUI leak detection and extra platform features (proptest, wayland/x11 code paths) and adds compile time. Expect a separate, longer first build.
- No `unsafe`, no FFI, and no new crate.

## Flags

```text
cargo run -p k8sboard --features screenshot -- \
  --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor \
  --screen pods|nodes|pod-drawer|pod-containers|pod-events|node-drawer|logs-dock|logs-zoomed|<plural>|<plural>-drawer [--theme light|dark] \
  --screenshot .tmp/ui-shots/<name>.png
```

Without the feature, `--screenshot` exits 2 before GPUI starts ([bootstrap.md](bootstrap.md)). `--screen` alone works in every build, which is useful for manual checks.

## Flow (`screenshot.rs`)

1. `main` opens the window as usual (1320×900). If `screenshot` is set, `AppShell` starts `run_screenshot(path, screen)` with `cx.spawn`.
2. Screen setup, applied once the target list is `Ready` or `Failed`:

| Screen | Setup |
|---|---|
| `pods` | screen Pods, no selection |
| `nodes` | screen Nodes, no selection |
| `pod-drawer` | Pods. Select the first pod with ≥ 2 containers, else row 0. Overview tab |
| `pod-containers` | as `pod-drawer`, plus the Containers tab and `is_expanded = true` |
| `pod-events` | as `pod-drawer`, on the Events tab (spec 0006); the capture waits for the debounced object events watch |
| `node-drawer` | Nodes, select row 0 |
| `logs-dock` / `logs-zoomed` | Pods with the log dock open on a pod (spec 0004); zoomed also zooms the dock |
| `<plural>` (spec 0005) | the kind screen, e.g. `deployments`, no selection |
| `<plural>-drawer` (spec 0005) | the kind screen with row 0 selected, e.g. `deployments-drawer`. Wired for all ten kinds of spec 0005 |

3. Wait condition, pure and testable: `fn is_screen_settled(screen, &SettleInput) -> bool`. True when:
   - the session is `Failed`, or
   - the kubeconfig/context failed, or
   - the target list is not `Loading`, and for drawer screens a subject is selected.

   Poll every 100 ms (`cx.background_executor().timer`), with a 30 s timeout.
4. Then wait 300 ms more (animations and the first frame after setup), call `window.refresh()`, and wait one more 100 ms tick.
5. Capture: `window_handle.update(cx, |_, window, _| window.render_to_image())`, then `.save(&path)`. Create the parent directory with `std::fs::create_dir_all`.
6. Record the outcome in a shared `Rc<Cell<ScreenshotOutcome>>` read by `main`, then `cx.quit()`.

| Outcome | Exit code | stderr |
|---|---|---|
| saved after settle | 0 | `screenshot saved: <path> (<w>x<h>)` |
| saved after the 30 s timeout | 3 | `screenshot saved after timeout (screen not settled)` |
| capture or save failed | 1 | the error chain |

- If `application().run` does not return after `cx.quit()` on Windows, `main` falls back to `std::process::exit(code)` inside the quit path. Document which one happened in the code comment.
- Gating per the style guide:
  - only the capture call and the flow wiring are `#[cfg(feature = "screenshot")]`;
  - `is_screen_settled` and the screen setup are plain code, used by both `--screen` and the hook;
  - `ScreenshotOutcome` and its exit-code mapping are `#[cfg(any(feature = "screenshot", test))]`.
- Error states are captured too, which is how the ui-verifier checks them. Example: `--context does-not-exist` gives the context error state.

## How to verify (coder-lite, then the ui-verifier)

1. `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`
2. For each screen in {pods, nodes, pod-drawer, pod-containers, node-drawer} × theme {light, dark}, run the command above into `.tmp/ui-shots/0003-<screen>-<theme>.png` and check:
   - the exit code is 0;
   - the file starts with the PNG signature: `head -c 8 <file> | od -An -tx1` gives `89 50 4e 47 0d 0a 1a 0a`;
   - stderr dimensions equal 1320×900 times the display scale factor.
3. Error state: run once with `--context does-not-exist --screen pods` (exit 0, error screen captured).
4. The ui-verifier opens the PNGs (Read tool) and compares them with the W4/W4b/W5 reference renders (headless Chrome, as defined in its agent file).
