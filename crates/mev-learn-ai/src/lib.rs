//! `mev-learn-ai` — the operator's own business content tooling (learn-agentic-ai.com
//! frontmatter/JSON validation, link checking, code-block linting, and the funnel host-list
//! checker), extracted out of the public `mev` crate so the public binary and its source carry
//! no reference to the operator's business.
//!
//! `publish = false`: this crate is never published to crates.io and is only ever pulled in via
//! `mev`'s non-default `learn-ai` cargo feature.
//!
//! This crate is currently empty — the learn_ai modules move here in a later task of
//! `MV.ticket.extract-learn-ai-into-a-private-optional-crate`. This task only stands up the
//! workspace boundary so it lands compiling and reviewable on its own.
