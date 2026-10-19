//! The page renderer starts from the built app, locks itself down and still draws its test
//! page. On Windows it must be doing so inside an AppContainer.

use std::io::Read;

/// The tag of the renderer's "ready" message (`panna_core::isolate`).
const READY: u8 = 6;

#[test]
fn renderer_locks_down_and_draws() {
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_panna"));
    let mut child = panna_sandbox::spawn(program, panna_core::isolate::ARG).expect("renderer starts");
    if cfg!(windows) {
        assert!(
            child.app_container,
            "the renderer should start in an AppContainer"
        );
    }
    let mut head = [0u8; 9];
    child
        .stdout
        .read_exact(&mut head)
        .expect("the renderer says it's ready rather than stopping");
    assert_eq!(head[0], READY);
}
