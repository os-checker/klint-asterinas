#![feature(rustc_private)]
#![allow(dead_code)]

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_log;
extern crate rustc_middle;
extern crate rustc_serialize;
extern crate rustc_session;
extern crate rustc_span;

#[macro_use]
mod ctxt;

#[macro_use]
extern crate tracing;

mod callbacks;
mod diagnostic;
mod hir_lints;
mod serde;

fn main() {
    let handler =
        rustc_session::EarlyDiagCtxt::new(rustc_session::config::ErrorOutputType::default());
    rustc_driver::init_logger(&handler, rustc_log::LoggerConfig::from_env("KLINT_LOG"));

    let mut args: Vec<_> = std::env::args().collect();
    if args
        .get(1)
        .and_then(|arg| std::path::Path::new(arg.as_str()).file_name()?.to_str())
        .is_some_and(|f| f == "rustc")
    {
        args.remove(1);
    }
    rustc_driver::run_compiler(&args, &mut crate::callbacks::Driver);
}
