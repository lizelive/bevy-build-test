# Bevy Build Performance Test

compares the performance impact of 
- nightly vs stable
- bevy/dynamic_linking
- "-Zshare-generics=y",
- CARGO_INCREMENTAL=0
- linker = "rust-lld.exe"

under follwoing senarios
- clean build
- second build
- mutated build
- dx serve --hot-patch --features "bevy/hotpatching"