//! CyberChef's own operation tests, run against the port.
//!
//! `vectors/cyberchef.json` is exported from CyberChef's
//! `tests/operations/tests` by `scripts/cyberchef/export.mjs`. Every vector
//! whose operations are all ported must pass, except those listed with a
//! reason in `vectors/known_differences.txt`.
//!
//! `VECTORS=Base64 cargo test -p utilify-chef --test cyberchef_vectors -- --nocapture`
//! runs only the vectors whose file or name contains `Base64`, and prints
//! each failure in full.

use serde::Deserialize;
use utilify_chef::{Dish, Recipe};

#[derive(Deserialize)]
struct Vector {
    file: String,
    name: String,
    input: String,
    recipe: serde_json::Value,
    expected: Option<String>,
    #[serde(rename = "expectedMatch")]
    expected_match: Option<String>,
}

fn known_differences() -> Vec<String> {
    include_str!("vectors/known_differences.txt")
        .lines()
        .map(|l| l.split(" # ").next().unwrap_or_default().trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[test]
fn cyberchef_vectors() {
    let vectors: Vec<Vector> =
        serde_json::from_str(include_str!("vectors/cyberchef.json")).expect("vectors parse");
    let filter = std::env::var("VECTORS").ok();
    let known = known_differences();
    let (mut ran, mut passed, mut excused) = (0, 0, 0);
    let mut failures = vec![];
    for v in &vectors {
        if let Some(f) = &filter
            && !v.file.contains(f.as_str())
            && !v.name.contains(f.as_str())
        {
            continue;
        }
        // Only vectors whose every operation is ported.
        let Ok(recipe) = Recipe::parse(&v.recipe.to_string()) else {
            continue;
        };
        ran += 1;
        let out = recipe.bake(Dish::Text(v.input.clone()));
        let got = out.dish.display();
        let ok = match (&v.expected, &v.expected_match) {
            (Some(e), _) if *e == got => true,
            (_, Some(re)) => regex::Regex::new(re).is_ok_and(|re| re.is_match(&got)),
            _ => false,
        };
        if ok {
            passed += 1;
        } else if known.contains(&v.name) {
            excused += 1;
        } else {
            let want = v
                .expected
                .as_deref()
                .or(v.expected_match.as_deref())
                .unwrap_or("");
            failures.push(format!(
                "{} [{}]\n  expected: {:?}\n  got:      {:?}",
                v.name,
                v.file,
                truncate(want),
                truncate(&got)
            ));
        }
    }
    println!(
        "CyberChef vectors: {passed}/{ran} pass, {excused} known differences, {} fail ({} not runnable yet)",
        failures.len(),
        vectors.len() - ran
    );
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

fn truncate(s: &str) -> String {
    if std::env::var_os("VECTORS").is_some() || s.chars().count() <= 300 {
        return s.to_string();
    }
    let head: String = s.chars().take(300).collect();
    format!("{head}…")
}
