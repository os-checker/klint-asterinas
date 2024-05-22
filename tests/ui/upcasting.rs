// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![crate_type="lib"]
#![feature(trait_upcasting)]

#[klint::drop_preempt_count(expect = 0)]
trait A {}

#[klint::drop_preempt_count(expect = 1)]
trait B: A {}

fn upcast(x: &dyn B) -> &dyn A {
    x
}
