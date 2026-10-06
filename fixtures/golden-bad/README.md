# Golden BAD

Profile `rust-strict-v1` MUST report exactly these 5 violations here:

1. `cargo-workspace-inherit-edition` — `crates/group/app/Cargo.toml`
   pins `edition = "2024"` instead of inheriting.
2. `tests-dir-present` — `crates/group/worker/` has no `tests/` dir
   (its manifest is clean, so only this rule fires).
3. `layout-no-flat-crate-manifest` — flat `crates/solo/Cargo.toml`
   (S4 requires `crates/<group>/<package>`).
4. `ci-build-runs-on` — `build` job on `ubuntu-24.04`, not
   `ubuntu-latest` (toolchain pin still matches, so parity passes).
5. `size-file-400` — `src/big.rs` (500 lines) is listed in
   `.gitignore`; it fails ONLY because the profile sets
   `respect_gitignore: false`.
