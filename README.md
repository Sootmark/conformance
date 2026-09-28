# conformance

Checks that a parser adapter keeps the Sootmark adapter contract.

`assert_conforms(adapter, name, bytes)` parses a fixture twice, then damaged variants of it (truncated, bytes inverted), and fails with the list of broken promises: panics, non-determinism, duplicate ids, wrong parser info, undeclared namespaces, wrong evidence, empty summaries, unnamed timestamps. Its own tests prove every check fires.

## Quality

`#![forbid(unsafe_code)]`, `clippy::pedantic` clean, `cargo-deny` (permissive licences, no network crates).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
