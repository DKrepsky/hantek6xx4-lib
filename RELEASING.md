# Releasing

1. Update `version` in the workspace `Cargo.toml` and the `version` of the `hantek6xx4-lib` dependency in
   `crates/hantek6xx4-cli/Cargo.toml`; move the `[Unreleased]` entries of `CHANGELOG.md` under the new version.
2. Run the checks locally:
   ```sh
   cargo fmt --all --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
   cargo package -p hantek6xx4-lib --locked
   ```
3. If anything changed in the driver, repeat the hardware checks listed in `docs/protocol.md` (verification status)
   on a real scope, at least `hantek6xx4 info`, a triggered `capture`, `capture --volts` against a known DC voltage
   and `roll`.
4. Commit, tag (`git tag -a vX.Y.Z -m "vX.Y.Z"`) and push the tag. CI must be green.
5. Publish the library first, then the CLI (which depends on the published library):
   ```sh
   cargo publish -p hantek6xx4-lib
   cargo publish -p hantek6xx4-cli
   ```
   `cargo publish -p hantek6xx4-cli --dry-run` only succeeds after the matching library version is on crates.io.
6. Create the GitHub release from the tag with the changelog entry.
