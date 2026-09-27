//! Agent Nexus semantic domain.
//!
//! ```text
//! Git / files ─▶ NexusArtifact ─▶ ArtifactParser ─▶ SemanticModel
//!                                                     │
//!             Validator ◀─────────┬───────────────────┤
//!             Translator ◀────────┤                   ▼
//!             semantic diff ◀─────┘            Proposal ─▶ review ─▶ canonical graph
//! ```
//!
//! `SemanticModel` is the intermediate representation between roles:
//! nothing is translated file-to-file. Parsers, translators and validators
//! are the internal extension points (traits); MCP tools stay thin adapters
//! over `SemanticEngine`.
//!
//! Evidence rules: `explicit` = stated in an authored source, `inferred` =
//! derived by a named deterministic rule, `candidate` = weak signal,
//! `unknown` = no evidence. Accepting a proposal puts statements into the
//! canonical graph but never changes their evidence.

pub mod confidence;
pub mod consolidate;
pub mod contract;
pub mod diff;
pub mod engine;
pub mod engineering;
pub mod error;
pub mod git;
pub mod graph_view;
pub mod ingest;
pub mod model;
pub mod parser;
pub mod proposal;
pub mod store;
pub mod text;
pub mod translate;
pub mod validate;
pub mod viz;

pub use engine::SemanticEngine;
pub use error::{SemanticError, SemanticResult};
pub use model::*;

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// Drives a future to completion on the current thread. The server is
/// synchronous; the traits are async so an LLM-backed implementation can
/// await network I/O later without changing their signatures.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}
