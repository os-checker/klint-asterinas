// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

pub struct X;

impl Drop for X {
    #[klint::preempt_count(expect = 0)]
    #[inline(never)]
    fn drop(&mut self) {}
}

#[klint::preempt_count(expect = 0..)]
pub fn foo(x: Option<X>) -> Option<X> {
    // This control flow only conditionally moved `x`, but it will need dropping anymore
    // regardless if this branch is taken.
    // It's important that we do not consider the destructor to possibly run at the end of scope.
    if let Some(x) = x {
        return Some(x);
    }
    None
}
