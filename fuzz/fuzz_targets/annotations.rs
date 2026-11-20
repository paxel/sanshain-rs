//! Filling the placeholders of annotation values: any value, any `--set` pair and any
//! environment, never a panic, and every entry is either kept or left out with a warning.
//!
//! The first line holds the annotation values, separated by tabs. Every further line is a
//! `NAME=value` pair, by turns a `--set` pair and an environment variable.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain::{Annotation, AnnotationScope, SET_ENV_PREFIX, fill_annotations_with};

fuzz_target!(|text: &str| {
    let mut lines = text.split('\n');
    let declared: Vec<Annotation> = lines
        .next()
        .unwrap_or_default()
        .split('\t')
        .enumerate()
        .map(|(index, value)| Annotation {
            scope: if index % 2 == 0 {
                AnnotationScope::Version
            } else {
                AnnotationScope::System
            },
            name: format!("annotation-{index}"),
            value: value.to_string(),
        })
        .collect();
    let mut sets = Vec::new();
    let mut environment = Vec::new();
    for (index, line) in lines.enumerate() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if index % 2 == 0 {
            sets.push((name.to_string(), value.to_string()));
        } else {
            environment.push((format!("{SET_ENV_PREFIX}{name}"), value.to_string()));
        }
    }
    let filled = fill_annotations_with(&declared, &sets, |variable| {
        environment
            .iter()
            .find(|(name, _)| name == variable)
            .map(|(_, value)| value.clone())
    });
    assert_eq!(
        filled.annotations.len() + filled.warnings.len(),
        declared.len()
    );
});
