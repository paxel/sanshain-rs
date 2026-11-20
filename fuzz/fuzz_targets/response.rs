//! What the service answers when a call fails: any status, any body and any
//! `Sanshain-Warning` header values, never a panic.
//!
//! The first two bytes are the status. The rest is read twice: as the body of the failure,
//! and, split at line feeds, as the values of the warning headers.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain::fuzzing::{error_messages, warning_lines};

fuzz_target!(|data: &[u8]| {
    let Some((status, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let status = u16::from_le_bytes(*status);
    let _ = error_messages(status, &String::from_utf8_lossy(rest));
    let values: Vec<&[u8]> = rest.split(|byte| *byte == b'\n').collect();
    // A header value that is not text is left out; none is ever added.
    assert!(warning_lines(&values).len() <= values.len());
});
