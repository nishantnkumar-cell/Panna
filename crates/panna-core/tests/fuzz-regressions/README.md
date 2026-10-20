Inputs that once crashed, hung or exhausted memory in a fuzzer (`fuzz/artifacts/<target>/`).
Copy each one here, under a name saying what it broke, before fixing it: `tests/fuzz_inputs.rs`
runs every file in this folder and in `fuzz/seeds/` through the same paths the fuzzers do.
