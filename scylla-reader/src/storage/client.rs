//! A shared Tokio runtime so the TUI's main thread can drive async backend
//! calls from synchronous code.

use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

pub fn block_on<F: Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}
