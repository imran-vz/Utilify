//! CSV to and from JSON, read and written by `utilify-csv` per RFC 4180.
//!
//! CSV to JSON reads with `ReadOptions::RFC4180`: quotes per §2 rules 5–7,
//! so a quote inside an unquoted field (`a"b"`), text after a closing quote
//! (`"a"b`) and an unclosed quote are errors. Control characters (a tab
//! in a cell) are allowed, as non-ASCII text is: the ABNF's `TEXTDATA` is
//! printable ASCII only, which no real CSV keeps to, and §3 leaves the
//! character set to the MIME `charset` parameter. A leading BOM is stripped; an empty line is a record of one empty field
//! (the ABNF), and the last record needs no line break (§2 rule 2), so
//! `x\ny` is two records. Ragged records are allowed (§2 rule 4's "should").
//! Each character of "Cell delimiters" is a delimiter (CyberChef's argument
//! meaning; characters, not UTF-16 units). "Row delimiters" is a set of
//! characters too, read with `ReadOptions::terminators`: each ends one
//! record outside quotes, so two in a row (the same or different) give an
//! empty record between, except CR LF when both are in the set, which is
//! one break (§2 rule 1; CR and LF alone are `Terminator::Any`, a lone CR
//! or LF one break each, as §3 notes). A set holding only one of CR and LF
//! reads the other as data. A delimiter that is the quote, or both a cell
//! and a row delimiter, and an empty row delimiter, are argument errors
//! (`InvalidOptions`), where CyberChef silently dropped them.
//!
//! JSON to CSV writes with `WriteOptions::RFC4180`, `QuoteStyle::Necessary`
//! (§2 rules 6–7, plus the writer's rules for separator strings and a
//! leading U+FEFF), the arguments as `delimiter_str`/`line_ending_str` and
//! `flexible(true)`. Separators the writer refuses (empty, over 32 bytes,
//! holding the quote or a line break in the cell delimiter, one inside the
//! other) are errors; control characters in cells are written as they are. What stays
//! CyberChef's is presentation, which RFC 4180 doesn't define: an array of
//! arrays is written row by row; otherwise the rows are objects and the
//! first row's keys are the header (§2 rule 3), a missing cell is empty;
//! cells are `String(v)` (ECMA-262), and when a cell is nested or a row
//! doesn't fit, the whole input is flattened into one row with the `flat`
//! package's dotted keys (`prev ? prev + "." + key : key`, so an empty-string
//! parent key adds no dot), nested values as JSON. A top-level `null`,
//! boolean, number or string is no table and is an error, where CyberChef
//! read strings as their UTF-16 units.

use super::CATEGORY;
use super::js::{Js, Obj, json_dish, json_input, stringify_compact, with_big_stack};
use crate::{Arg, Args, Dish, OpError, Operation};
use utilify_csv::{QuoteStyle, ReadOptions, WriteOptions};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&CSV_TO_JSON, &JSON_TO_CSV]
}

pub static CSV_TO_JSON: Operation = Operation {
    name: "CSV to JSON",
    category: CATEGORY,
    description: "Converts a CSV file to JSON format.",
    info_url: Some("https://wikipedia.org/wiki/Comma-separated_values"),
    args: &[
        Arg::binary("Cell delimiters", ","),
        Arg::binary("Row delimiters", "\\r\\n"),
        Arg::choice("Format", &["Array of dictionaries", "Array of arrays"]),
    ],
    run: csv_to_json_op,
};

fn csv_to_json_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let rows = parse_csv(&input.text(), args.str(0), args.str(1))
        .map_err(|e| OpError::new(format!("CSV to JSON - {e}")))?;
    let json = match args.str(2) {
        "Array of dictionaries" => {
            let header = rows.first().cloned().unwrap_or_default();
            rows.iter()
                .skip(1)
                .map(|row| {
                    let mut obj = Obj::new();
                    // A short row leaves its missing keys out; every header
                    // name, `__proto__` too, is data.
                    for (h, c) in header.iter().zip(row) {
                        obj.insert(h.clone(), Js::str(c.as_str()));
                    }
                    Js::Obj(obj)
                })
                .collect()
        }
        _ => rows
            .into_iter()
            .map(|row| Js::Arr(row.into_iter().map(Js::Str).collect()))
            .collect(),
    };
    json_dish(&Js::Arr(json))
}

