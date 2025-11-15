# Bevy Build Performance Test

compares the performance impact of:
- bevy/dynamic_linking
- "-Zshare-generics=y",
- CARGO_INCREMENTAL=0
- linker = "rust-lld.exe"
- sscache

under follwoing senarios
- clean build
- second build
- mutated build
- dx serve --hot-patch --features "bevy/hotpatching"