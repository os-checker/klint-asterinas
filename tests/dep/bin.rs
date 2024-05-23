// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use spin::*;

fn main() {
    let lock = Spinlock;
    drop(lock);
}
