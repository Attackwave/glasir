# Contributing to Glasir

Thank you for improving Glasir. Contributions are accepted through pull
requests only; direct changes to `main` are not part of the project workflow.

## Before opening a pull request

1. Create a focused branch from the current `main` branch.
2. Keep production code, comments, and user-facing documentation in English.
3. Add or update tests for every behavior change.
4. Run the checks CI runs:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo test --locked --manifest-path parsers/Cargo.toml --target-dir target/parser-tests
cargo build --release --locked
./target/release/glasir selfcheck
rm -f .glasir-graph && ./target/release/glasir benchmark . --check
./target/release/glasir langcheck bench/langs --check
./target/release/glasir langcheck bench/langs --anomalies --check
./target/release/glasir guard .
```

   Measure the benchmark in a clean clone: an untracked file in a working copy
   changes what is indexed and moves every recall floor. CI also runs the
   scaling gate (`python3 bench/scale.py target/release/glasir --check`).

5. A change to what a rebuild produces — edges, names, documentation, the
   partition — bumps `FORMAT_VERSION` in `src/snapshot.rs`; a scanner change
   bumps `IMPLEMENTATION_VERSION` in `parsers/src/rules.rs`. Otherwise a stored graph from
   before the change is reused and silently keeps the old behaviour.
6. Do not commit generated graphs (`.glasir-*`), credentials, editor state or
   assistant configuration.

## Pull request expectations

Describe the problem, the intended behavior, and the evidence that verifies
the change. Keep unrelated formatting and refactors out of the same pull
request. Changes to parsing or graph resolution must preserve deterministic
output and include a regression case when practical.

## Security-sensitive changes

Do not disclose exploitable weaknesses in a public issue or pull request.
Follow [SECURITY.md](SECURITY.md) for private reporting and coordinated fixes.

## License

By contributing, you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE).