/// Reads `data` with each character of `cells` a field delimiter and each
/// of `rows` a record terminator.
fn parse_csv(data: &str, cells: &str, rows: &str) -> Result<Vec<Vec<String>>, utilify_csv::Error> {
    let cell_set: Vec<char> = cells.chars().collect();
    let row_set: Vec<char> = rows.chars().collect();
    ReadOptions::RFC4180
        .delimiters(&cell_set)
        .terminators(&row_set)
        .allow_control_chars(true)
        .flexible(true)
        .has_headers(false)
        .max_record_len(usize::MAX)
        .max_fields(usize::MAX)
        .max_skip_len(usize::MAX)
        .read(data)
}

pub static JSON_TO_CSV: Operation = Operation {
    name: "JSON to CSV",
    category: CATEGORY,
    description: "Converts JSON data to a CSV based on the definition in RFC 4180.",
    info_url: Some("https://wikipedia.org/wiki/Comma-separated_values"),
    args: &[
        Arg::binary("Cell delimiter", ","),
        Arg::binary("Row delimiter", "\\r\\n"),
    ],
    run: |input, args| {
        // Parsing, flattening, printing and dropping the value recurse.
        with_big_stack(|| {
            let json = json_input(&input)?;
            json_to_csv(&json, args.str(0), args.str(1))
                .map(Dish::Text)
                .map_err(|e| OpError::new(format!("JSON to CSV - {e}")))
        })
    },
};

/// The records for `input`: rows as they are, and if a row doesn't fit (a
/// nested value, a row that isn't an object or array like the first), the
/// whole input flattened into one row with dotted keys.
fn json_to_csv(input: &Js, cell: &str, row: &str) -> Result<String, String> {
    let rows = match input {
        Js::Arr(items) => items.as_slice(),
        Js::Obj(_) => std::slice::from_ref(input),
        _ => return Err("expected an array or an object".into()),
    };
    let records = records(rows, false)
        .unwrap_or_else(|| records(&[Js::Obj(flatten(input))], true).unwrap_or_default());
    WriteOptions::RFC4180
        .delimiter_str(cell)
        .line_ending_str(row)
        .quote_style(QuoteStyle::Necessary)
        .allow_control_chars(true)
        .flexible(true)
        .max_record_len(usize::MAX)
        .max_fields(usize::MAX)
        .write(&records)
        .map_err(|e| e.to_string())
}

/// The records of `rows`: arrays as they are, or objects under a header of
/// the first one's keys. `None` if a row doesn't fit or (unless `force`) a
/// cell is nested.
fn records(rows: &[Js], force: bool) -> Option<Vec<Vec<String>>> {
    match rows.first() {
        Some(Js::Arr(_)) => rows
            .iter()
            .map(|r| match r {
                Js::Arr(cells) => cells.iter().map(|d| cell_text(d, force)).collect(),
                _ => None,
            })
            .collect(),
        Some(Js::Obj(first)) => {
            let header = first.keys();
            let mut records = Vec::with_capacity(rows.len() + 1);
            for r in rows {
                let Js::Obj(o) = r else { return None };
                let cells: Option<Vec<String>> = header
                    .iter()
                    .map(|h| {
                        o.get(h)
                            .map_or(Some(String::new()), |v| cell_text(v, force))
                    })
                    .collect();
                records.push(cells?);
            }
            records.insert(0, header);
            Some(records)
        }
        _ => None,
    }
}

/// A cell's text: primitives as `String(v)`, nested values as JSON only
/// when forced.
fn cell_text(data: &Js, force: bool) -> Option<String> {
    if !data.is_object_like() {
        return Some(data.to_js_string());
    }
    force.then(|| stringify_compact(data).ok().flatten().unwrap_or_default())
}

