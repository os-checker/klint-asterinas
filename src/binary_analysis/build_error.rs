use object::{File, Object, ObjectSection, ObjectSymbol, RelocationTarget};
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::Span;

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced_without_symbol)]
struct BuildErrorReferencedWithoutSymbol;

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced_without_instance)]
struct BuildErrorReferencedWithoutInstance<'a> {
    pub symbol: &'a str,
}

#[derive(Diagnostic)]
#[diag(klint_build_error_referenced)]
struct BuildErrorReferenced<'tcx> {
    #[primary_span]
    pub span: Span,
    pub kind: &'static str,
    pub instance: Instance<'tcx>,
}

pub fn build_error_detection<'tcx, 'obj>(tcx: TyCtxt<'tcx>, file: &File<'obj>) {
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
        tcx,
        crate::monomorphize_collector::MonoItemCollectionStrategy::Lazy,
    )
    .0;

    for section in file.sections() {
        for (offset, relocation) in section.relocations() {
            if relocation.target() == relo_target_needle {
                // Found a relocation that points to `build_error`. Emit and error.
                let Some((symbol, _)) =
                    super::find_symbol_from_section_offset(file, &section, offset)
                else {
                    tcx.dcx().emit_err(BuildErrorReferencedWithoutSymbol);
                    continue;
                };

                let Some(mono) = mono_items
                    .iter()
                    .find(|item| item.symbol_name(tcx).name == symbol)
                else {
                    tcx.dcx()
                        .emit_err(BuildErrorReferencedWithoutInstance { symbol });
                    continue;
                };

                let mut diag = tcx.dcx().create_err(match mono {
                    MonoItem::Fn(instance) => BuildErrorReferenced {
                        span: tcx.def_span(instance.def_id()),
                        kind: "fn",
                        instance: *instance,
                    },
                    MonoItem::Static(def_id) => BuildErrorReferenced {
                        span: tcx.def_span(def_id),
                        kind: "static",
                        instance: Instance::mono(tcx, *def_id),
                    },
                    MonoItem::GlobalAsm(_) => {
                        // We're not going to be covered by symbols inside global asm.
                        bug!();
                    }
                });

                let loader = super::dwarf::DwarfLoader::new(file)
                    .expect("DWARF loader creation should not fail");
                let mut frame = match mono {
                    MonoItem::Fn(instance) => Some(*instance),
                    _ => None,
                };

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
                        if let Some(caller) = frame.take() {
                            if let Some((callee, span)) = super::reconstruct::recover_fn_call_span(
                                tcx,
                                caller,
                                &call.callee,
                                call.location.as_ref(),
                            ) {
                                frame = Some(callee);
                                diag.span_note(span, format!("which calls `{callee}`"));
                            }
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

                    if let Some(frame) = frame
                        && let Some((_, span)) = super::reconstruct::recover_fn_call_span(
                            tcx,
                            frame,
                            "rust_build_error",
                            Some(&loc),
                        )
                    {
                        diag.span_note(
                            span,
                            "which contains a `build_error` call that is not optimized out",
                        );
                    } else {
                        let span = super::reconstruct::recover_span_from_line_no(tcx, &loc).ok_or(
                            super::dwarf::Error::Other("cannot find file in compiler session"),
                        )?;
                        diag.span_note(
                            span,
                            "which contains a `build_error` reference that is not optimized out",
                        );
                    }
                };
                if let Err(err) = result {
                    diag.note(format!(
                        "attempt to reconstruct line information from DWARF failed: {err}"
                    ));
                }

                diag.emit();
            }
        }
    }
}
