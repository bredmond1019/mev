//! Regression coverage for `extract_links`' inline-code tracking: an unterminated
//! inline backtick must close at end of line, the way real markdown parsers do,
//! rather than carrying `in_inline_code` state to end of file.
//!
//! Background (2026-09-02): `core/_planning/mev/status.md` carried three backticked
//! doubled-bracket constructs. Because the extractor's `in_inline_code` toggled on
//! every backtick with no per-line reset, one stray unbalanced backtick earlier in
//! the file inverted the in-code/not-in-code state for everything after it — two of
//! the three constructs were skipped as "in code" while the third was extracted and
//! red-gated the whole corpus with `E_LINK_DANGLING_WIKILINK`. Fixed by
//! `dbe858170` ("fix(mev): unbracket [[repos]] in status.md — wikilink scanner
//! red-gated the corpus").

use mev::extract_links;

/// THE REGRESSION: a document with one unbalanced inline backtick on an early line,
/// then a `[[slug]]` on a later line, must extract that wikilink identically to the
/// same document without the stray backtick. Asserted as equality between the two
/// extractions, never against a frozen count.
#[test]
fn unterminated_backtick() {
    let with_stray_backtick = "This line has a stray ` backtick.\n\nSee [[my-doc]] for more.\n";
    let without_stray_backtick = "This line has no stray backtick.\n\nSee [[my-doc]] for more.\n";

    let links_with = extract_links(with_stray_backtick);
    let links_without = extract_links(without_stray_backtick);

    assert_eq!(
        links_with, links_without,
        "an unbalanced inline backtick on an earlier line must not change whether \
         a wikilink on a later line is extracted"
    );
    assert_eq!(
        links_with.len(),
        1,
        "the wikilink must actually be extracted, not silently protected"
    );
    assert_eq!(links_with[0].target, "my-doc");
}

/// THE FENCE CONTROL: a fenced code block still protects its whole contents across
/// multiple fences in one document — the newline reset must never touch
/// `in_fenced_code`. This is the assertion that fails if someone "simplifies" the
/// triple-backtick branch to treat ``` as three inline toggles.
#[test]
fn fenced_code_still_protects_across_multiple_fences_and_lines() {
    let doc = "\
Intro text with [[real-link]].

```
This fence contains [[fenced-one]]
and spans several lines
before it closes.
```

Between fences: [[also-real]].

```rust
let x = [[fenced-two]];
still inside the second fence.
```

Trailing [[trailing-real]].
";

    let links = extract_links(doc);
    let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();

    assert_eq!(
        targets,
        vec!["real-link", "also-real", "trailing-real"],
        "only the wikilinks outside any fence should be extracted; fenced content \
         must stay protected across multiple lines and multiple fences"
    );
}

/// THE POSITIVE CONTROL: correctly-paired inline backticks on a single line still
/// protect what is between them. Without this, a change that simply deleted
/// inline-code tracking entirely would pass the regression and fence tests above.
#[test]
fn paired_inline_backticks_still_protect_their_contents() {
    let doc = "See `[[not-a-real-link]]` for the literal syntax, but [[real-link]] works.\n";

    let links = extract_links(doc);
    let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();

    assert_eq!(
        targets,
        vec!["real-link"],
        "a wikilink between paired inline backticks must stay protected"
    );
}

/// THE REAL-WORLD REGRESSION, embedded as a literal fixture: the exact shape that
/// red-gated the corpus on 2026-09-02. Three backticked doubled-bracket constructs
/// — `[[conformance_writers]]`, `[[wiki]]`, `[[repos]]` — with an odd number of
/// inline backticks appearing before the third. Under the old parity-only state
/// machine, the first two were treated as in-code (skipped) while the third was
/// extracted as a real wikilink, because an earlier unbalanced backtick elsewhere
/// in the file had flipped `in_inline_code` to `true` by the time the third
/// construct was reached. After the fix, all three must be treated identically
/// (all protected, since each is written as `` `[[slug]]` `` — paired backticks
/// around the construct itself).
///
/// Embedded as an inline string literal per the task's explicit instruction — never
/// read from `git show`, from `planning/`, or from any path under the HQ vault:
/// `planning/` is a symlink into a private vault excluded from this repo's git, so
/// a test reading it would pass on this machine and fail on every CI runner.
#[test]
fn real_world_status_md_shape_treats_all_three_constructs_identically() {
    let doc = "\
Some text mentions `[[conformance_writers]]` as a literal example.

Here is a stray backtick in prose: it's a `contraction, not code.

More prose that discusses `[[wiki]]` syntax as a literal example too.

Finally the corpus references `[[repos]]` as a literal example as well.
";

    let links = extract_links(doc);

    assert!(
        links.is_empty(),
        "all three backticked doubled-bracket constructs must be treated \
         identically (protected as inline code), matching the pre-fix pairing \
         actually written in the source — got: {links:?}"
    );
}
