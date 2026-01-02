//! Out-of-band attributes attached without source code changes.

use rustc_hir::def_id::{DefId, LOCAL_CRATE};
use rustc_hir::diagnostic_items::DiagnosticItems;
use rustc_middle::middle::exported_symbols::ExportedSymbol;
use rustc_middle::ty::TyCtxt;

pub fn infer_missing_items<'tcx>(tcx: TyCtxt<'tcx>, items: &mut DiagnosticItems) {
    if !items.name_to_id.contains_key(&crate::symbol::build_error) {
        if let Some(def_id) = infer_build_error_diagnostic_item(tcx) {
            super::collect_item(tcx, items, crate::symbol::build_error, def_id);
        }
    }
}

pub fn infer_build_error_diagnostic_item<'tcx>(tcx: TyCtxt<'tcx>) -> Option<DefId> {
    for exported in tcx.exported_non_generic_symbols(LOCAL_CRATE) {
        if let ExportedSymbol::NonGeneric(def_id) = exported.0
            && exported.0.symbol_name_for_local_instance(tcx).name == "rust_build_error"
        {
            return Some(def_id);
        }
    }

    None
}
