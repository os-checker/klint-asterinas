// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use rustc_ast::tokenstream::{self, TokenTree};
use rustc_ast::{DelimArgs, LitKind, MetaItemLit, token};
use rustc_errors::{Diag, ErrorGuaranteed};
use rustc_hir::{AttrArgs, AttrItem, Attribute, HirId};
use rustc_middle::ty::TyCtxt;
use rustc_span::symbol::Ident;
use rustc_span::{Span, Symbol, sym};

use crate::preempt_count::ExpectationRange;

#[derive(Debug, Clone, Copy, Encodable, Decodable)]
pub struct PreemptionCount {
    pub adjustment: Option<i32>,
    pub expectation: Option<ExpectationRange>,
    pub unchecked: bool,
}

impl Default for PreemptionCount {
    fn default() -> Self {
        PreemptionCount {
            adjustment: None,
            expectation: None,
            unchecked: false,
        }
    }
}

#[derive(Debug)]
pub enum KlintAttribute {
    PreemptionCount(PreemptionCount),
    DropPreemptionCount(PreemptionCount),
    ReportPreeptionCount,
    DumpMir,
    /// Make an item known to klint as special.
    ///
    /// This is similar to `rustc_diagnostic_item` in the Rust standard library.
    DiagnosticItem(Symbol),
}

struct Cursor<'a> {
    eof: TokenTree,
    cursor: tokenstream::TokenStreamIter<'a>,
}

impl<'a> Cursor<'a> {
    fn new(cursor: tokenstream::TokenStreamIter<'a>, end_span: Span) -> Self {
        let eof = TokenTree::Token(
            token::Token {
                kind: token::TokenKind::Eof,
                span: end_span,
            },
            tokenstream::Spacing::Alone,
        );
        Cursor { eof, cursor }
    }

    fn is_eof(&self) -> bool {
        self.cursor.peek().is_none()
    }

    fn peek(&self) -> &TokenTree {
        self.cursor.peek().unwrap_or(&self.eof)
    }

    fn next(&mut self) -> &TokenTree {
        self.cursor.next().unwrap_or(&self.eof)
    }
}

struct AttrParser<'tcx> {
    tcx: TyCtxt<'tcx>,
    hir_id: HirId,
}

impl<'tcx> AttrParser<'tcx> {
    fn error(
        &self,
        span: Span,
        decorate: impl for<'a, 'b> FnOnce(&'b mut Diag<'a, ()>),
    ) -> Result<!, ErrorGuaranteed> {
        self.tcx
            .node_span_lint(crate::INCORRECT_ATTRIBUTE, self.hir_id, span, |lint| {
                lint.primary_message("incorrect usage of `#[kint::preempt_count]`");
                decorate(lint);
            });
        Err(self
            .tcx
            .dcx()
            .span_delayed_bug(span, "incorrect usage of `#[kint::preempt_count]`"))
    }

