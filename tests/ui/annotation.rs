// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![crate_type = "lib"]

#[klint::preempt_count]
fn a() {}

#[klint::preempt_count()]
fn b() {}

#[klint::preempt_count(adjust = )]
fn c() {}

#[klint::preempt_count(expect = )]
fn d() {}

#[klint::preempt_count(expect = ..)]
fn e() {}

#[klint::preempt_count(unchecked)]
fn f() {}

#[klint::any_context]
fn g() {}

#[klint::atomic_context]
fn h() {}

#[klint::atomic_context_only]
fn i() {}

#[klint::process_context]
fn j() {}
