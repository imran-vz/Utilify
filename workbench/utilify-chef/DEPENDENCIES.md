# Dependency assessments

Each library considered for utilify-chef, assessed against `PORTING.md`'s
health bar and checked for compatibility. The assessments of 2026-10-20
compared libraries against real CyberChef 11.4.0 (oracle cases), under the
rule then in force that output on valid input match CyberChef. Since
2026-10-22 the format spec is the source of truth instead (see
`PORTING.md`), so CyberChef mismatches noted below are only a problem where
CyberChef was the one following the spec.

## Adopt

| Library | For | Health | Evidence |
|---|---|---|---|
| `ryu-js` 1.0.3 | `format_number` (JS number printing) | Boa JS engine org; 72 reverse deps (Boa, SWC, rspack); yearly releases | 0 mismatches vs Node on 4M doubles; ~100× faster. Pin ≥1.0.3 (1.0.2 had UB in `format_to_fixed`) |
| `num-bigint` 0.5.1 | every big-integer core (Base58/62, From/To Base, BCD, Text-Integer, CBOR bignums, ASN.1 OID arcs over 1,024 bits) | rust-num; 2,425 reverse deps; bus factor ~1, uneven issue response | 1,686 oracle cases identical; 13–3000× faster on 50 KB |
| `data-encoding` 2.11 | To/From Base32, To/From Base64 | 850 reverse deps; PRs merged in days; fuzz + Miri; one maintainer | 0 mismatches on 15,588 valid-input cases; −45 lines; decodes RFC-valid unpadded Base32 CyberChef garbles. The adapters add RFC 4648 §3.3's "padding ends the data" check (the library decodes concatenated padded blocks) |
| `base64` 0.22 | internal standard-alphabet helpers (recipe URL `input=`, CBOR tags 33/34: strict RFC 4648 base64url/base64 after `utilify-cbor`'s §3.4.5.3 check) | 19,205 reverse deps; already in the lock | 6,412 oracle + 300k fuzz cases identical; fixes an `atob` error-message bug |
| `percent-encoding` 2.3 | URL Encode, recipe URL decoding, UTF-8 step of URL Decode | servo/rust-url; 2,608 reverse deps; no dependencies; already in the lock | 978 oracle cases identical |
| `url` 2.5 | CBOR tag 32 (WHATWG serialization of absolute URLs; `utilify-cbor` checks RFC 3986 URI-reference) | servo/rust-url; 16,681 reverse deps; WPT tests + fuzzing; slow triage | our parser was wrong on 1,371 of 1,690 URLs; `url` on 4 (open upstream bugs) |
| `idna` 1.1 | To/From Punycode (RFC 3492 punycode module; UTS #46 ToASCII/ToUnicode for the domain-name mode) | servo/rust-url; already in the lock via `url` | RFC 3492 §7.1 samples; error kinds are not distinguished, overflow at 2^32 - 1 |
| `bech32` 0.12 | Bech32 checksum and bit conversion (`bech32::primitives`) | rust-bitcoin; 414 reverse deps; 0 open PRs; BIP vectors, fuzzing, mutation testing | 58/58 vectors; 1,923 oracle cases identical. Don't use top-level `encode`/`decode` or `segwit` |
| `saphyr` 0.2 + `saphyr-parser` 0.2 | YAML to JSON (parser and node loader, `early_parse` off; scalars resolved by the adapter under the YAML 1.2.2 core schema, §10.3.2) | 11 authors in 12 months, one leading; pre-1.0 churn; fuzzed; yaml-test-suite | `saphyr-parser` is a direct dependency only to drive the loader with `early_parse` off (already in the lock). saphyr's own resolution misread `++7`/`0x-1`, wide `0x` integers and explicit core tags; ours follows the spec. Adapter fixes and the parse-level bugs left are listed below. The dumper uses `utilify-yaml-emit` |
| `stacker` 0.1.25 | deep recursion in nested formats (replaces `with_big_stack`) | rust-lang org; used by rustc | fixes a crash: four ops overflowed gpui's 512 KiB worker stacks on 5,000-deep JSON |
| `tempfile` 3 | atomic saves (shared helper in utilify-core) | 37,430 reverse deps; issues answered within a day; no new crates on macOS | fixes a data-loss race: 12–19% of overlapping saves left a corrupt file |
| `jiff` 0.2 | utilify-core local time of day (replaces `localtime_r`) | 1,481 reverse deps; already in the lock | identical across 8 time-zone setups. Not for chef dates: JS years ±275,760 exceed jiff's ±9,999 |
| `regress` 0.12 | CBOR tag 21066 (ECMAScript RegExp: `new RegExp` syntax check; tag 35's syntax is out of band and tag 21065 is checked against the RFC 9485 ABNF in `cbor.rs`) | assessed 2026-10-22: releases every 1–5 months (0.12.0 2026-08-23), last commit 2026-10-20; two owners, one doing most commits plus ~10 contributors a year; recent issues fixed in 2–3 days, PRs merged in about a day; 391 reverse deps (Boa, oxc_resolver, rspack, swc_config, typify), 48M downloads; own syntax-error, property-escape and UnicodeSets suites, no test262 run or fuzzing; MIT/Apache-2.0; only `memchr` | 0 valid patterns rejected and 1 mismatch class (quantified `\b`/`\B` accepted) in 400k random patterns vs V8 (Node 24); 16,000 CBOR cases: 3,762 valid all accepted, 12,194 of 12,238 invalid now errors. Its limits (255 nesting levels, 65,535 quantifiers) and recursion per `\|` are worked around in `cbor.rs` |

## Don't adopt

| Library | Why |
|---|---|
| `base45` 3.2 | fixed alphabet only, opaque errors; slow maintainer responses |
| `bs58` 0.5 | no commits or releases for 31 months; PRs unreviewed since 2021; ASCII alphabets only |
| `base-x` 0.3 | one maintainer, 4-year release gap; panics on bad alphabets; counts code points, not UTF-16 units |
| `quoted_printable` 0.5 | not needed: `utilify-quoted-printable` covers both directions per RFC 2045 |
| `apache-avro` 0.22 | loses map key order; 38/324 oracle mismatches; +51 crates |
| `rmp` 0.8 | one release in 2 years; no replies to any 2026 issue |
| `ciborium-ll` 0.2 | no release in 2.7 years; unanswered issues ("Is this project dead?") |
| `minicbor` 2.3 | no public header API; BlueOak license; saves ~72 lines for +5 crates |
| `half` 2.7 | technically identical, but no issue replies since 2026-03 and no merges since 2026-02 |
| `csv` 1.4 | single-byte delimiters; mid-field quotes and empty lines parse differently (209/972) |
| `encoding_rs`, `oem_cp`, `yore`, `codepage` | none covers CyberChef's page list (EBCDIC, ISCII, Mac, Taiwanese pages); `utilify-codepages` does |

Formats without a suitable library were written from their specs as the
`utilify-*` crates in the separate utilify-codecs repository (a sibling
checkout, used by path). They are generic, not CyberChef replicas, so each op
keeps an adapter for CyberChef's behaviour; the module doc comments list the
accepted differences.

## utilify-codecs crates in use

| Crate | For | Still hand-written, and why |
|---|---|---|
| `utilify-js-json` | `JSON.parse`/`JSON.stringify` (`structured/js.rs`) | the `Js` type: the library's value can't hold `undefined` or BigInt |
| `utilify-base45`, `-base58`, `-base62`, `-base85`, `-base92` | the Base45–92 ops (Base85 through `UnicodeAlphabet`, one `char` per UTF-16 unit) | Base85's PLRM delimiters (`<~`, `~>`) and "Remove non-alphabet chars": `UnicodeOptions` has no delimiters |
| `utilify-codepages` | Encode/Decode text, brute force, MIME Decoding charsets (via `utilify-rfc2047`) | pages 620 (Mazovia) and 29001 (Europa 3): no authoritative table, so the crate lacks them and ours stay |
| `utilify-rfc2047` | MIME Decoding (strict, context per field name) | splitting the input into header fields |
| `utilify-quoted-printable` | To/From Quoted Printable (RFC 2045 encoder, lenient decoder) | nothing |
| `utilify-html-entities` | To/From HTML Entity | nothing (names are `name_of`'s) |
| `utilify-cbor` | CBOR Encode/Decode (strict decoding: validity and tag content per RFC 8949 §5.3, deterministic encoding §4.2.1) | the interpretation of tags into JSON (dates as RFC 3339 text, exact bignums, RFC 8943 dates, RFC 8746 typed arrays, the RFC 9485 I-Regexp grammar, tags 258/262/273/280 per their registry specs): the crate validates content but leaves its meaning to the caller |
| `utilify-msgpack` | To/From MessagePack (token reader/writer, strict decoding, timestamps; its `Timestamp` also formats CBOR tags 1 and 100) | nothing beyond presentation: JSON shapes for bin (Node `Buffer`), extensions (`[type, Buffer]`) and maps with non-str or repeated keys (`[key, value]` pairs) |
| `utilify-avro` | Avro to JSON (strict container decoding per Avro 1.12) | avsc's plain-JSON value shapes (unions wrapped only when ambiguous, Node `Buffer`s); longs beyond ±2^53 as exact decimal strings |
| `utilify-amf` | AMF Encode (`encode_from_json`: its JSON mapping, AMF 3 string and traits references by value, no object references), AMF Decode (`wire::Reader`, events assembled into the element-tree JSON) | nothing |
| `utilify-rison` | Rison Encode/Decode (ids, numbers, key order and URI encoding all the crate's, per the spec) | nothing |
| `utilify-cobs`, `utilify-tlv` | To/From COBS, Parse TLV | TLV: keys over 8 bytes and little-endian length fields over 32 bytes |
| `utilify-csv` | CSV to JSON (reader), JSON to CSV (writer) | JSON to CSV's header and `flat`-style flattening (presentation RFC 4180 doesn't define) |
| `utilify-hexdump` | To Hexdump (all widths to 65,536, via `LayoutLimits`); From Hexdump (`parse_auto_with` for `hexdump -C`, `xxd`, `od -t x1`, Wireshark's hex copies, `text2pcap` input and 010 Editor hex text, plus To Hexdump's own layout); From Hex, Swap endianness and Hex to PEM (strict `hex::decode`) | nothing |
| `utilify-asn1` | Parse ASN.1 hex string (`RawWalker`, `RawElement::encapsulated`, `Oid`, `Integer`, `StringType`) | the dump's text layout and jsrsasign's OID names |
| `utilify-yaml-emit` | JSON to YAML (all writing and every style choice: `Schema::Yaml11Safe`, `StyleMode::Prefer`, indent 2, 80 columns) | nothing: the js-yaml style policy and its stand-in and header workarounds are gone. 5,000 generated values round-trip through saphyr to the same JSON (test). Adds no other crates to the lock |

## Bugs the assessments found in our code

Fixed with the migration: 1 (deep input; `stacker`), 2 (atomic saves;
`tempfile`), 3 (CBOR URLs; `url`), 5 (recipe URL params), 6 (From Base
prefix), 7 (negative years), 9 (CBOR `atob`), 10 (empty-key flattening), and 8
except RegExp tags. Fixed on 2026-10-22:

4. `util::expand_alph_range` expands per UTF-16 unit, as CyberChef. The
   Base32/45/58/62/64/85 ops give the same output as before on 8,243 random
   BMP-alphabet cases; the expansion matches a port of `expandAlphRange` on
   20,000 strings (the old expansion differed on 1,301 with non-BMP characters and 31 with
   ranges across the surrogate block).
8. CBOR RegExp tags (35, 21065, 21066) reject invalid patterns and flags,
   with `regress` (accepted differences in `cbor.rs`).
11. Recipes with a number argument that `parseFloat` reads as NaN (`"abc"`,
    `""`, `true`, objects) fail to load, as CyberChef's `Ingredient.prepare`
    makes them; text is read with `parseFloat` (`"8px"` is 8, `"0x10"` is 0),
    and `String()` of arrays (`[4]` is 4). `null` still means the default
    (CyberChef passes `null` to the operation).
12. Confirmed: `split_whitespace_runs`, `strip_delim` and `parse_int` used
    Rust's whitespace (U+0085 in, U+FEFF out). They and recipe parsing now
    use JavaScript's; 3,204 of 7,248 inputs holding U+0085 or U+FEFF changed,
    no other input did (12,752).

Still open: none.

## Upstream issues found

- `regress` 0.12.0: accepts a quantified `\b` or `\B` (`\b*`, `\B{2}`),
  which ECMAScript forbids; rejects patterns nested more than 255 deep or
  with more than 65,535 quantifiers, which V8 accepts; recurses once per `|`
  of an alternation (about 1 KiB each in a debug build, so a few thousand
  overflow a 512 KiB stack), and its optimizer is quadratic in long
  alternations (47 s for 100,000 in a debug build). Not yet reported.

- `saphyr-parser` 0.2.2: `load_from_str` never returns when the input ends
  in a line starting with `%` and no line break (`"%"`, `"a: 1\n%x"`). The
  YAML adapter appends a line break. Other spec deviations in 0.2.2: a block
  scalar without content lines at the end of the input reads as `"\n"`
  (§8.1.1.2: `""` when clipped or stripped; the adapter fixes it from the
  header); a root block scalar's indentation indicator counts from column 0,
  not -1 (§9.1.3; the adapter lowers indicators 2 to 9, `|1` stays wrong);
  a tab after `:` is rejected (§6.2 allows it); yaml.org tags on
  collections are dropped by `saphyr`'s loader (`!!str [a]` loads); flow
  collections nest at most 255 deep. Not yet reported.
- `utilify-codepages`: UTF-7 rejected a surrogate pair split over two shifted
  runs (`+2D0-+3gA-`); fixed upstream (Unreleased).
