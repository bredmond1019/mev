//! Brain consumer: crawl the company-brain repo's Markdown files for OKF validation.
//!
//! This module groups all brain-specific code. Phase 2, Block G implements the crawl
//! entry point; Blocks H and I add OKF frontmatter parsing and the `validate-brain`
//! subcommand respectively.

pub mod crawl;
