# Porting CyberChef to utilify-chef

## Status

The "Data format" category is ported (80 of 80 operations; 362 of 363
CyberChef test vectors pass, the other is listed in
`tests/vectors/known_differences.txt`). No other category is being ported.

The hand-written codecs have been replaced with libraries (see
`DEPENDENCIES.md` for which, and what stays ours and why). The port resumes,
one category at a time.

## Policy: libraries first

This applies to every category, including the ones not yet ported.

- **Use the standard library, or a battle-tested crate, for any established
  algorithm or format**: encodings, compression, hashing, encryption, key
  formats, serialization formats (JSON, YAML, CBOR, MessagePack, Avro…),
  character sets, images, dates. Never implement cryptography ourselves.
- **Our code is the adapter.** An operation maps CyberChef's arguments to the
  library, and the library's results and errors to the op's output. Write
  the codec by hand only when no healthy library covers what CyberChef does,
  and say why in the module's doc comment.
- **Check a crate's health before adding it**:
  - released or committed to recently (within the last year), with more than
    one active maintainer, or a single maintainer who is clearly responsive;
  - issues triaged and answered promptly, bugs fixed; open pull requests
    reviewed rather than left to pile up;
  - real-world use: downloads and reverse dependencies on crates.io, use by
    known projects;
  - its own test suite, ideally with fuzzing or conformance tests for the
    format;
  - not deprecated or archived (e.g. `serde_yaml` is archived), and a
    compatible license (MIT, Apache-2.0, BSD, ISC, Zlib).

  Record the assessment (date, findings, decision) in `DEPENDENCIES.md`
  when adding a dependency.
- **Prefer pure Rust**, and few transitive dependencies for what we use.

## The spec is the source of truth

Each operation implements its format's specification (RFC, published format
spec, standards body), not CyberChef's behaviour. CyberChef and the
JavaScript libraries it uses deviate from the specs in places: they misread
valid input, write invalid output, wrap or truncate values, or decode garbage
that the spec says is an error. **We don't copy those deviations.**

What stays CyberChef's:

- Operation names, arguments (names, order, defaults, options) and the recipe
  formats, so CyberChef recipes load unchanged.
- Output presentation where no spec defines one: the text layout of a dump,
  the JSON shape of a parse tree. The content is the spec-correct reading.
- ECMA-262 semantics where that is the spec in question: `JSON.parse` /
  `JSON.stringify`, JavaScript number printing, `parseInt` / `parseFloat`,
  UTF-16 text arguments.

Two checks keep the port honest:

- `tests/cyberchef_vectors.rs` runs CyberChef's own operation tests
  (`tests/vectors/cyberchef.json`). Every vector whose operations are all
  ported must pass, or be listed in `known_differences.txt` with a `spec:`
  reason citing where CyberChef's expected output deviates from the spec.
- `scripts/cyberchef/oracle.mjs` bakes recipes with real CyberChef, for
  comparing cases the vectors don't cover; a difference is fine when we are
  the one following the spec.

Each module's doc comment names the spec it follows and anything where it
can't (with the reason).

## Adding an operation

1. Read the operation in CyberChef (`src/core/operations/<Name>.mjs`) and
   the JavaScript library it calls.
2. Pick the library (policy above), then write the operation in
   `src/ops/<category>/`, following `src/ops/data_format/hex.rs`.
3. Unit-test cases the vectors miss, with expected values from the oracle.
4. In `workbench/`, run `cargo test -p utilify-chef`, `cargo clippy --all-targets -- -D
   warnings` and `cargo fmt --check`.
