use object::{File, Object, ObjectSection, ObjectSymbol, RelocationTarget};
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::ty::{Instance, TypingEnv};
use rustc_span::Span;

use crate::ctxt::AnalysisCtxt;
use crate::diagnostic::use_stack::{UseSite, UseSiteKind};

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced_without_symbol)]
struct BuildErrorReferencedWithoutSymbol;

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced_without_instance)]
struct BuildErrorReferencedWithoutInstance<'a> {
    pub symbol: &'a str,
}

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced_without_debug)]
struct BuildErrorReferencedWithoutDebug<'tcx> {
    #[primary_span]
    pub span: Span,
    pub kind: &'static str,
    pub instance: Instance<'tcx>,
    pub err: String,
}

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced)]
struct BuildErrorReferenced;

pub fn build_error_detection<'tcx, 'obj>(cx: &AnalysisCtxt<'tcx>, file: &File<'obj>) {
    let Some(build_error_symbol) = file.symbol_by_name("rust_build_error") else {
        // This object file contains no reference to `build_error`, all good!
        return;
    };

    // This object file defines this symbol; in which case we're codegenning for `build_error` crate.
    // Nothing to do.
    if !build_error_symbol.is_undefined() {
        return;
    }

    let relo_target_needle = RelocationTarget::Symbol(build_error_symbol.index());

    // Now this file contains reference to `build_error`, this is not expected.
    // We need to figure out why it is being generated.

    // Collect all mono items, which we will use to find out which symbol is problematic.
    let mono_items = crate::monomorphize_collector::collect_crate_mono_items(
        cx.tcx,
        crate::monomorphize_collector::MonoItemCollectionStrategy::Lazy,
    )
    .0;

    for section in file.sections() {
        for (offset, relocation) in section.relocations() {
            if relocation.target() == relo_target_needle {
                // Found a relocation that points to `build_error`. Emit an error.
                let Some((symbol, _)) =
                    super::find_symbol_from_section_offset(file, &section, offset)
                else {
                    cx.dcx().emit_err(BuildErrorReferencedWithoutSymbol);
                    continue;
                };

                let Some(mono) = mono_items
                    .iter()
                    .find(|item| item.symbol_name(cx.tcx).name == symbol)
                else {
                    cx.dcx()
                        .emit_err(BuildErrorReferencedWithoutInstance { symbol });
                    continue;
                };

                let loader = super::dwarf::DwarfLoader::new(file)
                    .expect("DWARF loader creation should not fail");

                let mut diag = cx.dcx().create_err(BuildErrorReferenced);
                let mut frame = match mono {
                    MonoItem::Fn(instance) => *instance,
                    MonoItem::Static(def_id) => Instance::mono(cx.tcx, *def_id),
                    MonoItem::GlobalAsm(_) => bug!(),
                };

                let mut recovered_call_stack = Vec::new();
                let result: Result<_, super::dwarf::Error> = try {
                    let call_stack = loader.inline_info(section.index(), offset)?;
                    if let Some(first) = call_stack.first() {
                        if first.caller != symbol {
                            Err(super::dwarf::Error::UnexpectedDwarf(
                                "root of call stack is unexpected",
                            ))?
                        }
                    }
                    for call in call_stack {
                        if let Some((callee, site)) = super::reconstruct::recover_fn_call_span(
                            cx.tcx,
                            frame,
                            &call.callee,
                            call.location.as_ref(),
                        ) {
                            recovered_call_stack.push(UseSite {
                                instance: TypingEnv::fully_monomorphized().as_query_input(frame),
                                kind: site,
                            });
                            frame = callee;
                        }
                    }
                };
                if let Err(err) = result {
                    diag.note(format!(
                        "attempt to reconstruct inline information from DWARF failed: {err}"
                    ));
                }

                let result: Result<_, super::dwarf::Error> = try {
                    let loc = loader.locate(section.index(), offset)?.ok_or(
                        super::dwarf::Error::UnexpectedDwarf("cannot find line number info"),
                    )?;

                    if let Some((_, site)) = super::reconstruct::recover_fn_call_span(
                        cx.tcx,
                        frame,
                        "rust_build_error",
                        Some(&loc),
                    ) {
                        recovered_call_stack.push(UseSite {
                            instance: TypingEnv::fully_monomorphized().as_query_input(frame),
                            kind: site,
                        });
                    } else {
                        let span = super::reconstruct::recover_span_from_line_no(cx.tcx, &loc)
                            .ok_or(super::dwarf::Error::Other(
                                "cannot find file in compiler session",
                            ))?;
                        recovered_call_stack.push(UseSite {
                            instance: TypingEnv::fully_monomorphized().as_query_input(frame),
                            kind: UseSiteKind::Other(
                                span,
                                "which is referenced by this function".to_string(),
                            ),
                        })
                    }
                };
                if let Err(err) = result {
                    diag.cancel();

                    // If even line number cannot be recovered, emit a different diagnostic.
                    cx.dcx().emit_err(match mono {
                        MonoItem::Fn(instance) => BuildErrorReferencedWithoutDebug {
                            span: cx.def_span(instance.def_id()),
                            kind: "fn",
                            instance: *instance,
                            err: err.to_string(),
                        },
                        MonoItem::Static(def_id) => BuildErrorReferencedWithoutDebug {
                            span: cx.def_span(def_id),
                            kind: "static",
                            instance: Instance::mono(cx.tcx, *def_id),
                            err: err.to_string(),
                        },
                        MonoItem::GlobalAsm(_) => {
                            // We're not going to be covered by symbols inside global asm.
                            bug!();
                        }
                    });
                    continue;
                }

                cx.note_use_stack(&mut diag, &recovered_call_stack);
                diag.span_note(
                    cx.def_span(mono.def_id()),
                    format!("reference contained in `{}`", mono),
                );
                diag.emit();
            }
        }
    }
}
