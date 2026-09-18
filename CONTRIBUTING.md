# Contributing to Glasir

Thank you for improving Glasir. Contributions are accepted through pull
requests only; direct changes to `main` are not part of the project workflow.

## Before opening a pull request

1. Create a focused branch from the current `main` branch.
2. Keep production code, comments, and user-facing documentation in English.
3. Add or update tests for every behavior change.
4. Run the required local checks:

```sh
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked
```

5. Do not commit generated graphs, local credentials, editor state, or local
   assistant configuration. The global Git excludes file handles local
   workstation artifacts.

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
