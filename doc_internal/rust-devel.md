# Rust development

fish is one Cargo workspace. `Cargo.toml` at the repository root declares the `fish` package and the packages under `crates/`.

The `fish` package contains the shell library in `src/lib.rs` and the modules under `src/`, and three binaries: `src/bin/fish.rs`, `src/bin/fish_indent.rs` and `src/bin/fish_key_reader.rs`.

The tree contains no C++ sources. The only C source file is `tests/fish_test_helper.c`, which `tests/test_driver.py` compiles into the helper binary that the behaviour tests run. `build.rs` compiles C feature probes through `rsconf` to declare the platform cfgs.

`build.rs` sets the environment variables that the library embeds with `env!`:

- `FISH_BUILD_VERSION`, from `build_tools/git_version_gen.sh`;
- `BUILD_TARGET_TRIPLE`, `BUILD_HOST_TRIPLE` and `BUILD_PROFILE`, the target, host and profile of the compilation;
- `FISH_RESOLVED_BUILD_DIR`, the build directory that `fish` compares against its executable path at runtime;
- `PREFIX` and `SYSCONFDIR`, and `DATADIR`, `BINDIR` and `DOCDIR` when they are set.

Development commands and the build types are in `CONTRIBUTING.rst`, sections "Building" and "Testing".