/// The `flat` package's `flatten`: nested non-empty objects and arrays
/// become dotted keys; empty ones stay as values. Its `prev ? prev + "." +
/// key : key` treats an empty-string parent key as no parent.
fn flatten(v: &Js) -> Obj {
    fn entries(v: &Js) -> Vec<(String, Js)> {
        match v {
            Js::Obj(o) => o.iter().map(|(k, v)| (k.into_owned(), v.clone())).collect(),
            Js::Arr(a) => a
                .iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v.clone()))
                .collect(),
            _ => vec![],
        }
    }
    fn step(v: &Js, prev: Option<&str>, out: &mut Obj) {
        for (key, value) in entries(v) {
            let new_key = match prev {
                Some(p) if !p.is_empty() => format!("{p}.{key}"),
                _ => key,
            };
            let nested = match &value {
                Js::Obj(o) => !o.is_empty(),
                Js::Arr(a) => !a.is_empty(),
                _ => false,
            };
            if nested {
                step(&value, Some(&new_key), out);
            } else {
                out.insert(new_key, value);
            }
        }
    }
    let mut out = Obj::new();
    step(v, None, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(op: &Operation, input: &str, args: &[&str]) -> String {
        let args: Vec<ArgValue> = args.iter().map(|a| ArgValue::Str((*a).into())).collect();
        match op.apply(Dish::Text(input.into()), &args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    #[test]
    fn csv_to_json_per_rfc4180() {
        let arrays = |input: &str, cells: &str, rows: &str| {
            run(&CSV_TO_JSON, input, &[cells, rows, "Array of arrays"])
        };
        let dicts = run(
            &CSV_TO_JSON,
            "a,b,__proto__\r\n1\r\n2,3,4\r\n",
            &[",", "\\r\\n", "Array of dictionaries"],
        );
        assert_eq!(
            dicts,
            "[\n    {\n        \"a\": \"1\"\n    },\n    {\n        \"a\": \"2\",\n        \"b\": \"3\",\n        \"__proto__\": \"4\"\n    }\n]"
        );
        // §2 rule 2: the last record needs no line break.
        assert_eq!(
            arrays("x\ny", ",", "\\n"),
            "[\n    [\n        \"x\"\n    ],\n    [\n        \"y\"\n    ]\n]"
        );
        assert_eq!(
            run(&CSV_TO_JSON, "", &[",", "\\r\\n", "Array of dictionaries"]),
            "[]"
        );
        // CRLF is one break, a lone CR or LF one each; an empty line is one
        // empty field; a quoted break is data.
        assert_eq!(
            arrays("a;b\n\r\"c\r\nd\"\r\ne,f", ",;", "\\r\\n"),
            "[\n    [\n        \"a\",\n        \"b\"\n    ],\n    [\n        \"\"\n    ],\n    [\n        \"c\\r\\nd\"\n    ],\n    [\n        \"e\",\n        \"f\"\n    ]\n]"
        );
        // Several row delimiters, none a line break.
        assert_eq!(
            arrays("a,\"\"\"\"|b,c;", ",", "|;"),
            "[\n    [\n        \"a\",\n        \"\\\"\"\n    ],\n    [\n        \"b\",\n        \"c\"\n    ]\n]"
        );
        let compact = |input: &str, rows: &str| {
            let out = arrays(input, ",", rows);
            let v: serde_json::Value = serde_json::from_str(&out).expect(&out);
            v.to_string()
        };
        // A row delimiter inside quotes is data.
        assert_eq!(compact("\"a|b\";c", "|;"), r#"[["a|b"],["c"]]"#);
        // Adjacent delimiters, the same or different, end one record each.
        assert_eq!(compact("a|;b||c", "|;"), r#"[["a"],[""],["b"],[""],["c"]]"#);
        // With CR and LF in a larger set, CR LF is one break, LF CR two.
        assert_eq!(
            compact("a\r\nb\n\rc;\nd", "\\r\\n;"),
            r#"[["a"],["b"],[""],["c"],[""],["d"]]"#
        );
        // Only one of CR and LF in the set: the other is data.
        assert_eq!(compact("a\r\nb;c", "\\n;"), r#"[["a\r"],["b"],["c"]]"#);
        // §2 rules 5–7: stray quotes are errors.
        for bad in ["a\"b\"\n", "\"a\"b\n", "\"a\n"] {
            assert!(
                arrays(bad, ",", "\\n").starts_with("CSV to JSON - "),
                "{bad:?}"
            );
        }
        assert!(
            arrays("a\"b\"\n", ",", "\\n")
                .starts_with("CSV to JSON - quote inside an unquoted field")
        );
        // Control characters are kept, in cells and as delimiters.
        assert_eq!(
            arrays("a\tb", ",", "\\n"),
            "[\n    [\n        \"a\\tb\"\n    ]\n]"
        );
        assert_eq!(
            arrays("a\tb", "\t", "\\n"),
            "[\n    [\n        \"a\",\n        \"b\"\n    ]\n]"
        );
        // Delimiter arguments the reader refuses.
        for (cells, rows) in [
            (",\"", "\\n"),
            (",", ""),
            (",", "\"|"),
            (",|", "|;"),
            (",", ","),
        ] {
            assert!(
                arrays("a", cells, rows).starts_with("CSV to JSON - invalid options"),
                "{cells:?} {rows:?}"
            );
        }
    }

    #[test]
    fn json_to_csv_per_rfc4180() {
        let args = [",", "\\r\\n"];
        assert_eq!(
            run(&JSON_TO_CSV, r#"[[1,null,true],["a\"b"]]"#, &args),
            "1,null,true\r\n\"a\"\"b\"\r\n"
        );
        assert_eq!(
            run(&JSON_TO_CSV, r#"{"a":{"b":[1,{}]},"c":"x"}"#, &args),
            "a.b.0,a.b.1,c\r\n1,{},x\r\n"
        );
        // A missing cell is empty.
        assert_eq!(
            run(&JSON_TO_CSV, r#"[{"a":1,"b":2},{"b":3}]"#, &args),
            "a,b\r\n1,2\r\n,3\r\n"
        );
        // `flat` joins no parent for an empty-string key.
        assert_eq!(
            run(
                &JSON_TO_CSV,
                r#"{"":{"k":1,"":{"j":[2]}},"x":{"":3}}"#,
                &args
            ),
            "k,j.0,x.\r\n1,2,3\r\n"
        );
        // Rows that don't fit the first: the input flattened.
        assert_eq!(run(&JSON_TO_CSV, r#"[1,"a"]"#, &args), "0,1\r\n1,a\r\n");
        let deep = format!("{}1{}", "[".repeat(4900), "]".repeat(4900));
        let out = run(&JSON_TO_CSV, &format!("{{\"a\":{deep}}}"), &args);
        assert_eq!(out, format!("a{}\r\n1\r\n", ".0".repeat(4900)));
        // Ragged rows; a lone empty field is quoted so it reads back.
        assert_eq!(
            run(&JSON_TO_CSV, r#"[[1,2],[""],[],["a\nb"]]"#, &args),
            "1,2\r\n\"\"\r\n\r\n\"a\nb\"\r\n"
        );
        assert_eq!(
            run(&JSON_TO_CSV, r#"[["a;b","c d"]]"#, &["; ", "\\n"]),
            "a;b; c d\n"
        );
        // §2 rule 6 and the writer's rules: a leading BOM, a cell that ends
        // with the start of a separator string, a line break of any kind.
        assert_eq!(
            run(&JSON_TO_CSV, "[[\"\u{feff}a\",\"b\\r\"]]", &args),
            "\"\u{feff}a\",\"b\r\"\r\n"
        );
        assert_eq!(
            run(&JSON_TO_CSV, r#"[["a|","b"]]"#, &["||", "\\n"]),
            "\"a|\"||b\n"
        );
        // Separators the writer refuses.
        let long = "-".repeat(40);
        for (cell, row) in [
            ("", "\\n"),
            (&long, "\\n"),
            (",", ""),
            ("\"", "\\n"),
            ("ab", "abc"),
        ] {
            assert!(
                run(&JSON_TO_CSV, r#"[["a","b"]]"#, &[cell, row])
                    .starts_with("JSON to CSV - invalid options"),
                "{cell:?} {row:?}"
            );
        }
        assert_eq!(run(&JSON_TO_CSV, r#"[["a\tb"]]"#, &args), "a\tb\r\n");
        for scalar in ["null", "5", "\"ab\""] {
            assert_eq!(
                run(&JSON_TO_CSV, scalar, &args),
                "JSON to CSV - expected an array or an object"
            );
        }
    }

    #[test]
    fn deep_input_fits_small_stacks() {
        let run = || {
            let deep = format!("{}1{}", "[".repeat(4999), "]".repeat(4999));
            let out = run(&JSON_TO_CSV, &format!("[{deep}]"), &[",", "\\n"]);
            assert!(out.starts_with("0.0.0."), "{}", &out[..20.min(out.len())]);
            let out = run(&JSON_TO_CSV, &format!("{{\"a\":[{deep}]}}"), &[",", "\\n"]);
            assert!(out.starts_with("a.0.0."));
        };
        let thread = std::thread::Builder::new().stack_size(512 << 10).spawn(run);
        assert!(thread.unwrap().join().is_ok());
    }
}
