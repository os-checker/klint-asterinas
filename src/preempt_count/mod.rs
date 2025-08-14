// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod adjustment;
pub mod annotation;
pub mod check;
pub mod dataflow;
pub mod expectation;

use rustc_errors::ErrorGuaranteed;
use rustc_middle::ty::{Instance, PseudoCanonicalInput};
use rustc_span::Span;

use self::dataflow::AdjustmentBounds;
use crate::lattice::MeetSemiLattice;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encodable, Decodable)]
pub enum Error {
    TooGeneric,
    Error(ErrorGuaranteed),
}

pub struct PolyDisplay<'a, 'tcx, T>(pub &'a PseudoCanonicalInput<'tcx, T>);

impl<T> std::fmt::Display for PolyDisplay<'_, '_, T>
where
    T: std::fmt::Display + Copy,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let PseudoCanonicalInput { typing_env, value } = self.0;
        write!(f, "{}", value)?;
        if !typing_env.param_env.caller_bounds().is_empty() {
            write!(f, " where ")?;
            for (i, predicate) in typing_env.param_env.caller_bounds().iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", predicate)?;
            }
        }
        Ok(())
    }
}

/// Range of preemption count that the function expects.
///
/// Since the preemption count is a non-negative integer, the lower bound is just represented using a `u32`
/// and "no expectation" is represented with 0; the upper bound is represented using an `Option<u32>`, with
/// `None` representing "no expectation". The upper bound is exclusive so `(0, Some(0))` represents an
/// unsatisfiable condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encodable, Decodable)]
pub struct ExpectationRange {
    pub lo: u32,
    pub hi: Option<u32>,
}

impl ExpectationRange {
    pub const fn top() -> Self {
        Self { lo: 0, hi: None }
    }

    pub const fn single_value(v: u32) -> Self {
        Self {
            lo: v,
            hi: Some(v + 1),
        }
    }

    pub fn is_empty(&self) -> bool {
        if let Some(hi) = self.hi {
            self.lo >= hi
        } else {
            false
        }
    }

    pub fn contains_range(&self, mut other: Self) -> bool {
        !other.meet(self)
    }
}

impl MeetSemiLattice for ExpectationRange {
    fn meet(&mut self, other: &Self) -> bool {
        let mut changed = false;
        if self.lo < other.lo {
            self.lo = other.lo;
            changed = true;
        }

        match (self.hi, other.hi) {
            (_, None) => (),
            (None, Some(_)) => {
                self.hi = other.hi;
                changed = true;
            }
            (Some(a), Some(b)) => {
                if a > b {
                    self.hi = Some(b);
                    changed = true;
                }
            }
        }

        changed
    }
}

impl std::fmt::Display for ExpectationRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.lo, self.hi) {
            (lo, None) => write!(f, "{}..", lo),
            (lo, Some(hi)) if lo >= hi => write!(f, "unsatisfiable"),
            (lo, Some(hi)) if lo + 1 == hi => write!(f, "{lo}"),
            (lo, Some(hi)) => write!(f, "{}..{}", lo, hi),
        }
    }
}

fn saturating_add(x: u32, y: i32) -> u32 {
    let (res, overflow) = x.overflowing_add(y as u32);
    if overflow == (y < 0) {
        res
    } else if overflow {
        u32::MAX
    } else {
        0
    }
}

impl std::ops::Add<i32> for ExpectationRange {
    type Output = Self;

    fn add(self, rhs: i32) -> Self::Output {
        Self {
            lo: saturating_add(self.lo, rhs),
            hi: self.hi.map(|hi| saturating_add(hi, rhs)),
        }
    }
}

impl std::ops::Sub<i32> for ExpectationRange {
    type Output = Self;

    fn sub(self, rhs: i32) -> Self::Output {
        Self {
            lo: saturating_add(self.lo, -rhs),
            hi: self.hi.map(|hi| saturating_add(hi, -rhs)),
        }
    }
}

impl std::ops::Add<AdjustmentBounds> for ExpectationRange {
    type Output = Self;

    fn add(self, rhs: AdjustmentBounds) -> Self::Output {
        Self {
            lo: match rhs.lo {
                None => 0,
                Some(bound) => saturating_add(self.lo, bound),
            },
            hi: self
                .hi
                .zip(rhs.hi)
                .map(|(hi, bound)| saturating_add(hi, bound - 1)),
        }
    }
}

impl std::ops::Sub<AdjustmentBounds> for ExpectationRange {
    type Output = Self;

    fn sub(self, rhs: AdjustmentBounds) -> Self::Output {
        Self {
            lo: match rhs.lo {
                None => 0,
                Some(bound) => saturating_add(self.lo, -bound),
            },
            hi: match (self.hi, rhs.hi) {
                (None, _) => None,
                (_, None) => Some(0),
                (Some(hi), Some(bound)) => Some(saturating_add(hi, 1 - bound)),
            },
        }
    }
}

#[derive(Debug)]
pub enum UseSiteKind {
    Call(Span),
    Drop {
        /// Span that causes the drop.
        drop_span: Span,
        /// Span of the place being dropped.
        place_span: Span,
    },
    PointerCoercion(Span),
    Vtable(Span),
}

#[derive(Debug)]
pub struct UseSite<'tcx> {
    pub instance: PseudoCanonicalInput<'tcx, Instance<'tcx>>,
    pub kind: UseSiteKind,
}
