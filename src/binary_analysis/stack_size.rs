use object::File;
use rustc_errors::Level;
use rustc_hir::CRATE_HIR_ID;
use rustc_session::declare_tool_lint;
use rustc_span::{Span, Symbol, sym};

use crate::ctxt::AnalysisCtxt;

declare_tool_lint! {
    //// The `stack_frame_too_large` lint detects large stack frames that may potentially
    /// lead to stack overflow.
    pub klint::STACK_FRAME_TOO_LARGE,
    Allow,
    "frame size is too large"
}

#[derive(Diagnostic)]
#[diag(klint_stack_frame_limit_missing)]
#[help(klint_stack_frame_limit_help)]
struct StackFrameLimitMissing {
    #[primary_span]
    pub span: Span,
    pub default: u32,
}

#[derive(Diagnostic)]
#[diag(klint_stack_frame_limit_invalid)]
#[help(klint_stack_frame_limit_help)]
struct StackFrameLimitInvalid {
    #[primary_span]
    pub span: Span,
    pub setting: Symbol,
}

pub fn stack_size_check<'tcx, 'obj>(cx: &AnalysisCtxt<'tcx>, _file: &File<'obj>) {
    let lint_cfg = cx.lint_level_at_node(STACK_FRAME_TOO_LARGE, CRATE_HIR_ID);
    // Given inlining and cross-crate monomorphization happening, it does not make
    // a lot of sense to define this lint on anywhere except codegen unit level. So
    // just take levels from the crate root.
    let _level = match lint_cfg.level {
        // Don't run any of the checks if the lint is allowed.
        // This is one of the more expensive checks.
        //
        // NOTE: `expect` is actually not supported as this check is too late.
        // But we need to match it so treat like `allow` anyway.
        rustc_lint::Level::Allow | rustc_lint::Level::Expect => return,
        rustc_lint::Level::Warn => Level::Warning,
        rustc_lint::Level::ForceWarn => Level::ForceWarning,
        rustc_lint::Level::Deny | rustc_lint::Level::Forbid => Level::Error,
    };

    // Obtain the stack size limit.
    // Ideally we support `#![klint::stack_frame_size_limit = 4096]`, but this is not yet stable
    // (custom_inner_attributes).
    // Instead, we find via `CONFIG_FRAME_WARN` cfg.
    let frame_limit_sym = cx
        .sess
        .psess
        .config
        .iter()
        .copied()
        .find(|&(k, v)| k == crate::symbol::CONFIG_FRAME_WARN && v.is_some())
        .map(|(_, v)| v.unwrap())
        .unwrap_or(sym::empty);
    let _frame_limit = if frame_limit_sym.is_empty() {
        cx.dcx().emit_warn(StackFrameLimitMissing {
            span: lint_cfg.src.span(),
            default: 2048,
        });
        2048
    } else if let Ok(v) = frame_limit_sym.as_str().parse() {
        v
    } else {
        cx.dcx().emit_err(StackFrameLimitInvalid {
            span: lint_cfg.src.span(),
            setting: frame_limit_sym,
        });
        return;
    };
}
