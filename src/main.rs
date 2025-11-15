use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum Operation {
    First,
    Second,
    Hotpatching,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct ScenarioTimings {
    /// a clean build
    first: Option<Duration>,
    /// second build, with no changes to the system
    second: Option<Duration>,
    /// time it takes for hotpatch
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

fn main() {
    let prepared = prepare_scenarios();
    println!("Prepared {} Bevy benchmark payloads.", prepared.len());
    for entry in &prepared {
        println!(
            "- {} ({}) => marker={}, payload_value={}, files: Cargo.toml {}B, main.rs {}B, config {}B, toolchain {}B",
            entry.slug,
            entry.scenario.describe(),
            entry.ready_marker,
            entry.payload_value,
            entry.code.cargo_toml.len(),
            entry.code.src_main_rs.len(),
            entry.code.cargo_config_toml.len(),
            entry.code.rust_toolchain_toml.len()
        );
    }
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
                if matches!(dynamic, Some(Dynamic::DynamicLinking)) {
                    scenarios.push(Scenario {
                        hotpatching: Some(Hotpatching::Dx),
                        ..base
                    });
                }
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
