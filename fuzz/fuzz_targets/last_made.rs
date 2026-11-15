#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(made) = opal_core::lastmade::LastMade::decode(data) {
        // What reads back writes back the same.
        assert_eq!(made.encode(), data);
    }
    let _ = opal_core::keyed::open_sealed(&[0; 32], data);
});
