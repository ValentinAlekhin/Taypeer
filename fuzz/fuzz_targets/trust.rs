//! Production signed authority and provenance parser target.
#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| taypeer_parser_fuzz::Parser::Trust.run(bytes));
