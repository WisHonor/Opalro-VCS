#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Encoding is canonical, so anything that decodes must encode back to the same bytes.
    if let Ok(record) = opal_core::conflict::Record::decode(data) {
        assert_eq!(record.encode(), data);
    }
});
