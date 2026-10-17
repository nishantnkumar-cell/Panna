# Fuzzing

Requires a nightly toolchain and `cargo install cargo-fuzz`.

```sh
cd fuzz
cargo +nightly fuzz run open_render -- -max_total_time=600
```

Seed the corpus with real-world PDFs for much better coverage:

```sh
mkdir -p corpus/open_render && cp ~/Documents/*.pdf corpus/open_render/
```

Any crash is written to `artifacts/open_render/`. Add a regression test for it in
`crates/panna-core/tests/` before fixing.
