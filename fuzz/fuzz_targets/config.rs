//! `sanshain.yaml` as any text: a configuration or an error, never a panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain::{Config, puzzled_cat};

fuzz_target!(|yaml: &str| {
    if let Ok(loaded) = Config::load_yaml_str(yaml) {
        // What a command does next with a file it could read.
        let _ = puzzled_cat(&loaded.unknown_fields, loaded.strict);
    }
});
