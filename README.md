# Bevy Build Performance Test

This crate drives a suite of build + hotpatch benchmarks for a minimal Bevy
application. Every scenario generates a throw-away Bevy project, builds it,
optionally runs `dx serve --hot-patch`, and tears the workspace back down so the
next scenario starts from a clean slate.

The generated payload (based on `sample/`) prints a unique
`PAYLOAD_SYSTEM_IS_READY__...` marker on startup and a `PAYLOAD_RANDOM_VALUE=...`
line so the harness can confirm that the right code is running before and after
hotpatching.

## Scenario Matrix

For each combination of settings below the harness emits a dedicated project:

| Dimension  | Values                                            |
|------------|----------------------------------------------------|
| Linker     | default, `rust-lld`                                |
| Cache      | default incremental, `CARGO_INCREMENTAL=0`, sccache |
| Dynamic    | default, `bevy/dynamic_linking`, `-Zshare-generics` |
| Hotpatch   | none, `dx serve --hot-patch`                        |

Each scenario records:

1. Clean build (`cargo build` in a fresh temporary directory).
2. Second build (`cargo build` immediately after, to capture incremental gains).
3. Modified build: rewrite the generated payload to touch code, run `cargo build`
  again, and measure the partial recompilation cost.
4. Hotpatch time (only when `Hotpatch = dx`): start `dx serve --hot-patch`, wait
   for the ready marker, rewrite the payload constant, wait for the new
   `PAYLOAD_RANDOM_VALUE=...` line, then terminate `dx`.

## Requirements

- Rust toolchain capable of building Bevy (nightly is configured via
  `rust-toolchain.toml`).
- `dx` CLI available on `PATH` (the harness calls `dx serve --hot-patch`).
- Optional: `sccache`, `rust-lld`, and `dx` hotpatch prerequisites depending on
  which scenarios you intend to run.

## Running the Benchmarks

From the repository root:

```powershell
cargo run
```

The program will enumerate every scenario, stream the `cargo`/`dx` output to
your console, and print a concise timing summary per scenario. Temporary
workspaces live under your system temp directory and are deleted automatically
after each scenario completes.

Each invocation also writes an incremental RON log to `results/run-YYYYMMDD-HHMMSS.ron`
so you can archive or post-process timing data later. The file is updated after
every scenario finishes.

If a required tool (such as `dx` or `sccache`) is missing the corresponding
scenario will fail with a descriptive error so you can install the dependency or
skip those configurations.
