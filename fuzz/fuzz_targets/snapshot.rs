//! Historical snapshots are untrusted files: streaming them never panics and
//! only hands over objects.
#![no_main]

use libfuzzer_sys::fuzz_target;
use observatory_domain::snapshot::{for_each_repository, for_each_user};

fuzz_target!(|data: &[u8]| {
    let _ = for_each_repository(data, |_, repository| {
        assert!(repository.is_object());
        Ok(())
    });
    let _ = for_each_user(data, |_, _| Ok(()));
});
