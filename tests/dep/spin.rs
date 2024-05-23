// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

pub struct Guard;

impl Drop for Guard {
    #[klint::preempt_count(adjust = -1, unchecked)]
    fn drop(&mut self) {}
}

pub struct Spinlock;

impl Spinlock {
    #[klint::preempt_count(adjust = 1, unchecked)]
    pub fn lock(&self) -> Guard {
        Guard
    }
}
