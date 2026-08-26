// Lexing and parsing must never panic or hang, whatever the input
#![no_main]

use libfuzzer_sys::fuzz_target;

use eclipse_luau::{lexer, parser};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(lexed) = lexer::lex(src) {
        let _ = parser::parse(src, &lexed.toks);
    }
});
