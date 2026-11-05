//! Consistency checks for the test corpus (corpus/prompts/*.json).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn prompts() -> BTreeMap<String, Value> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/prompts");
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(&dir).expect("corpus/prompts exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{stem}.json: {e}"));
        out.insert(stem, v);
    }
    out
}

#[test]
fn every_prompt_file_is_well_formed() {
    let all = prompts();
    assert!(!all.is_empty());
    let categories = ["greenfield", "edit", "trap", "control", "vague"];
    let polarities = ["must", "must_not", "prefer"];
    for (stem, p) in &all {
        assert_eq!(p["schema"], 1, "{stem}");
        assert_eq!(p["id"].as_str(), Some(stem.as_str()), "id must match file name");
        assert!(categories.contains(&p["category"].as_str().unwrap()), "{stem}: category");
        assert_eq!(p["language"], "ts", "{stem}: JS/TS only for v1");
        let turns = p["turns"].as_array().expect("turns");
        assert!(!turns.is_empty() && turns.iter().all(|t| !t.as_str().unwrap().trim().is_empty()), "{stem}: turns");
        let mut ids = BTreeSet::new();
        for fact in p["expected_facts"].as_array().expect("expected_facts") {
            let id = fact["id"].as_str().expect("fact id");
            assert!(ids.insert(id), "{stem}: duplicate fact id {id}");
            assert!(fact["description"].is_string(), "{stem}/{id}: description");
            assert!(fact["kind"].is_string(), "{stem}/{id}: kind");
            assert!(polarities.contains(&fact["polarity"].as_str().unwrap()), "{stem}/{id}: polarity");
        }
        assert!(p["couldnt_check"].is_array() && p["traps"].is_array(), "{stem}");
        assert!(p["first_batch"].is_boolean(), "{stem}: first_batch");
    }
}

#[test]
fn bases_exist_and_are_greenfield() {
    let all = prompts();
    for (stem, p) in &all {
        let is_greenfield = p["category"] == "greenfield";
        match p["base"].get("from").and_then(Value::as_str) {
            None => assert!(is_greenfield, "{stem}: non-greenfield prompts need a base"),
            Some(from) => {
                let base = all.get(from).unwrap_or_else(|| panic!("{stem}: unknown base {from}"));
                assert_eq!(base["category"], "greenfield", "{stem}: base {from} must be greenfield");
            }
        }
    }
}

#[test]
fn first_batch_includes_every_base_it_needs() {
    let all = prompts();
    for (stem, p) in all.iter().filter(|(_, p)| p["first_batch"] == true) {
        if let Some(from) = p["base"].get("from").and_then(Value::as_str) {
            assert_eq!(all[from]["first_batch"], true, "{stem} needs {from} in the first batch");
        }
    }
}

#[test]
fn vague_prompts_have_unverifiable_text() {
    for (stem, p) in prompts().iter().filter(|(_, p)| p["category"] == "vague") {
        assert!(!p["couldnt_check"].as_array().unwrap().is_empty(), "{stem}");
    }
}
