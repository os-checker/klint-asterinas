use rustc_middle::ty::{Instance, TyCtxt};
use rustc_middle::{mir, ty};
use rustc_span::{BytePos, DUMMY_SP, FileName, Span};

pub fn recover_span_from_line_no<'tcx>(
    tcx: TyCtxt<'tcx>,
    location: &super::dwarf::Location,
) -> Option<Span> {
    // Find the file in session's source map.
    let source_map = tcx.sess.source_map();
    let mut found_file = None;
    for file in source_map.files().iter() {
        if let FileName::Real(real) = &file.name {
            if real.local_path_if_available() == location.file {
                found_file = Some(file.clone());
            }
        }
    }

    let Some(found_file) = found_file else {
        return None;
    };

    let range = found_file.line_bounds((location.line as usize).saturating_sub(1));
    Some(Span::with_root_ctxt(
        BytePos(range.start.0 + location.column.saturating_sub(1) as u32),
        // We only have a single column info. A good approximation is to extend to end of line (which is typically the case for function calls).
        BytePos(range.end.0 - 1),
    ))
}

// Compare a recovered span from a compiler-produced span, and determine if they're likely the same source.
pub fn recover_span<'tcx>(recover_span: Span, span: Span) -> bool {
    // Recovered span is produced through debug info. This will undergo the debuginfo collapse process.
    // Before comparing, undergo the same process for `span`.

    let collapsed = rustc_span::hygiene::walk_chain_collapsed(span, DUMMY_SP);

    let range = collapsed.lo()..collapsed.hi();
    range.contains(&recover_span.lo())
}

pub fn recover_fn_call_span<'tcx>(
    tcx: TyCtxt<'tcx>,
    caller: Instance<'tcx>,
    callee: &str,
    location: Option<&super::dwarf::Location>,
) -> Option<(Instance<'tcx>, Span)> {
    let mir = tcx.instance_mir(caller.def);

    let mut callee_instance = None;
    let mut spans = Vec::new();

    for block in mir.basic_blocks.iter() {
        let terminator = block.terminator();

        // Skip over inlined body. We'll check them from scopes directly.
        if mir.source_scopes[terminator.source_info.scope]
            .inlined
            .is_some()
        {
            continue;
        }

        let instance = match terminator.kind {
            mir::TerminatorKind::Call { ref func, .. }
            | mir::TerminatorKind::TailCall { ref func, .. } => {
                let callee_ty = func.ty(mir, tcx);
                let callee_ty = caller.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    ty::TypingEnv::fully_monomorphized(),
                    ty::EarlyBinder::bind(callee_ty),
                );

                let ty::FnDef(def_id, args) = *callee_ty.kind() else {
                    continue;
                };
                ty::Instance::expect_resolve(
                    tcx,
                    ty::TypingEnv::fully_monomorphized(),
                    def_id,
                    args,
                    terminator.source_info.span,
                )
            }
            mir::TerminatorKind::Drop { ref place, .. } => {
                let ty = place.ty(mir, tcx).ty;
                let ty = caller.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    ty::TypingEnv::fully_monomorphized(),
                    ty::EarlyBinder::bind(ty),
                );
                Instance::resolve_drop_in_place(tcx, ty)
            }

            _ => continue,
        };

        if tcx.symbol_name(instance).name != callee {
            continue;
        }

        callee_instance = Some(instance);
        spans.push(terminator.source_info.span);
    }

    // In addition to direct function calls, we should also inspect inlined functions.
    for scope in mir.source_scopes.iter() {
        if scope.inlined_parent_scope.is_none()
            && let Some((instance, span)) = scope.inlined
        {
            if tcx.symbol_name(instance).name != callee {
                continue;
            }

            callee_instance = Some(instance);
            spans.push(span);
        }
    }

    let Some(callee_instance) = callee_instance else {
        tracing::error!("{} does not contain call to {}", caller, callee);
        return None;
    };

    // If there's only a single span, then it has to be the correct span.
    if spans.len() == 1 {
        return Some((callee_instance, spans[0]));
    }

    // Otherwise, we need to use the DWARF location information to find the best related span.
    let Some(loc) = &location else {
        tracing::warn!(
            "no way to distinguish {}'s use of {}",
            caller,
            callee_instance
        );
        return Some((callee_instance, spans[0]));
    };

    let Some(recovered_span) = recover_span_from_line_no(tcx, loc) else {
        tracing::warn!(
            "no way to distinguish {}'s use of {}",
            caller,
            callee_instance
        );
        return Some((callee_instance, spans[0]));
    };

    // Now we have a recovered span. Use this span to match spans that we have.
    for span in spans {
        if recover_span(recovered_span, span) {
            return Some((callee_instance, span));
        }
    }

    // No perfect match, just use the recovered span that we have.
    Some((callee_instance, recovered_span))
}
