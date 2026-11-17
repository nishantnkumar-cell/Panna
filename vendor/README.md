Crates patched in place of their crates.io releases (see `[patch.crates-io]` in Cargo.toml).

- `hayro` 0.9.0: `fill_cached_glyph` (src/glyph_cache.rs) adds a glyph's place and its cached
  offset as `f64` rather than `i32`, which overflowed for text placed near the edge of the
  `i32` range (found by the fuzzers; tests/fuzz-regressions/open-render-crash-5638af88.pdf).
  Release builds keep overflow checks and catch the panic, so only the fuzzers aborted, but
  the page didn't draw. Drop the patch once a release fixes it upstream.
