#![deny(klint::stack_frame_too_large)]

#[unsafe(no_mangle)]
fn very_large_frame() {
    core::hint::black_box([0; 1024]);
}
