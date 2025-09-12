use std::path::Path;

use rustc_middle::ty::TyCtxt;
use rustc_span::{BytePos, DUMMY_SP, FileName, Span};

pub fn recover_span_from_line_no<'tcx>(
    tcx: TyCtxt<'tcx>,
    path: &Path,
    line: u32,
    column: u32,
) -> Option<Span> {
    // Find the file in session's source map.
    let source_map = tcx.sess.source_map();
    let mut found_file = None;
    for file in source_map.files().iter() {
        if let FileName::Real(real) = &file.name {
            if real.local_path_if_available() == path {
                found_file = Some(file.clone());
            }
        }
    }

    let Some(found_file) = found_file else {
        return None;
    };

    let range = found_file.line_bounds((line as usize).saturating_sub(1));
    Some(Span::with_root_ctxt(
        BytePos(range.start.0 + column.saturating_sub(1)),
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
