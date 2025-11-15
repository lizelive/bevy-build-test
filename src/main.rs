use anyhow::{bail, Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Scenario {
    pub linker: Option<Linker>,
    pub cache: Option<Cache>,
    pub dynamic: Option<Dynamic>,
    pub hotpatching: Option<Hotpatching>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Linker {
    RustLld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Cache {
    DisableIncremental,
    Sscache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Dynamic {
    DynamicLinking,
    ShareGenerics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Hotpatching {
    Dx,
}

#[derive(Debug, Clone)]
struct Code {
    pub cargo_config_toml: String,
    pub src_main_rs: String,
    pub cargo_toml: String,
    pub rust_toolchain_toml: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScenarioTimings {
    first: Option<Duration>,
    second: Option<Duration>,
    hotpatch: Option<Duration>,
}

#[derive(Debug, Clone)]
struct PreparedScenario {
    scenario: Scenario,
    slug: String,
    ready_marker: String,
    payload_value: u64,
    code: Code,
}

#[derive(Debug)]
struct ScenarioResult {
    slug: String,
    timings: ScenarioTimings,
}

#[derive(Debug)]
struct Workspace {
    dir: TempDir,
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
enum StreamEvent {
    Line(StreamKind, String),
    Closed(StreamKind),
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:?}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let prepared = prepare_scenarios();
    println!("Benchmarking {} scenario(s)...", prepared.len());

    for scenario in &prepared {
        println!("\n=== Scenario: {} ===", scenario.slug);
        println!("{}", scenario.scenario.describe());
        let result = run_scenario(scenario)
            .with_context(|| format!("benchmark failed for {}", scenario.slug))?;
        report_timings(&result);
    }

    println!("\nAll scenarios completed.");
    Ok(())
}

fn run_scenario(prepared: &PreparedScenario) -> Result<ScenarioResult> {
    let workspace = Workspace::create(prepared)?;
    let first = run_cargo_build(&workspace, "clean")?;
    let second = run_cargo_build(&workspace, "second")?;
    let hotpatch = if prepared.scenario.hotpatching.is_some() {
        Some(run_dx_hotpatch(&workspace, prepared)?)
    } else {
        None
    };

    Ok(ScenarioResult {
        slug: prepared.slug.clone(),
        timings: ScenarioTimings {
            first: Some(first),
            second: Some(second),
            hotpatch,
        },
    })
}

fn run_cargo_build(workspace: &Workspace, label: &str) -> Result<Duration> {
    println!(
        "[bench] Running {label} cargo build in {}",
        workspace.path().display()
    );
    let start = Instant::now();
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(workspace.path())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run cargo build ({label})"))?;

    if !status.success() {
        bail!("cargo build ({label}) failed with status {status}");
    }

    Ok(start.elapsed())
}

fn run_dx_hotpatch(workspace: &Workspace, prepared: &PreparedScenario) -> Result<Duration> {
    println!("[bench] Starting dx serve hotpatch session...");
    let mut child = Command::new("dx")
        .arg("serve")
        .arg("--hot-patch")
        .arg("--features")
        .arg("bevy/hotpatching")
        .current_dir(workspace.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn dx serve")?;

    let stdout = child
        .stdout
        .take()
        .context("dx serve stdout pipe missing")?;
    let stderr = child
        .stderr
        .take()
        .context("dx serve stderr pipe missing")?;

    let (tx, rx) = mpsc::channel();
    spawn_stream_reader(stdout, StreamKind::Stdout, tx.clone());
    spawn_stream_reader(stderr, StreamKind::Stderr, tx.clone());
    drop(tx);

    let ready_deadline = Instant::now() + Duration::from_secs(180);
    let mut ready_seen = false;
    let mut expected_payload_line: Option<String> = None;
    let mut hotpatch_started: Option<Instant> = None;

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(StreamEvent::Line(kind, line)) => {
                forward_stream_line(kind, &line);

                if !ready_seen && line.contains(&prepared.ready_marker) {
                    ready_seen = true;
                    println!("[bench] Ready marker {} observed.", prepared.ready_marker);
                    let (next_value, expected_line) = mutate_payload_constant(workspace, prepared)?;
                    println!(
                        "[bench] Hotpatch triggered, waiting for PAYLOAD_RANDOM_VALUE={next_value}."
                    );
                    expected_payload_line = Some(expected_line);
                    hotpatch_started = Some(Instant::now());
                    continue;
                }

                if let (Some(expected), Some(started)) =
                    (expected_payload_line.as_ref(), hotpatch_started)
                {
                    if line.contains(expected) {
                        println!("[bench] Hotpatch payload observed.");
                        shutdown_process(&mut child)?;
                        return Ok(started.elapsed());
                    }
                }
            }
            Ok(StreamEvent::Closed(kind)) => {
                if let Some(status) = child
                    .try_wait()
                    .context("failed to poll dx serve status")?
                {
                    bail!("dx serve exited early ({kind:?}) with status {status}");
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !ready_seen && Instant::now() > ready_deadline {
                    shutdown_process(&mut child)?;
                    bail!("timeout waiting for ready marker {}", prepared.ready_marker);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                let status = child.wait().context("failed to wait for dx serve")?;
                bail!("dx serve output closed unexpectedly (status {status})");
            }
        }
    }
}

fn mutate_payload_constant(
    workspace: &Workspace,
    prepared: &PreparedScenario,
) -> Result<(u64, String)> {
    let new_value = next_payload_value(prepared.payload_value);
    let new_source = build_payload_main(&prepared.ready_marker, new_value);
    fs::write(workspace.src_main_file(), new_source)
        .context("failed to update payload source for hotpatch")?;
    Ok((new_value, format!("PAYLOAD_RANDOM_VALUE={new_value}")))
}

fn next_payload_value(previous: u64) -> u64 {
    let candidate = previous ^ 0xa076_1d64_78bd_642f;
    if candidate != previous {
        candidate
    } else {
        previous.wrapping_add(0x9e37)
    }
}

fn spawn_stream_reader<R>(reader: R, kind: StreamKind, tx: Sender<StreamEvent>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let buf_reader = BufReader::new(reader);
        for line in buf_reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(StreamEvent::Line(kind, line)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(StreamEvent::Closed(kind));
    });
}

fn forward_stream_line(kind: StreamKind, line: &str) {
    match kind {
        StreamKind::Stdout => println!("[dx] {line}"),
        StreamKind::Stderr => eprintln!("[dx][stderr] {line}"),
    }
}

fn shutdown_process(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        child.kill().ok();
        child.wait().context("failed to wait for dx serve during shutdown")?;
    }
    Ok(())
}

fn report_timings(result: &ScenarioResult) {
    println!(
        "[bench] Results for {} -> clean={}, second={}, hotpatch={}",
        result.slug,
        format_duration(result.timings.first),
        format_duration(result.timings.second),
        format_duration(result.timings.hotpatch)
    );
}

fn format_duration(duration: Option<Duration>) -> String {
    match duration {
        Some(value) => format!("{:.3}s", value.as_secs_f64()),
        None => "n/a".to_string(),
    }
}

impl Workspace {
    fn create(prepared: &PreparedScenario) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix(&format!("bench-{}-", prepared.slug))
            .tempdir()
            .context("failed to create temporary workspace")?;
        write_workspace_files(dir.path(), &prepared.code)?;
        Ok(Self { dir })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn src_main_file(&self) -> PathBuf {
        self.path().join("src").join("main.rs")
    }
}

fn write_workspace_files(root: &Path, code: &Code) -> Result<()> {
    fs::create_dir_all(root.join("src"))
        .context("failed to create src directory in temporary workspace")?;
    fs::create_dir_all(root.join(".cargo"))
        .context("failed to create .cargo directory in temporary workspace")?;

    fs::write(root.join("Cargo.toml"), &code.cargo_toml)
        .context("failed to write Cargo.toml")?;
    fs::write(root.join("src").join("main.rs"), &code.src_main_rs)
        .context("failed to write generated main.rs")?;
    fs::write(root.join(".cargo").join("config.toml"), &code.cargo_config_toml)
        .context("failed to write .cargo/config.toml")?;
    fs::write(root.join("rust-toolchain.toml"), &code.rust_toolchain_toml)
        .context("failed to write rust-toolchain.toml")?;

    Ok(())
}

fn prepare_scenarios() -> Vec<PreparedScenario> {
    enumerate_scenarios()
        .into_iter()
        .map(PreparedScenario::new)
        .collect()
}

fn enumerate_scenarios() -> Vec<Scenario> {
    let linkers = [None, Some(Linker::RustLld)];
    let caches = [None, Some(Cache::DisableIncremental), Some(Cache::Sscache)];
    let dynamics = [
        None,
        Some(Dynamic::DynamicLinking),
        Some(Dynamic::ShareGenerics),
    ];

    let mut scenarios = Vec::new();
    for linker in linkers {
        for cache in caches {
            for dynamic in dynamics {
                let base = Scenario {
                    linker,
                    cache,
                    dynamic,
                    hotpatching: None,
                };
                scenarios.push(base);
                scenarios.push(Scenario {
                    hotpatching: Some(Hotpatching::Dx),
                    ..base
                });
            }
        }
    }

    scenarios
}

impl PreparedScenario {
    fn new(scenario: Scenario) -> Self {
        let slug = scenario.slug();
        let seed = scenario.payload_seed();
        let ready_marker = ready_marker(&slug, seed);
        let payload_value = payload_value(seed);
        let code = Code::for_scenario(&scenario, &slug, &ready_marker, payload_value);

        Self {
            scenario,
            slug,
            ready_marker,
            payload_value,
            code,
        }
    }
}

impl Scenario {
    fn slug(&self) -> String {
        let mut parts = Vec::with_capacity(4);
        parts.push(match self.linker {
            Some(Linker::RustLld) => "rust-lld",
            None => "default-linker",
        });
        parts.push(match self.cache {
            Some(Cache::DisableIncremental) => "no-incremental",
            Some(Cache::Sscache) => "sscache",
            None => "incremental",
        });
        parts.push(match self.dynamic {
            Some(Dynamic::DynamicLinking) => "dynamic-linking",
            Some(Dynamic::ShareGenerics) => "share-generics",
            None => "default-dynamic",
        });
        parts.push(match self.hotpatching {
            Some(Hotpatching::Dx) => "dx-hotpatch",
            None => "no-hotpatch",
        });

        parts.join("-")
    }

    fn payload_seed(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    fn describe(&self) -> String {
        format!(
            "linker={}, cache={}, dynamic={}, hotpatch={}",
            self.linker_label(),
            self.cache_label(),
            self.dynamic_label(),
            self.hotpatch_label()
        )
    }

    fn linker_label(&self) -> &'static str {
        match self.linker {
            Some(Linker::RustLld) => "rust-lld",
            None => "default",
        }
    }

    fn cache_label(&self) -> &'static str {
        match self.cache {
            Some(Cache::DisableIncremental) => "no-incremental",
            Some(Cache::Sscache) => "sscache",
            None => "incremental",
        }
    }

    fn dynamic_label(&self) -> &'static str {
        match self.dynamic {
            Some(Dynamic::DynamicLinking) => "dynamic-linking",
            Some(Dynamic::ShareGenerics) => "share-generics",
            None => "default",
        }
    }

    fn hotpatch_label(&self) -> &'static str {
        match self.hotpatching {
            Some(Hotpatching::Dx) => "dx",
            None => "none",
        }
    }
}

impl Code {
    fn for_scenario(
        scenario: &Scenario,
        slug: &str,
        ready_marker: &str,
        payload_value: u64,
    ) -> Self {
        Self {
            cargo_config_toml: build_cargo_config(scenario, slug),
            src_main_rs: build_payload_main(ready_marker, payload_value),
            cargo_toml: build_cargo_toml(scenario, slug),
            rust_toolchain_toml: default_toolchain(),
        }
    }
}

fn ready_marker(slug: &str, seed: u64) -> String {
    format!("PAYLOAD_SYSTEM_IS_READY__{slug}__{seed:016x}")
}

fn payload_value(seed: u64) -> u64 {
    seed.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15
}

fn build_payload_main(ready_marker: &str, payload_value: u64) -> String {
    format!(
        r#"use bevy::prelude::*;

const READY_MARKER: &str = "{ready_marker}";
const PAYLOAD_RANDOM_VALUE: u64 = {payload_value};

fn main() {{
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, announce_ready)
        .add_systems(Update, heartbeat)
        .run();
}}

fn announce_ready() {{
    println!("{{}}", READY_MARKER);
    println!("PAYLOAD_RANDOM_VALUE={{}}", PAYLOAD_RANDOM_VALUE);
}}

fn heartbeat(mut ticks: Local<u32>) {{
    *ticks += 1;
    if *ticks % 600 == 0 {{
        println!("PAYLOAD_HEARTBEAT::{{}}::{{}}", READY_MARKER, *ticks);
    }}
}}
"#
    )
}

fn build_cargo_config(scenario: &Scenario, slug: &str) -> String {
    let mut output = String::new();
    output.push_str("[build]\n");
    output.push_str(&format!("target-dir = \"target/{slug}\"\n"));

    let mut env_lines: Vec<(&str, &str)> = Vec::new();
    if let Some(cache) = scenario.cache {
        match cache {
            Cache::DisableIncremental => env_lines.push(("CARGO_INCREMENTAL", "0")),
            Cache::Sscache => env_lines.push(("RUSTC_WRAPPER", "sccache")),
        }
    }

    if matches!(scenario.dynamic, Some(Dynamic::ShareGenerics)) {
        env_lines.push(("RUSTFLAGS", "-Zshare-generics=y"));
    }

    if !env_lines.is_empty() {
        output.push_str("\n[env]\n");
        for (key, value) in env_lines {
            output.push_str(&format!("{key} = \"{value}\"\n"));
        }
    }

    if matches!(scenario.linker, Some(Linker::RustLld)) {
        output.push_str("\n[target.'cfg(all())']\n");
        output.push_str("linker = \"rust-lld.exe\"\n");
    }

    output
}

fn build_cargo_toml(scenario: &Scenario, slug: &str) -> String {
    let mut bevy_features = Vec::new();
    if matches!(scenario.dynamic, Some(Dynamic::DynamicLinking)) {
        bevy_features.push("dynamic_linking");
    }
    if matches!(scenario.hotpatching, Some(Hotpatching::Dx)) {
        bevy_features.push("hotpatching");
    }

    let features_clause = if bevy_features.is_empty() {
        String::new()
    } else {
        let feature_list = bevy_features
            .into_iter()
            .map(|feat| format!("\"{feat}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(", features = [{feature_list}]")
    };

    format!(
        r#"[package]
name = "bench-payload-{slug}"
version = "0.1.0"
edition = "2024"

[dependencies]
bevy = {{ version = "0.17.2"{features_clause} }}

[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
"#
    )
}

fn default_toolchain() -> String {
    const TOOLCHAIN: &str = r#"[toolchain]
channel = "nightly"
components = ["llvm-tools-preview"]
profile = "default"
"#;

    TOOLCHAIN.to_string()
}
