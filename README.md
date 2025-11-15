# Bevy Build Performance Test
benchmaks the time it takes to build bevy program
use code included in /sample

## Varibles

Linker : (default_linker, rust_lld)
Cache: (incremental, sscache)
Dynamic : (default_dynamic, dynamic_linking, share_generics)

compares the performance impact of:
- bevy/dynamic_linking
- "-Zshare-generics=y",
- CARGO_INCREMENTAL=0
- linker = "rust-lld.exe"
- 

under follwoing senarios
- clean build
- second build
- mutated build
- dx serve --hot-patch --features "bevy/hotpatching"