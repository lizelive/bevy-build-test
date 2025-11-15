use std::time::Duration;

#[derive(Debug, Clone, Copy)]
enum Operation {
    First,
    Second,
    Hotpatching,
}

#[derive(Debug, Clone, Copy)]
struct Scenario {
    pub linker: Option<Linker>,
    pub cache: Option<Cache>,
    pub dynamic: Option<Dynamic>,
    pub hotpatching: Option<Hotpatching>,
}
#[derive(Debug, Clone, Copy)]
enum Linker {
    RustLld,
}
#[derive(Debug, Clone, Copy)]
enum Cache {
    Sscache,
    None,
}
#[derive(Debug, Clone, Copy)]
enum Dynamic {
    DynamicLinking,
    ShareGenerics,
}
#[derive(Debug, Clone, Copy)]
enum Hotpatching {
    Dx,
}

#[derive(Debug, Clone)]
struct Code {
    pub cargo_config_toml : String,
    pub src_main_rs : String,
    pub cargo_toml : String,
    pub rust_toolchain_toml : String,
}


#[derive(Debug, Clone, Copy)]
struct ScenarioTimings {
    /// a clean build
    first: Option<Duration>,
    /// second build, with no changes to the system
    second: Option<Duration>,
    /// time it takes for hotpatch
    hotpatch: Option<Duration>,
}

fn main() {}