    fn parse_comma_delimited(
        &self,
        mut cursor: Cursor<'_>,
        mut f: impl for<'a> FnMut(Cursor<'a>) -> Result<Cursor<'a>, ErrorGuaranteed>,
    ) -> Result<(), ErrorGuaranteed> {
        loop {
            if cursor.is_eof() {
                return Ok(());
            }

            cursor = f(cursor)?;

            if cursor.is_eof() {
                return Ok(());
            }

            // Check and skip `,`.
            let comma = cursor.next();
            if !matches!(
                comma,
                TokenTree::Token(
                    token::Token {
                        kind: token::TokenKind::Comma,
                        ..
                    },
                    _
                )
            ) {
                self.error(comma.span(), |diag| {
                    diag.help("`,` expected between property values");
                })?;
            }
        }
    }

    fn parse_eq_delimited<'a>(
        &self,
        mut cursor: Cursor<'a>,
        need_eq: impl FnOnce(Ident) -> Result<bool, ErrorGuaranteed>,
        f: impl FnOnce(Ident, Cursor<'a>) -> Result<Cursor<'a>, ErrorGuaranteed>,
    ) -> Result<Cursor<'a>, ErrorGuaranteed> {
        let prop = cursor.next();
        let invalid_prop = |span| {
            self.error(span, |diag| {
                diag.help("identifier expected");
            })?;
        };

        let TokenTree::Token(token, _) = prop else {
            return invalid_prop(prop.span());
        };
        let Some((name, _)) = token.ident() else {
            return invalid_prop(token.span);
        };

        let need_eq = need_eq(name)?;

        // Check and skip `=`.
        let eq = cursor.peek();
        let is_eq = matches!(
            eq,
            TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::Eq,
                    ..
                },
                _
            )
        );
        if need_eq && !is_eq {
            self.error(eq.span(), |diag| {
                diag.help("`=` expected after property name");
            })?;
        }
        if !need_eq && is_eq {
            self.error(eq.span(), |diag| {
                diag.help("unexpected `=` after property name");
            })?;
        }

        if is_eq {
            cursor.next();
        }

        cursor = f(name, cursor)?;

        Ok(cursor)
    }

    fn parse_i32<'a>(&self, mut cursor: Cursor<'a>) -> Result<(i32, Cursor<'a>), ErrorGuaranteed> {
        let expect_int = |span| {
            self.error(span, |diag| {
                diag.help("an integer expected");
            })
        };

        let negative = if matches!(
            cursor.peek(),
            TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::Minus,
                    ..
                },
                _
            )
        ) {
            cursor.next();
            true
        } else {
            false
        };

        let token = cursor.next();
        let TokenTree::Token(
            token::Token {
                kind: token::TokenKind::Literal(lit),
                ..
            },
            _,
        ) = token
        else {
            expect_int(token.span())?
        };
        if lit.kind != token::LitKind::Integer || lit.suffix.is_some() {
            expect_int(token.span())?;
        }
        let Some(v) = lit.symbol.as_str().parse::<i32>().ok() else {
            expect_int(token.span())?;
        };
        let v = if negative { -v } else { v };

        Ok((v, cursor))
    }

    fn parse_expectation_range<'a>(
        &self,
        mut cursor: Cursor<'a>,
    ) -> Result<((u32, Option<u32>), Cursor<'a>), ErrorGuaranteed> {
        let expect_range = |span| {
            self.error(span, |diag| {
                diag.help("a range expected");
            })
        };

        let start_span = cursor.peek().span();
        let mut start = 0;
        if !matches!(
            cursor.peek(),
            TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::DotDot | token::TokenKind::DotDotEq,
                    ..
                },
                _
            )
        ) {
            let token = cursor.next();
            let TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::Literal(lit),
                    ..
                },
                _,
            ) = token
            else {
                expect_range(token.span())?
            };
            if lit.kind != token::LitKind::Integer {
                expect_range(token.span())?;
            }
            let Some(v) = lit.symbol.as_str().parse::<u32>().ok() else {
                expect_range(token.span())?;
            };
            start = v;
        }

        let inclusive = match cursor.peek() {
            TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::DotDot,
                    ..
                },
                _,
            ) => Some(false),
            TokenTree::Token(
                token::Token {
                    kind: token::TokenKind::DotDotEq,
                    ..
                },
                _,
            ) => Some(true),
            _ => None,
        };

        let mut end = Some(start + 1);
        if let Some(inclusive) = inclusive {
            cursor.next();

            let skip_hi = match cursor.peek() {
                TokenTree::Token(
                    token::Token {
                        kind: token::TokenKind::Comma | token::TokenKind::Eof,
                        ..
                    },
                    _,
                ) => true,
                _ => false,
            };

            if skip_hi {
                end = None;
            } else {
                let token = cursor.next();
                let TokenTree::Token(
                    token::Token {
                        kind: token::TokenKind::Literal(lit),
                        ..
                    },
                    _,
                ) = token
                else {
                    expect_range(token.span())?
                };
                if lit.kind != token::LitKind::Integer {
                    expect_range(token.span())?;
                }
                let Some(range) = lit.symbol.as_str().parse::<u32>().ok() else {
                    expect_range(token.span())?;
                };

                end = Some(if inclusive { range + 1 } else { range });
            }
        }

        if end.is_some() && end.unwrap() <= start {
            let end_span = cursor.next().span();

            self.error(start_span.until(end_span), |diag| {
                diag.help("the preemption count expectation range must be non-empty");
            })?;
        }

        Ok(((start, end), cursor))
    }

    fn parse_preempt_count(
        &self,
        attr: &Attribute,
        item: &AttrItem,
    ) -> Result<PreemptionCount, ErrorGuaranteed> {
        let mut adjustment = None;
        let mut expectation = None;
        let mut unchecked = false;

        let AttrArgs::Delimited(DelimArgs {
            dspan: delim_span,
            tokens: tts,
            ..
        }) = &item.args
        else {
            self.error(attr.span(), |diag| {
                diag.help("correct usage looks like `#[kint::preempt_count(...)]`");
            })?;
        };

        self.parse_comma_delimited(Cursor::new(tts.iter(), delim_span.close), |cursor| {
            self.parse_eq_delimited(
                cursor,
                |name| {
                    Ok(match name.name {
                        crate::symbol::adjust | sym::expect => true,
                        crate::symbol::unchecked => false,
                        _ => {
                            self.error(name.span, |diag| {
                                diag.help(
                                    "unknown property, expected `adjust`, `expect` or `unchecked`",
                                );
                            })?;
                        }
                    })
                },
                |name, mut cursor| {
                    match name.name {
                        crate::symbol::adjust => {
                            let v;
                            (v, cursor) = self.parse_i32(cursor)?;
                            adjustment = Some(v);
                        }
                        sym::expect => {
                            let (lo, hi);
                            ((lo, hi), cursor) = self.parse_expectation_range(cursor)?;
                            expectation = Some(ExpectationRange { lo, hi });
                        }
                        crate::symbol::unchecked => {
                            unchecked = true;
                        }
                        _ => unreachable!(),
                    }

                    Ok(cursor)
                },
            )
        })?;

        if adjustment.is_none() && expectation.is_none() {
            self.error(delim_span.entire(), |diag| {
                diag.help("at least one of `adjust` or `expect` property must be specified");
            })?;
        }

        Ok(PreemptionCount {
            adjustment,
            expectation,
            unchecked,
        })
    }

    fn parse(&self, attr: &Attribute) -> Option<KlintAttribute> {
        let Attribute::Unparsed(item) = attr else {
            return None;
        };
        if item.path.segments[0] != crate::symbol::klint {
            return None;
        };
        if item.path.segments.len() != 2 {
            self.tcx
                .node_span_lint(crate::INCORRECT_ATTRIBUTE, self.hir_id, item.span, |lint| {
                    lint.primary_message("invalid klint attribute");
                });
            return None;
        }
        match item.path.segments[1] {
            // Shorthands
            crate::symbol::any_context | crate::symbol::atomic_context => {
                Some(KlintAttribute::PreemptionCount(PreemptionCount {
                    adjustment: None,
                    expectation: Some(ExpectationRange::top()),
                    unchecked: false,
                }))
            }
            crate::symbol::atomic_context_only => {
                Some(KlintAttribute::PreemptionCount(PreemptionCount {
                    adjustment: None,
                    expectation: Some(ExpectationRange { lo: 1, hi: None }),
                    unchecked: false,
                }))
            }
            crate::symbol::process_context => {
                Some(KlintAttribute::PreemptionCount(PreemptionCount {
                    adjustment: None,
                    expectation: Some(ExpectationRange::single_value(0)),
                    unchecked: false,
                }))
            }

            crate::symbol::preempt_count => Some(KlintAttribute::PreemptionCount(
                self.parse_preempt_count(attr, item).ok()?,
            )),
            crate::symbol::drop_preempt_count => Some(KlintAttribute::DropPreemptionCount(
                self.parse_preempt_count(attr, item).ok()?,
            )),
            crate::symbol::report_preempt_count => Some(KlintAttribute::ReportPreeptionCount),
            crate::symbol::dump_mir => Some(KlintAttribute::DumpMir),
            crate::symbol::diagnostic_item => {
                let AttrArgs::Eq {
                    eq_span: _,
                    expr:
                        MetaItemLit {
                            kind: LitKind::Str(value, _),
                            ..
                        },
                } = item.args
                else {
                    self.error(attr.span(), |diag| {
                        diag.help(
                            r#"correct usage looks like `#[kint::diagnostic_item = "name"]`"#,
                        );
                    })
                    .ok()?;
                };

                Some(KlintAttribute::DiagnosticItem(value))
            }
            _ => {
                self.tcx.node_span_lint(
                    crate::INCORRECT_ATTRIBUTE,
                    self.hir_id,
                    item.path.span,
                    |lint| {
                        lint.primary_message("unrecognized klint attribute");
                    },
                );
                None
            }
        }
    }
}

pub fn parse_klint_attribute(
    tcx: TyCtxt<'_>,
    hir_id: HirId,
    attr: &Attribute,
) -> Option<KlintAttribute> {
    AttrParser { tcx, hir_id }.parse(attr)
}

memoize!(
    pub fn klint_attributes<'tcx>(
        cx: &AnalysisCtxt<'tcx>,
        hir_id: HirId,
    ) -> Arc<Vec<KlintAttribute>> {
        let mut v = Vec::new();
        for attr in cx.hir_attrs(hir_id) {
            let Some(attr) = crate::attribute::parse_klint_attribute(cx.tcx, hir_id, attr) else {
                continue;
            };
            v.push(attr);
        }
        Arc::new(v)
    }
);
