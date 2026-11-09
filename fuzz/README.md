# Fuzzing

Requires a nightly toolchain and `cargo install cargo-fuzz`.

```sh
cd fuzz
cargo +nightly fuzz run open_render -- -max_total_time=600
```

Seed the corpus first. `seeds/pdf/` holds small PDFs that each reach a different part of the
parser and the editing code, plus a few hostile ones (made by `seeds/make_seeds.py`), and
`seeds/data/` holds comment and form data files for `import_data`:

```sh
mkdir -p corpus/open_render && cp seeds/pdf/* ../crates/panna-core/tests/fixtures/*.pdf corpus/open_render/
cargo +nightly fuzz run open_render corpus/open_render -- -max_total_time=600
```

CI does the same on every push. The nightly run also adds the pdf.js test suite's PDFs under
1 MB, which are only ever read as input. Your own PDFs help too (`cp ~/Documents/*.pdf
corpus/open_render/`).

Any crash is written to `artifacts/open_render/`. Copy it into
`crates/panna-core/tests/fuzz-regressions/` before fixing: `tests/fuzz_inputs.rs` runs every
file there, and every seed, on each `cargo test`.
