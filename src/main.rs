// Copyright Gary Guo.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![feature(rustc_private)]
#![feature(box_patterns)]
#![feature(if_let_guard)]
#![feature(never_type)]
// Used in monomorphize collector
#![feature(impl_trait_in_assoc_type)]
#![feature(once_cell_get_mut)]
#![warn(rustc::internal)]

#[macro_use]
extern crate rustc_macros;
#[macro_use]
extern crate rustc_middle;
#[macro_use]
extern crate tracing;

extern crate itertools;
extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_fluent_macro;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_log;
extern crate rustc_metadata;
extern crate rustc_mir_dataflow;
extern crate rustc_monomorphize;
extern crate rustc_serialize;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_trait_selection;

use rustc_driver::Callbacks;
use rustc_interface::interface::Config;
use rustc_middle::ty::TyCtxt;
use rustc_session::EarlyDiagCtxt;
use rustc_session::config::ErrorOutputType;
use std::sync::atomic::Ordering;

#[macro_use]
mod ctxt;

mod atomic_context;
mod attribute;
mod driver;
mod infallible_allocation;
mod lattice;
mod mir;
mod monomorphize_collector;
mod preempt_count;
mod serde;
mod symbol;
mod util;

rustc_session::declare_tool_lint! {
    pub klint::INCORRECT_ATTRIBUTE,
    Forbid,
    "Incorrect usage of klint attributes"
}

struct MyCallbacks;

impl Callbacks for MyCallbacks {
    fn config(&mut self, config: &mut Config) {
        config.locale_resources.push(crate::DEFAULT_LOCALE_RESOURCE);

        config.override_queries = Some(|_, provider| {
            // Calling `optimized_mir` will steal the result of query `mir_drops_elaborated_and_const_checked`,
            // so hijack `optimized_mir` to run `analysis_mir` first.
            hook_query!(provider.optimized_mir => |tcx, def_id, original| {
                // Skip `analysis_mir` call if this is a constructor, since it will be delegated back to
                // `optimized_mir` for building ADT constructor shim.
                if !tcx.is_constructor(def_id.to_def_id()) {
                    crate::mir::local_analysis_mir(tcx, def_id);
                }

                original(tcx, def_id)
            });
        });
        config.register_lints = Some(Box::new(move |_, lint_store| {
            lint_store.register_lints(&[&INCORRECT_ATTRIBUTE]);
            lint_store.register_lints(&[&infallible_allocation::INFALLIBLE_ALLOCATION]);
            lint_store.register_lints(&[&atomic_context::ATOMIC_CONTEXT]);
            // lint_store
            //     .register_late_pass(|_| Box::new(infallible_allocation::InfallibleAllocation));
            lint_store.register_late_pass(|tcx| Box::new(atomic_context::AtomicContext::new(tcx)));
        }));
    }
}

impl driver::CallbacksExt for MyCallbacks {
    type ExtCtxt<'tcx> = TyCtxt<'tcx>;

    fn ext_cx<'tcx>(&mut self, tcx: TyCtxt<'tcx>) -> Self::ExtCtxt<'tcx> {
        tcx
    }
}

fn main() {
    let handler = EarlyDiagCtxt::new(ErrorOutputType::default());
    rustc_driver::init_logger(&handler, rustc_log::LoggerConfig::from_env("KLINT_LOG"));
    let args: Vec<_> = std::env::args().collect();

    driver::run_compiler(&args, MyCallbacks);
}

rustc_fluent_macro::fluent_messages! { "./messages.ftl" }
