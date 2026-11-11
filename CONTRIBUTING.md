# Contributing

Thanks for your interest in improving SmartReName. Bug reports, fixes, and
small focused enhancements are welcome.

## Before opening an issue

- Include your operating system and, when relevant, the Rust version.
- For a bug, describe the steps to reproduce it and what you expected to happen.
- Do not include private file names or other sensitive information in screenshots
  or logs.

## Development checks

Install the stable Rust toolchain, then run these commands from the repository
root before submitting a pull request:

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The rename engine is in `crates/srename-core`; add or update tests there for
engine behavior. Keep changes focused, and include a clear summary of behavior
changes in the pull request.
