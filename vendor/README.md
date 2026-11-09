Crates patched in place of their crates.io releases (see `[patch.crates-io]` in Cargo.toml).

- `hayro-syntax` 0.8.0: `ArrayIter::next` (src/object/array.rs) ends the array on an
  unreadable item instead of panicking on an `unwrap`, which damaged files reach (found by the
  fuzzers; tests/regressions.rs and tests/fuzz-regressions/). Drop the patch once a release
  fixes it upstream.
