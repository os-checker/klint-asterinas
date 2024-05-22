// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![crate_type="lib"]

#[klint::preempt_count(expect = 0..)]
fn waker_ops(x: &core::task::Waker) {
    x.clone().wake();
    x.wake_by_ref();
}
