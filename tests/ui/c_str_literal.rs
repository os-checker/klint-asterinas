#![deny(klint::c_str_literal)]

#[klint::diagnostic_item = "c_str"]
macro_rules! c_str {
    ($str:expr) => {{
        const S: &str = concat!($str, "\0");
        const C: &::core::ffi::CStr = match ::core::ffi::CStr::from_bytes_with_nul(S.as_bytes()) {
            Ok(v) => v,
            Err(_) => panic!("string contains interior NUL"),
        };
        C
    }};
}

macro_rules! forward_c_str {
    ($str:expr) => {
        c_str!($str)
    };
}

macro_rules! wrap_c_str {
    () => {
        c_str!("hello")
    };
}

fn main() {
    // Should warn.
    c_str!("hello");

    // Should not warn.
    c_str!(concat!("a", "b"));
    // Should not warn.
    c_str!(stringify!(hello));

    // Should not warn.
    forward_c_str!("a");

    // Should warn. We currently do not have ability to warn directly at `macro_rules!`; warning
    // is only generated once the macro is used.
    wrap_c_str!();
}
